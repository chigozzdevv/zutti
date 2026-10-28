import assert from "node:assert/strict";
import { test } from "node:test";
import { create_handler } from "../src/handler.ts";

const origin = "http://127.0.0.1:3002";
const account = { network: "testnet", zcash_address: "utest1-test" };
const review = {
  swap_id: "01",
  txid: "02",
  zec: "0.01",
  usdc: "1",
  fee_zatoshi: "10000",
  recipient: "utest1-maker",
  expiry_height: 4_560_000,
};

function fixture({ approve = true, verify = true } = {}) {
  const calls = [];
  const authorization = {
    review: () => JSON.stringify(review),
    sign: (_wallet, now) => {
      calls.push(["sign", now]);
      return new Uint8Array([1, 2, 3]);
    },
    free: () => calls.push(["free-authorization"]),
  };
  const wallet = {
    account: () => JSON.stringify(account),
    authorize: (_payment, now) => {
      calls.push(["verify", now]);
      if (!verify) throw new Error("invalid reservation");
      return authorization;
    },
    prepare_deposit: (amount, owner) => {
      calls.push(["prepare-deposit", amount, owner]);
      return JSON.stringify({ amount, owner, witness: "aabb" });
    },
    verify_deposit: (witness, receipt) => {
      calls.push(["verify-deposit", witness, receipt]);
      if (!verify) throw new Error("invalid deposit proof");
      return JSON.stringify({ chain_id: 11155111, data: "0x1d0a9b2e" });
    },
    free: () => calls.push(["free-wallet"]),
  };
  let retained_entropy;
  let time = 100;
  const provider = {
    request: async (request) => {
      calls.push([request.method, request.params]);
      if (request.method === "snap_getEntropy") return `0x${"42".repeat(32)}`;
      if (request.method === "snap_dialog") {
        time = 110;
        return approve;
      }
      throw new Error("unexpected wallet API");
    },
  };
  const handler = create_handler(
    {
      wallet: async (entropy) => {
        retained_entropy = entropy;
        assert.equal(entropy[0], 0x42);
        return wallet;
      },
    },
    provider,
    () => time,
  );
  const invoke = (method, params, overrides = {}) =>
    handler({
      origin,
      originMetadata: null,
      request: { jsonrpc: "2.0", id: 1, method, ...(params === undefined ? {} : { params }) },
      ...overrides,
    });
  return { calls, invoke, entropy: () => retained_entropy };
}

test("unapproved origins and unknown methods cannot reach private entropy", async () => {
  const f = fixture();
  await assert.rejects(f.invoke("zutti_getAccount", undefined, { origin: "https://example.com" }));
  await assert.rejects(
    f.invoke("zutti_getAccount", undefined, { originMetadata: { name: "other" } }),
  );
  await assert.rejects(f.invoke("exportSeed"));
  await assert.rejects(f.invoke("zutti_getAccount", { seed: "42" }));
  await assert.rejects(f.invoke("zutti_signPayment", { payment: "{}", now: 100 }));
  assert.deepEqual(f.calls, []);
});

test("deposit preparation requires exact Testnet terms and consent before releasing its witness", async () => {
  const owner = `0x${"11".repeat(20)}`;
  const bad = fixture();
  for (const params of [
    { amount: "1", owner, seed: "aa" },
    { amount: "0", owner },
    { amount: "1.0000001", owner },
    { amount: "1", owner: "0x0" },
  ])
    await assert.rejects(bad.invoke("zutti_prepareDeposit", params));
  assert.deepEqual(bad.calls, []);
  const denied = fixture({ approve: false });
  await assert.rejects(denied.invoke("zutti_prepareDeposit", { amount: "1", owner }), /declined/);
  assert.equal(denied.calls.at(-1)[0], "free-wallet");
  const approved = fixture();
  assert.deepEqual(await approved.invoke("zutti_prepareDeposit", { amount: "1", owner }), {
    amount: "1",
    owner,
    witness: "aabb",
  });
  assert.ok(
    approved.calls.find(
      ([method, params]) =>
        method === "snap_dialog" && JSON.stringify(params).includes("remote prover"),
    ),
  );
  assert.ok(approved.entropy().every((byte) => byte === 0));
});

test("deposit proof validation cannot sign Ethereum transactions or disclose wallet keys", async () => {
  const bad = fixture({ verify: false });
  await assert.rejects(
    bad.invoke("zutti_verifyDeposit", { witness: "aabb", receipt: "ccdd" }),
    /invalid deposit proof/,
  );
  assert.deepEqual(
    bad.calls.map(([method]) => method),
    ["snap_getEntropy", "verify-deposit", "free-wallet"],
  );
  const approved = fixture();
  assert.deepEqual(
    await approved.invoke("zutti_verifyDeposit", { witness: "aabb", receipt: "ccdd" }),
    { chain_id: 11155111, data: "0x1d0a9b2e" },
  );
});

test("account sharing requires approval and wipes the temporary seed", async () => {
  const denied = fixture({ approve: false });
  await assert.rejects(denied.invoke("zutti_getAccount"), /declined/);
  assert.deepEqual(
    denied.calls.map(([method]) => method),
    ["snap_dialog"],
  );
  const approved = fixture();
  assert.deepEqual(await approved.invoke("zutti_getAccount"), account);
  assert.deepEqual(
    approved.calls.map(([method]) => method),
    ["snap_dialog", "snap_getEntropy", "free-wallet"],
  );
  assert.ok(approved.entropy().every((byte) => byte === 0));
});

test("verified origins remain usable when Flask omits optional origin metadata", async () => {
  const f = fixture();
  assert.deepEqual(
    await f.invoke("zutti_getAccount", undefined, { originMetadata: undefined }),
    account,
  );
});

test("invalid evidence fails before the signing approval dialog", async () => {
  const f = fixture({ verify: false });
  await assert.rejects(f.invoke("zutti_signPayment", { payment: "{}" }), /invalid reservation/);
  assert.deepEqual(
    f.calls.map(([method]) => method),
    ["snap_getEntropy", "verify", "free-wallet"],
  );
});

test("declining a verified payment frees it without signing", async () => {
  const f = fixture({ approve: false });
  await assert.rejects(f.invoke("zutti_signPayment", { payment: "{}" }), /declined/);
  assert.deepEqual(
    f.calls.map(([method]) => method),
    ["snap_getEntropy", "verify", "snap_dialog", "free-authorization", "free-wallet"],
  );
});

test("signing uses a fresh wallet clock after approval and returns no keys", async () => {
  const f = fixture();
  const result = await f.invoke("zutti_signPayment", { payment: "{}" });
  assert.deepEqual(result, { swap_id: "01", txid: "02", pczt: "010203" });
  assert.deepEqual(
    f.calls.filter(([method]) => ["verify", "sign"].includes(method)),
    [
      ["verify", 100],
      ["sign", 110],
    ],
  );
  assert.equal(f.calls.at(-1)[0], "free-wallet");
  assert.ok(f.entropy().every((byte) => byte === 0));
});

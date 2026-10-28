import assert from "node:assert/strict";
import { test } from "node:test";
import { BrowserFunding, usdc_units } from "../src/shared/funding.ts";

const owner = `0x${"12".repeat(20)}`;
const pool = `0x${"23".repeat(20)}`;
const token = `0x${"34".repeat(20)}`;
const domain = "45".repeat(32);
const id = "56".repeat(32);
const approval = `0x${"67".repeat(32)}`;
const transaction = `0x${"78".repeat(32)}`;
const block = `0x${"89".repeat(32)}`;
const tick = () => new Promise(setImmediate);

function fixture(t, options = {}) {
  const calls = [];
  const state = {
    account: owner,
    ready: false,
    balance: options.balance ?? 10_000_000n,
    allowance: 0n,
  };
  const profile = {
    name: "testnet",
    deployment_id: domain,
    ethereum: { chain_id: "11155111", usdc: token, pool },
  };
  const provider = {
    request: async ({ method, params }) => {
      calls.push({ method, params });
      if (method === "eth_accounts") return [state.account];
      if (method === "eth_chainId") return "0xaa36a7";
      if (method === "eth_getBalance") return "0x1000000";
      if (method === "eth_call")
        return `0x${(params[0].data.startsWith("0x70a08231") ? state.balance : state.allowance).toString(16)}`;
      if (method === "eth_sendTransaction") {
        if (options.unknown) throw new Error("connection dropped after submission");
        if (params[0].to === token) {
          state.allowance = 1_000_000n;
          return approval;
        }
        return transaction;
      }
      if (method === "eth_getTransactionReceipt")
        return {
          transactionHash: params[0],
          status: "0x1",
          blockNumber: "0x4",
          blockHash: block,
          from: owner,
          to: params[0] === approval ? token : pool,
        };
      if (method === "eth_getBlockByNumber") return { number: "0x5", hash: block };
      throw new Error(`unexpected ${method}`);
    },
  };
  const ethereum = { snapshot: () => ({ account: state.account }), extension: () => provider };
  let verified = 0;
  const zcash = {
    snapshot: () => ({ account: { domain, usdc_receiver: { owner: "02" + "9a".repeat(32) } } }),
    prepare_deposit: async (amount, address) => {
      assert.equal(address, owner);
      if (options.switch_account) state.account = `0x${"ff".repeat(20)}`;
      return { amount, units: usdc_units(amount).toString(), pool, token, witness: "aabb" };
    },
    verify_deposit: async () => {
      verified++;
      if (options.bad_proof) throw new Error("deposit proof verification failed");
      return { from: owner, to: pool, token, units: "1000000", value: "0x0", data: "0x1d0a9b2e00" };
    },
  };
  const store = new Map();
  const storage = {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
  };
  const result = () => ({
    id,
    witness: "aabb",
    state: state.ready ? "proof_ready" : "awaiting_proof",
    receipt: state.ready ? "ccdd" : null,
  });
  const funding = new BrowserFunding(ethereum, zcash, profile, storage, {
    prepare: async () => result(),
    get: async () => result(),
  });
  funding.start();
  t.after(() => funding.stop());
  const ready = async () => {
    funding.stop();
    state.ready = true;
    funding.start();
    for (let i = 0; i < 10; i++) await tick();
    assert.equal(funding.snapshot().stage, "proof_ready");
  };
  return { funding, calls, state, ready, verified: () => verified, storage, store };
}

test("unfunded wallets cannot prepare or submit deposit proofs or transactions", async (t) => {
  const f = fixture(t, { balance: 0n });
  await f.funding.prepare("1");
  assert.match(f.funding.snapshot().error, /has 0 Sepolia USDC/);
  assert.equal(f.funding.snapshot().stage, "idle");
  assert.equal(f.store.size, 0);
  assert.ok(f.calls.every((call) => call.method !== "eth_sendTransaction"));
});

test("waiting for proof makes no financial requests; genuine verification precedes exact approval", async (t) => {
  const f = fixture(t);
  await f.funding.prepare("1");
  await f.funding.complete();
  assert.equal(f.funding.snapshot().stage, "awaiting_proof");
  assert.equal(f.verified(), 0);
  assert.ok(f.calls.every((call) => call.method !== "eth_sendTransaction"));
  await f.ready();
  await f.funding.complete();
  assert.equal(f.verified(), 1);
  const sends = f.calls.filter((call) => call.method === "eth_sendTransaction");
  assert.equal(sends.length, 2);
  assert.equal(sends[0].params[0].to, token);
  assert.equal(BigInt(`0x${sends[0].params[0].data.slice(-64)}`), 1_000_000n);
  assert.equal(sends[1].params[0].to, pool);
  assert.equal(f.funding.snapshot().stage, "confirmed");
  await f.funding.complete();
  assert.equal(f.calls.filter((call) => call.method === "eth_sendTransaction").length, 2);
});

test("invalid proof and account changes cannot trigger token approval", async (t) => {
  const bad = fixture(t, { bad_proof: true });
  await bad.funding.prepare("1");
  await bad.ready();
  await bad.funding.complete();
  assert.match(bad.funding.snapshot().error, /verification failed/);
  assert.ok(bad.calls.every((call) => call.method !== "eth_sendTransaction"));
  const changed = fixture(t, { switch_account: true });
  await changed.funding.prepare("1");
  assert.match(changed.funding.snapshot().error, /wallet changed/);
  assert.equal(changed.store.size, 0);
});

test("ambiguous transaction submission is retained and never automatically retried", async (t) => {
  const f = fixture(t, { unknown: true });
  await f.funding.prepare("1");
  await f.ready();
  await f.funding.complete();
  assert.equal(f.funding.snapshot().stage, "inspect");
  assert.match(f.funding.snapshot().error, /outcome is unknown/);
  await f.funding.complete();
  await f.funding.prepare("1");
  assert.equal(f.calls.filter((call) => call.method === "eth_sendTransaction").length, 1);
  assert.equal(JSON.parse([...f.store.values()][0]).submitting, true);
});

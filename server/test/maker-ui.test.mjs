import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { MakerService } from "../dist/features/maker/maker.service.js";
import { create_app_server } from "../dist/ui-app.js";
import { load_app_information } from "../dist/features/app/app.service.js";
import { RfqClient } from "../../packages/runtime/dist/index.js";
import { setup, options, profile } from "../../tests/support/rfq.mjs";

const settings = { ...options, pairs: ["ZEC/USDC"] };
const tick = () => new Promise((resolve) => setTimeout(resolve, 10));
const connection = (fixture, extra = {}) => ({
  wallet: { zutti_maker: () => ({ profile, maker: fixture.maker_runtime }) },
  ...extra,
});
async function idle(service) {
  for (let n = 0; n < 200; n++) {
    const value = await service.snapshot();
    if (!value.pending_action) return value;
    await tick();
  }
  assert.fail("Maker action did not finish");
}
function inventory(fixture, extra = {}) {
  return {
    maker_id: fixture.maker_identity.id,
    deployment_id: profile.deployment_id,
    available_usdc: "12.5",
    reserved_usdc: "1",
    available_zec: "0",
    checked_at: Date.now(),
    ...extra,
  };
}

test("maker gateway ignores placeholder balances and admits only fresh owner-bound inventory", async () => {
  const fixture = await setup();
  let balances = inventory(fixture);
  let trades = {
    maker_id: fixture.maker_identity.id,
    deployment_id: profile.deployment_id,
    checked_at: Date.now(),
    complete: true,
    items: [
      {
        id: "ab".repeat(32),
        input: { asset: "ZEC", amount: "0.01" },
        output: { asset: "USDC", amount: "1" },
        state: "reserved",
        updated_at: Date.now(),
      },
    ],
  };
  const service = new MakerService(
    connection(fixture, { inventory: () => balances, trades: () => trades }),
    profile,
  );
  const unavailable = new MakerService(connection(fixture), profile);
  try {
    assert.equal((await unavailable.snapshot()).inventory, null);
    assert.equal((await unavailable.snapshot()).active_trades, null);
    assert.equal((await service.snapshot()).inventory.available_usdc, "12.5");
    assert.equal((await service.snapshot()).active_trades, 1);
    trades.complete = false;
    assert.equal((await service.snapshot()).active_trades, null);
    balances = inventory(fixture, { deployment_id: "ff".repeat(32) });
    assert.equal((await service.snapshot()).inventory, null);
    balances = inventory(fixture, { maker_id: "ff".repeat(32) });
    assert.equal((await service.snapshot()).inventory, null);
    balances = inventory(fixture, { checked_at: Date.now() - 301_000 });
    assert.equal((await service.snapshot()).inventory, null);
    const broken = new MakerService(
      connection(fixture, {
        inventory: () => {
          throw new Error("Private wallet failure");
        },
      }),
      profile,
    );
    assert.equal((await broken.snapshot()).inventory, null);
    assert.throws(
      () => new MakerService(connection(fixture), { ...profile, deployment_id: "ff".repeat(32) }),
      { code: "deployment_unavailable" },
    );
  } finally {
    await service.close();
    await fixture.close();
  }
});

test("maker controls serialize repeated starts and use the real SDK quoting lifecycle", async () => {
  const fixture = await setup();
  let calls = 0;
  let release;
  const ready = new Promise((resolve) => {
    release = resolve;
  });
  fixture.maker_config.preflight = async () => {
    calls++;
    await ready;
  };
  const service = new MakerService(connection(fixture), profile);
  try {
    await Promise.all([
      service.control({ action: "start", settings }),
      service.control({ action: "start", settings }),
    ]);
    await tick();
    assert.equal(calls, 1);
    await assert.rejects(service.control({ action: "stop" }), { code: "service_unavailable" });
    release();
    assert.equal((await idle(service)).state, "running");
    await service.control({ action: "start", settings });
    await idle(service);
    assert.equal(calls, 1);
    await service.control({ action: "pause" });
    assert.equal((await idle(service)).state, "paused");
    await service.control({ action: "resume" });
    assert.equal((await idle(service)).state, "running");
    await assert.rejects(service.control({ action: "start", settings: options }), {
      code: "invalid_input",
    });
    await service.close();
    assert.equal((await service.snapshot()).state, "stopped");
    await assert.rejects(service.control({ action: "resume" }), { code: "wallet_unavailable" });
  } finally {
    release();
    await fixture.close();
  }
});

test("maker funding deduplicates requests and never confirms a mismatched receipt", async () => {
  const fixture = await setup();
  let calls = 0;
  let finish;
  const result = new Promise((resolve) => {
    finish = resolve;
  });
  const service = new MakerService(
    connection(fixture, {
      deposit: () => {
        calls++;
        return result;
      },
    }),
    profile,
  );
  const input = { id: "ac".repeat(32), asset: "USDC", amount: "10" };
  try {
    await Promise.all([service.deposit(input), service.deposit(input)]);
    assert.equal(calls, 1);
    await assert.rejects(service.deposit({ ...input, amount: "20" }), { code: "invalid_input" });
    await assert.rejects(service.deposit({ ...input, id: "ad".repeat(32) }), {
      code: "service_unavailable",
    });
    finish({
      ...input,
      amount: "20",
      evidence: {
        chain: "ethereum",
        block_hash: "aa".repeat(32),
        height: "1",
        transaction_id: "bb".repeat(32),
        policy_id: profile.policy.source_policy_id,
      },
      confirmed_at: Date.now(),
    });
    await tick();
    const value = await service.snapshot();
    assert.equal(value.deposit.state, "paused");
    assert.equal(value.deposit.receipt, null);
    await service.deposit(input);
    assert.equal(calls, 1);
  } finally {
    await service.close();
    await fixture.close();
  }
});

test("maker API requires an owner session and rejects foreign control requests", async () => {
  const fixture = await setup();
  await writeFile(join(fixture.root, "index.html"), "<main>Zutti test application</main>");
  const information = await load_app_information(new URL("../..", import.meta.url).pathname);
  const port = 13973;
  const origin = `http://127.0.0.1:${port}`;
  const server = create_app_server({
    information: { ...information, profile, wallet_available: true },
    origins: [origin],
    assets: fixture.root,
    connect_wallet: async () => ({
      label: "Maker transport test",
      wallet: { zutti: () => ({ profile, rfq: new RfqClient(fixture.client_config) }) },
      maker: connection(fixture),
    }),
  });
  server.listen(port, "127.0.0.1");
  await once(server, "listening");
  const headers = { "content-type": "application/json", "x-zutti-request": "1", origin };
  try {
    assert.equal((await fetch(`${origin}/maker`)).status, 200);
    assert.equal((await fetch(`${origin}/api/maker`)).status, 409);
    const connect = await fetch(`${origin}/api/wallet/connect`, {
      method: "POST",
      headers,
      body: "{}",
    });
    assert.equal(connect.status, 200);
    const cookie = connect.headers.get("set-cookie").split(";")[0];
    const snapshot = await fetch(`${origin}/api/maker`, { headers: { cookie } });
    assert.equal((await snapshot.json()).active_trades, null);
    const forbidden = await fetch(`${origin}/api/maker/control`, {
      method: "POST",
      headers: { ...headers, cookie, origin: "https://foreign.example" },
      body: JSON.stringify({ action: "start", settings }),
    });
    assert.equal(forbidden.status, 403);
    const unavailable = await fetch(`${origin}/api/maker/deposits`, {
      method: "POST",
      headers: { ...headers, cookie },
      body: JSON.stringify({ id: "cc".repeat(32), asset: "USDC", amount: "1" }),
    });
    assert.equal(unavailable.status, 409);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    await fixture.close();
  }
});

test("maker funding accepts a matching receipt and retains synchronous adapter failures", async () => {
  const fixture = await setup();
  const input = { id: "de".repeat(32), asset: "USDC", amount: "10" };
  const receipt = {
    ...input,
    evidence: {
      chain: "ethereum",
      block_hash: "aa".repeat(32),
      height: "1",
      transaction_id: "bb".repeat(32),
      policy_id: profile.policy.source_policy_id,
    },
    confirmed_at: Date.now(),
  };
  let calls = 0;
  const service = new MakerService(
    connection(fixture, {
      deposit: () => {
        calls++;
        return receipt;
      },
    }),
    profile,
  );
  const broken = new MakerService(
    connection(fixture, {
      deposit: () => {
        throw new Error("Native adapter unavailable");
      },
    }),
    profile,
  );
  try {
    await service.deposit(input);
    await tick();
    assert.deepEqual((await service.snapshot()).deposit.receipt, receipt);
    assert.equal((await service.deposit(input)).deposit.state, "confirmed");
    assert.equal(calls, 1);
    await broken.deposit(input);
    await tick();
    assert.equal((await broken.snapshot()).deposit.state, "paused");
    assert.equal((await broken.snapshot()).deposit.receipt, null);
  } finally {
    await service.close();
    await broken.close();
    await fixture.close();
  }
});

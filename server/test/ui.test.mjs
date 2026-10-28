import test from "node:test";
import assert from "node:assert/strict";
import { once } from "node:events";
import { request as http_request } from "node:http";
import { create_app_server } from "../dist/ui-app.js";
import { load_app_information } from "../dist/features/app/app.service.js";
import { WalletService } from "../dist/features/wallet/wallet.service.js";
import { setup, options, profile } from "../../tests/support/rfq.mjs";
import { RfqClient } from "../../packages/runtime/dist/index.js";

const information = await load_app_information(new URL("../..", import.meta.url).pathname);

test("UI reads actual completed evidence without exposing private witnesses or starting a wallet", async () => {
  assert.equal(information.profile.ethereum.chain_id, "11155111");
  assert.equal(information.wallet_available, false);
  assert.equal(information.activity.length, 1);
  assert.equal(
    information.activity[0].withdrawal_transaction,
    "0x9e274d999a848138d4ff045e3dec7803ec9af425f974ae5400eb5064c1f8475e",
  );
  assert.equal(information.activity[0].output.amount, "1");
  assert.equal(information.witnesses, "public-testnet");
  assert.equal(information.refunds, false);
  const encoded = JSON.stringify(information);
  for (const secret of [
    "witness.bin",
    "private_key",
    "nullifier_secret",
    "viewing_key",
    "owner.json",
  ])
    assert.equal(encoded.includes(secret), false);
});

test("app gateway rejects foreign origins and unknown hosts and requires an owner session", async () => {
  const port = 13972;
  const origin = `http://127.0.0.1:${port}`;
  const server = create_app_server({ information, origins: [origin] });
  server.listen(port, "127.0.0.1");
  await once(server, "listening");
  try {
    const body = JSON.stringify({});
    const headers = { "content-type": "application/json", "x-zutti-request": "1" };
    const foreign = await fetch(`${origin}/api/wallet/connect`, {
      method: "POST",
      headers: { ...headers, origin: "https://untrusted.example" },
      body,
    });
    assert.equal(foreign.status, 403);
    const missing_header = await fetch(`${origin}/api/wallet/connect`, {
      method: "POST",
      headers: { "content-type": "application/json", origin },
      body,
    });
    assert.equal(missing_header.status, 403);
    const rebinding_status = await new Promise((resolve, reject) => {
      const call = http_request(
        `${origin}/api/app`,
        { headers: { host: "untrusted.example" } },
        (response) => {
          response.resume();
          resolve(response.statusCode);
        },
      );
      call.on("error", reject);
      call.end();
    });
    assert.equal(rebinding_status, 403);
    const unavailable = await fetch(`${origin}/api/wallet/connect`, {
      method: "POST",
      headers: { ...headers, origin },
      body,
    });
    assert.equal(unavailable.status, 409);
    assert.equal((await unavailable.json()).error, "wallet_unavailable");
    const unauthenticated = await fetch(`${origin}/api/operations`);
    assert.equal(unauthenticated.status, 409);
    const app = await fetch(`${origin}/api/app`);
    assert.equal(app.headers.get("cache-control"), "no-store");
    assert.deepEqual(await app.json(), information);
    assert.equal((await fetch(`${origin}/.env`)).status, 404);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
});

test("wallet bridge pins the SDK deployment before exposing any quote capability", async () => {
  const service = new WalletService(information, async () => ({
    label: "Wrong deployment",
    wallet: { zutti: () => ({ profile }) },
  }));
  await assert.rejects(service.connect(), { code: "deployment_unavailable" });
  await service.close();
});

test("gateway exchanges genuine encrypted SDK quotes, isolates sessions and deduplicates accept and withdrawal", async () => {
  const fixture = await setup();
  let accept_calls = 0;
  let withdraw_calls = 0;
  let wrong_amount = false;
  let finish;
  const completed = new Promise((resolve) => {
    finish = resolve;
  });
  const service = new WalletService(
    { ...information, profile, wallet_available: true },
    async () => ({
      label: "Transport test wallet",
      wallet: {
        zutti: () => ({
          profile,
          rfq: new RfqClient(fixture.client_config),
          execution: {
            accept: async (offer) => {
              accept_calls++;
              await completed;
              return {
                outcome: "settled",
                swap_id: "aa".repeat(32),
                received: wrong_amount ? { asset: "USDC", amount: "200" } : offer.output,
                source_payment: {
                  chain: "zcash",
                  block_hash: "bb".repeat(32),
                  height: "123",
                  transaction_id: "cc".repeat(32),
                  policy_id: profile.policy.source_policy_id,
                },
                pool_settlement: {
                  chain: "ethereum",
                  block_hash: `0x${"dd".repeat(32)}`,
                  height: "456",
                  transaction_id: `0x${"ee".repeat(32)}`,
                  policy_id: profile.policy.source_policy_id,
                },
                confirmed_at: Date.now(),
              };
            },
          },
          account: {
            withdraw: async (terms) => {
              withdraw_calls++;
              return {
                operation_id: "aa".repeat(32),
                kind: "withdrawal",
                amount: { asset: "USDC", amount: terms.amount },
                output_mode: "public_erc20",
                to: terms.to,
                evidence: {
                  chain: "ethereum",
                  block_hash: `0x${"dd".repeat(32)}`,
                  height: "457",
                  transaction_id: `0x${"ff".repeat(32)}`,
                  policy_id: profile.policy.source_policy_id,
                },
                confirmed_at: Date.now(),
              };
            },
          },
        }),
      },
    }),
  );
  try {
    await fixture.maker.start(options);
    const owner = (await service.connect()).token;
    const other = (await service.connect()).token;
    const quote = await service.quote(owner, { from: "ZEC", to: "USDC", amount: "5" });
    assert.equal(quote.offers.items[0].authentication, "verified");
    assert.equal(quote.offers.items[0].output.amount, "199");
    assert.equal(JSON.stringify(fixture.issued).includes('"amount"'), false);
    const terms = { offer_id: quote.offers.items[0].id };
    const first = service.accept(owner, quote.id, terms);
    const second = service.accept(owner, quote.id, terms);
    assert.equal(first.id, second.id);
    assert.throws(() => service.get(other, first.id), { code: "invalid_input" });
    await assert.rejects(service.disconnect(owner), { code: "service_unavailable" });
    await assert.rejects(service.quote(owner, { from: "ZEC", to: "USDC", amount: "5" }), {
      code: "service_unavailable",
    });
    assert.throws(() => service.accept(owner, quote.id, { offer_id: "77".repeat(32) }), {
      code: "invalid_input",
    });
    finish();
    for (let n = 0; n < 100 && service.get(owner, first.id).state === "pending"; n++)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(service.get(owner, first.id).state, "settled");
    assert.equal(accept_calls, 1);
    const withdrawal = { asset: "USDC", amount: "199", to: `0x${"12".repeat(20)}` };
    const one = service.withdraw(owner, first.id, withdrawal);
    const two = service.withdraw(owner, first.id, withdrawal);
    assert.equal(one.id, two.id);
    assert.throws(
      () => service.withdraw(owner, first.id, { ...withdrawal, to: `0x${"13".repeat(20)}` }),
      { code: "invalid_input" },
    );
    for (let n = 0; n < 100 && service.get(owner, one.id).state === "pending"; n++)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(service.get(owner, one.id).state, "confirmed");
    assert.equal(withdraw_calls, 1);
    assert.equal(service.list(owner).length, 2);
    await assert.rejects(service.quote(owner, { from: "USDC", to: "ZEC", amount: "5" }));
    wrong_amount = true;
    const next = await service.quote(owner, { from: "ZEC", to: "USDC", amount: "5" });
    const uncertain = service.accept(owner, next.id, { offer_id: next.offers.items[0].id });
    for (let n = 0; n < 100 && service.get(owner, uncertain.id).state === "pending"; n++)
      await new Promise((resolve) => setTimeout(resolve, 10));
    assert.equal(service.get(owner, uncertain.id).state, "paused");
    assert.equal(service.get(owner, uncertain.id).receipt, null);
    await assert.rejects(service.quote(owner, { from: "ZEC", to: "USDC", amount: "5" }), {
      code: "service_unavailable",
    });
    await assert.rejects(service.disconnect(owner), { code: "service_unavailable" });
    assert.equal(
      service.accept(owner, next.id, { offer_id: next.offers.items[0].id }).id,
      uncertain.id,
    );
    assert.equal(accept_calls, 2);
  } finally {
    finish();
    await service.close();
    await fixture.close();
  }
});

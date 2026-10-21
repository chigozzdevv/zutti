import test from "node:test";
import assert from "node:assert/strict";
import { setup, options, domain } from "../../tests/support/rfq.mjs";
import {
  RfqClient,
  RfqMaker,
  MessagingIdentity,
  announce,
  authorize_poll,
  seal_message,
  random_id,
} from "../../packages/runtime/dist/index.js";

test("both SDKs exchange real encrypted HTTP RFQs and native fees in both directions", async () => {
  const fixture = await setup();
  try {
    await fixture.maker.start(options);
    const client = fixture.make_client();
    for (const [from, to, amount, output] of [
      ["ZEC", "USDC", "5", "199"],
      ["USDC", "ZEC", "100", "2.4875"],
    ]) {
      const intent = client.intent({ from, to, amount });
      const offers = await intent.offers();
      assert.equal(offers.collection, "complete");
      assert.equal(offers.items.length, 1);
      assert.deepEqual(offers.items[0].output, { asset: to, amount: output });
      assert.equal(offers.items[0].authentication, "verified");
      assert.ok(Object.isFrozen(offers.items[0]));
      await assert.rejects(intent.accept({ offer_id: offers.items[0].id }), {
        code: "deployment_unavailable",
      });
    }
    const summary = await fixture.maker.summary();
    assert.equal(summary.active_swaps, 0);
    assert.equal(summary.live_offers, 2);
    assert.ok(!JSON.stringify(fixture.issued).includes('"amount"'));
    await client.close();
    assert.throws(() => client.intent({ from: "ZEC", to: "USDC", amount: "5" }), {
      code: "wallet_unavailable",
    });
  } finally {
    await fixture.close();
  }
});
test("retry and restart retain recipients, signed messages and the same quote", async () => {
  const fixture = await setup();
  let restarted;
  try {
    await fixture.maker.start(options);
    const runtime = new RfqClient(fixture.client_config);
    const id = random_id();
    const request = { from: "ZEC", to: "USDC", amount: "5" };
    const first = await runtime.offers(id, request);
    assert.equal(first.items.length, 1);
    const calls = fixture.price_calls();
    await fixture.maker.stop();
    restarted = new RfqMaker(fixture.maker_config);
    await restarted.start(options);
    assert.equal((await restarted.summary()).live_offers, 1);
    const messages = [];
    const relay = {
      ...fixture.client_config.relay,
      makers: async () => {
        throw new Error("saved intent must not rediscover makers");
      },
      send: async (message, signal) => {
        messages.push(message);
        await fixture.relay.send(message, signal);
      },
      poll: (...args) => fixture.relay.poll(...args),
    };
    const again = await new RfqClient({ ...fixture.client_config, relay }).offers(id, request);
    assert.deepEqual(again.items, first.items);
    assert.deepEqual(again.requested_makers, first.requested_makers);
    assert.equal(fixture.price_calls(), calls);
    assert.equal(messages.length, 0);
    const plan = await fixture.client_store.read(`intent-${id}`);
    await fixture.relay.send(plan.selected[0].message);
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.equal(fixture.price_calls(), calls);
    assert.equal(fixture.issued.at(-1).signature, fixture.issued[0].signature);
    await assert.rejects(
      new RfqClient(fixture.client_config).offers(id, { ...request, amount: "6" }),
      { code: "persistence_failure" },
    );
  } finally {
    await restarted?.stop();
    await fixture.close();
  }
});
test("relay protects mailbox access, nonce reuse and request-response binding", async () => {
  const fixture = await setup();
  try {
    await fixture.maker.start(options);
    const owner = await MessagingIdentity.create();
    const message = await seal_message(
      owner,
      {
        network: domain.network,
        deployment_id: domain.deployment_id,
        type: "request",
        request_id: random_id(),
        recipient_id: fixture.maker_identity.id,
        recipient_key: fixture.maker_identity.encryption_key,
        deadline: Date.now() + 10_000,
      },
      { private: "opaque payload" },
    );
    await fixture.relay.send(message);
    await fixture.relay.send(message);
    const poll = await authorize_poll(owner, domain, Date.now());
    await fixture.relay.poll(poll);
    await assert.rejects(fixture.relay.poll(poll));
    await assert.rejects(
      fixture.relay.poll({ ...poll, nonce: random_id(), recipient_id: fixture.maker_identity.id }),
    );
    const other = await MessagingIdentity.create();
    await assert.rejects(
      fixture.relay.send(
        await seal_message(
          other,
          {
            network: domain.network,
            deployment_id: domain.deployment_id,
            type: "offer",
            request_id: message.request_id,
            recipient_id: owner.id,
            recipient_key: owner.encryption_key,
            deadline: message.deadline,
          },
          { amount: "999" },
        ),
      ),
    );
  } finally {
    await fixture.close();
  }
});
test("limits produce signed declines and stale pricing never becomes an actionable offer", async () => {
  const fixture = await setup();
  let stale;
  try {
    await fixture.maker.start(options);
    const client = fixture.make_client();
    const limits = await client.intent({ from: "ZEC", to: "USDC", amount: "11" }).offers();
    assert.equal(limits.collection, "complete");
    assert.equal(limits.items.length, 0);
    await fixture.maker.stop();
    stale = new RfqMaker({
      ...fixture.maker_config,
      price: async () => ({
        rate: "40",
        observed_at: Date.now() - 60_000,
        valid_until: Date.now() + 10_000,
      }),
    });
    await stale.start(options);
    const result = await client.intent({ from: "ZEC", to: "USDC", amount: "5" }).offers();
    assert.equal(result.items.length, 0);
    assert.equal(result.collection, "timed_out");
  } finally {
    await stale?.stop();
    await fixture.close();
  }
});
test("an intent never expands its persisted three-maker privacy budget", async () => {
  const fixture = await setup();
  try {
    const makers = [];
    for (let i = 0; i < 4; i++) {
      const identity = await MessagingIdentity.create();
      makers.push(identity);
      await fixture.relay.announce(
        await announce(identity, {
          network: domain.network,
          deployment_id: domain.deployment_id,
          key_version: 1,
          pairs: ["ZEC/USDC"],
          policy: domain.policy,
          deadline: Date.now() + 60_000,
        }),
      );
    }
    const sent = [];
    const relay = {
      makers: (...args) => fixture.relay.makers(...args),
      send: async (message, signal) => {
        sent.push(message);
        await fixture.relay.send(message, signal);
      },
      poll: (...args) => fixture.relay.poll(...args),
      announce: (...args) => fixture.relay.announce(...args),
    };
    const config = {
      domain,
      maker_ids: makers.map((maker) => maker.id),
      core: fixture.core,
      store: fixture.client_store,
      relay,
      collection_ms: 100,
    };
    const id = random_id();
    const request = { from: "ZEC", to: "USDC", amount: "5" };
    const first = await new RfqClient(config).offers(id, request);
    assert.equal(first.requested_makers.length, 3);
    const second = await new RfqClient(config).offers(id, request);
    assert.deepEqual(second.requested_makers, first.requested_makers);
    assert.deepEqual(sent.slice(0, 3), sent.slice(3));
  } finally {
    await fixture.close();
  }
});

test("a private journal failure blocks delivery before contacting a maker", async () => {
  const fixture = await setup();
  let sends = 0;
  try {
    await fixture.maker.start(options);
    const config = {
      ...fixture.client_config,
      store: {
        read: async () => undefined,
        list: async () => [],
        create: async () => {
          throw new Error("storage unavailable");
        },
      },
      relay: {
        makers: (...args) => fixture.relay.makers(...args),
        send: async (...args) => {
          sends++;
          return fixture.relay.send(...args);
        },
        poll: (...args) => fixture.relay.poll(...args),
      },
    };
    await assert.rejects(
      new RfqClient(config).offers(random_id(), { from: "ZEC", to: "USDC", amount: "5" }),
    );
    assert.equal(sends, 0);
    assert.equal(fixture.price_calls(), 0);
  } finally {
    await fixture.close();
  }
});
test("maker startup cannot run twice or resurrect itself after stop", async () => {
  const fixture = await setup();
  const maker = new RfqMaker({ ...fixture.maker_config, preflight: () => new Promise(() => {}) });
  try {
    const starting = maker.start(options);
    const rejected = starting.catch((error) => error);
    await assert.rejects(maker.start(options), { code: "invalid_input" });
    await maker.stop();
    assert.equal((await rejected).code, "aborted");
    assert.equal((await fixture.relay.makers()).length, 0);
  } finally {
    await maker.stop();
    await fixture.close();
  }
});
test("acceptance never substitutes an offer from another intent", async () => {
  const fixture = await setup();
  try {
    await fixture.maker.start(options);
    const sdk = fixture.make_client();
    const first = sdk.intent({ from: "ZEC", to: "USDC", amount: "5" });
    const first_offers = await first.offers();
    const second = sdk.intent({ from: "ZEC", to: "USDC", amount: "6" });
    const second_offers = await second.offers();
    await assert.rejects(first.accept({ offer_id: second_offers.items[0].id }), {
      code: "offer_unavailable",
    });
    await assert.rejects(first.accept({ offer_id: first_offers.items[0].id }), {
      code: "deployment_unavailable",
    });
  } finally {
    await fixture.close();
  }
});

test("a directory cannot select recipients without the owner's permitted identities", async () => {
  const fixture = await setup();
  try {
    assert.throws(() => new RfqClient({ ...fixture.client_config, maker_ids: [] }), {
      code: "wallet_unavailable",
    });
    await fixture.maker.start(options);
    const id = random_id();
    const input = { from: "ZEC", to: "USDC", amount: "5" };
    const first = await new RfqClient(fixture.client_config).offers(id, input);
    assert.equal(first.items.length, 1);
    await assert.rejects(
      new RfqClient({ ...fixture.client_config, maker_ids: ["aa".repeat(32)] }).offers(id, input),
      { code: "persistence_failure" },
    );
  } finally {
    await fixture.close();
  }
});
test("aborting a quote wait retains the plan without granting payment authority", async () => {
  const fixture = await setup();
  try {
    await fixture.maker.start(options);
    await fixture.maker.pause();
    const id = random_id();
    const controller = new AbortController();
    const waiting = new RfqClient(fixture.client_config).offers(
      id,
      { from: "ZEC", to: "USDC", amount: "5" },
      controller.signal,
    );
    const rejected = waiting.catch((error) => error);
    setTimeout(() => controller.abort(), 150);
    assert.equal((await rejected).code, "aborted");
    const plan = await fixture.client_store.read(`intent-${id}`);
    assert.equal(plan.selected.length, 1);
    assert.equal((await fixture.maker.summary()).active_swaps, 0);
  } finally {
    await fixture.close();
  }
});

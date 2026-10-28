import test from "node:test";
import assert from "node:assert/strict";
import { RfqClient, random_id } from "../dist/index.js";
import { setup, options } from "../../../tests/support/rfq.mjs";

test("a maker that loses readiness after startup cannot issue a new quote", async () => {
  const fixture = await setup();
  let ready = true;
  let checks = 0;
  fixture.maker_config.preflight = async () => {
    checks++;
    if (!ready) throw new Error("source or inventory evidence is unavailable");
  };
  try {
    await fixture.maker.start(options);
    ready = false;
    const client = new RfqClient(fixture.client_config);
    const collection = await client.offers(random_id(), { from: "ZEC", to: "USDC", amount: "5" });
    assert.equal(collection.items.length, 0);
    assert.equal(fixture.price_calls(), 0);
    assert.ok(checks >= 2);
  } finally {
    await fixture.close();
  }
});

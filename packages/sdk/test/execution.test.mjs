import test from "node:test";
import assert from "node:assert/strict";
import { create_zutti } from "../dist/index.js";
import { RfqClient } from "../../runtime/dist/index.js";
import { setup, profile, options } from "../../../tests/support/rfq.mjs";

const destination = `0x${"88".repeat(20)}`;
const evidence = {
  chain: "ethereum",
  block_hash: "11".repeat(32),
  height: "12000000",
  transaction_id: "22".repeat(32),
  policy_id: profile.policy.source_policy_id,
};

test("withdrawals reject invalid principals before requesting wallet authority", async () => {
  let calls = 0;
  const sdk = create_zutti({
    network: "testnet",
    wallet: {
      zutti: () => ({
        profile,
        account: {
          withdraw: async () => {
            calls++;
            throw new Error("unexpected authorization");
          },
        },
      }),
    },
  });
  try {
    for (const amount of ["0", "0.000000", "0.0000001", "1e6", "-1", "01"])
      await assert.rejects(sdk.account().withdraw({ asset: "USDC", amount, to: destination }));
    assert.equal(calls, 0);
    assert.throws(() => sdk.intent({ from: "ZEC", to: "USDC", amount: "1" }), {
      code: "wallet_unavailable",
    });
  } finally {
    await sdk.close();
  }
});

test("withdrawal completion must preserve the authorized recipient, amount and deployment", async () => {
  const receipt = {
    operation_id: "33".repeat(32),
    kind: "withdrawal",
    amount: { asset: "USDC", amount: "1" },
    output_mode: "public_erc20",
    to: destination,
    evidence,
    confirmed_at: Date.now(),
  };
  let current = receipt;
  const sdk = create_zutti({
    network: "testnet",
    wallet: {
      zutti: () => ({
        profile,
        account: {
          withdraw: async (input) => {
            assert.ok(Object.isFrozen(input));
            return structuredClone(current);
          },
        },
      }),
    },
  });
  try {
    for (const change of [
      (value) => {
        value.to = `0x${"99".repeat(20)}`;
      },
      (value) => {
        value.amount.amount = "0.9";
      },
      (value) => {
        value.evidence.policy_id = "99".repeat(32);
      },
      (value) => {
        value.evidence.chain = "zcash";
      },
    ]) {
      current = structuredClone(receipt);
      change(current);
      await assert.rejects(
        sdk.account().withdraw({ asset: "USDC", amount: "1", to: destination }),
        { code: "invalid_message" },
      );
    }
    current = receipt;
    const result = await sdk.account().withdraw({ asset: "USDC", amount: "1", to: destination });
    assert.ok(Object.isFrozen(result));
    await sdk.close();
    await assert.rejects(sdk.account().withdraw({ asset: "USDC", amount: "1", to: destination }), {
      code: "aborted",
    });
  } finally {
    await sdk.close();
  }
});

test("SDK acceptance binds a wallet result to its authenticated RFQ amount and source policy", async () => {
  const fixture = await setup();
  let current;
  let received;
  const client = create_zutti({
    network: "testnet",
    wallet: {
      zutti: () => ({
        profile,
        rfq: new RfqClient(fixture.client_config),
        execution: {
          accept: async (offer) => {
            received = offer;
            return current;
          },
        },
      }),
    },
  });
  try {
    await fixture.maker.start(options);
    const intent = client.intent({ from: "ZEC", to: "USDC", amount: "5" });
    const offers = await intent.offers();
    const receipt = {
      outcome: "settled",
      swap_id: "44".repeat(32),
      received: offers.items[0].output,
      source_payment: { ...evidence, chain: "zcash" },
      pool_settlement: evidence,
      confirmed_at: Date.now(),
    };
    for (const change of [
      (value) => {
        value.received.amount = "198";
      },
      (value) => {
        value.source_payment.policy_id = "99".repeat(32);
      },
    ]) {
      current = structuredClone(receipt);
      change(current);
      await assert.rejects(intent.accept({ offer_id: offers.items[0].id }), {
        code: "invalid_message",
      });
    }
    current = receipt;
    const swap = await intent.accept({ offer_id: offers.items[0].id });
    assert.equal(received.id, offers.items[0].id);
    assert.ok(Object.isFrozen(received));
    assert.equal((await swap.summary()).stage, "settled");
  } finally {
    await client.close();
    await fixture.close();
  }
});

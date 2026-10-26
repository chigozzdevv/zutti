import test from "node:test";
import assert from "node:assert/strict";
import {
  amount,
  intent_input,
  envelope,
  swap_policy,
  rfq_domain,
  require_pair,
} from "../dist/index.js";

test("transport inputs reject floating point, excess fields and unsupported routes", () => {
  for (const value of [5, "-1", "1e3", "01", " 5", "5.", "NaN"])
    assert.equal(amount.safeParse(value).success, false);
  assert.equal(intent_input.safeParse({ from: "ZEC", to: "ZEC", amount: "5" }).success, false);
  assert.equal(
    intent_input.safeParse({ from: "ZEC", to: "USDC", amount: "5", destination: "public" }).success,
    false,
  );
  assert.deepEqual(intent_input.parse({ from: "ZEC", to: "USDC", amount: "5" }), {
    from: "ZEC",
    to: "USDC",
    amount: "5",
  });
});

test("checkpoint profiles bind a Testnet origin and reject reverse quotes", () => {
  const policy = swap_policy.parse({
    payment_profile_id: "11".repeat(32),
    source_policy_id: "22".repeat(32),
    proof_profile_id: "33".repeat(32),
    ethereum_finality: "finalized",
    min_source_window_blocks: 10,
    max_source_window_blocks: 100,
    paid_claim_deadline: null,
    source_checkpoint: {
      commitment: "44".repeat(32),
      height: 4_500_000,
      hash: "55".repeat(32),
      program_id: "66".repeat(32),
    },
  });
  assert.doesNotThrow(() => require_pair(policy, "ZEC", "USDC"));
  assert.throws(() => require_pair(policy, "USDC", "ZEC"), { code: "invalid_input" });
  assert.equal(swap_policy.safeParse({ ...policy, max_source_window_blocks: 129 }).success, false);
  assert.ok(
    rfq_domain.safeParse({ network: "testnet", deployment_id: "77".repeat(32), policy }).success,
  );
  assert.equal(
    rfq_domain.safeParse({ network: "mainnet", deployment_id: "77".repeat(32), policy }).success,
    false,
  );
  assert.equal(
    swap_policy.safeParse({
      ...policy,
      source_checkpoint: { ...policy.source_checkpoint, height: 0 },
    }).success,
    false,
  );
});
test("policy cannot add a paid claim deadline or relax finalized Ethereum", () => {
  const policy = {
    payment_profile_id: "11".repeat(32),
    source_policy_id: "22".repeat(32),
    proof_profile_id: "33".repeat(32),
    ethereum_finality: "finalized",
    min_source_window_blocks: 10,
    max_source_window_blocks: 100,
    paid_claim_deadline: null,
  };
  assert.ok(swap_policy.safeParse(policy).success);
  for (const change of [
    { ethereum_finality: "confirmed" },
    { paid_claim_deadline: 1234 },
    { min_source_window_blocks: 101 },
  ])
    assert.equal(swap_policy.safeParse({ ...policy, ...change }).success, false);
  assert.equal(envelope.safeParse({ ...policy, ciphertext: "00" }).success, false);
});

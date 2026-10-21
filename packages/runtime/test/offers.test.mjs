import test from "node:test";
import assert from "node:assert/strict";
import { create_native_core } from "../../bindings/dist/native.js";
import { digest, verify_offer } from "../dist/index.js";
import { domain } from "../../../tests/support/rfq-domain.mjs";

test("even a correctly identified maker cannot hide wrong fee arithmetic or change request terms", async () => {
  const core = create_native_core(process.env.ZUTTI_CORE_BINARY);
  const request = { from: "ZEC", to: "USDC", amount: "5", min_output: "198" };
  const deadline = Date.now() + 10_000;
  const body = {
    intent_id: "55".repeat(32),
    request_id: "66".repeat(32),
    maker_id: "77".repeat(32),
    input: { asset: "ZEC", amount: "5" },
    gross_output: { asset: "USDC", amount: "200" },
    maker_fee: { bps: 50, amount: { asset: "USDC", amount: "1" } },
    output: { asset: "USDC", amount: "199" },
    fees: [
      {
        kind: "source_network",
        asset: "ZEC",
        paid_by: "user",
        pay_from: "public_wallet",
        max_amount: "0.001",
        estimated_amount: null,
      },
      {
        kind: "ethereum_execution",
        asset: "ETH",
        paid_by: "maker",
        pay_from: "maker_inventory",
        max_amount: "0.005",
        estimated_amount: null,
      },
    ],
    deadline,
    policy: domain.policy,
    nonce: "88".repeat(32),
  };
  const verify = async (value) =>
    verify_offer(
      { ...value, id: await digest("offer", value) },
      request,
      body.intent_id,
      body.request_id,
      body.maker_id,
      domain,
      core,
      Date.now(),
      deadline,
    );
  await verify(body);
  for (const change of [
    (value) => {
      value.maker_fee.amount.amount = "0";
    },
    (value) => {
      value.output.amount = "200";
    },
    (value) => {
      value.input.amount = "6";
    },
    (value) => {
      value.request_id = "99".repeat(32);
    },
    (value) => {
      value.policy.source_policy_id = "99".repeat(32);
    },
    (value) => {
      value.fees.pop();
    },
    (value) => {
      value.fees[0].estimated_amount = "1";
    },
    (value) => {
      value.fees[0].pay_from = "maker_inventory";
    },
    (value) => {
      value.deadline += 1;
    },
  ]) {
    const value = structuredClone(body);
    change(value);
    await assert.rejects(verify(value));
  }
  const signed_id = await digest("offer", body);
  await assert.rejects(
    verify_offer(
      { ...body, id: signed_id, output: { asset: "USDC", amount: "198" } },
      request,
      body.intent_id,
      body.request_id,
      body.maker_id,
      domain,
      core,
      Date.now(),
      deadline,
    ),
  );
});

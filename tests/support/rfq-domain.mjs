export const domain = {
  network: "testnet",
  deployment_id: "44".repeat(32),
  policy: {
    payment_profile_id: "11".repeat(32),
    source_policy_id: "22".repeat(32),
    proof_profile_id: "33".repeat(32),
    ethereum_finality: "finalized",
    min_source_window_blocks: 10,
    max_source_window_blocks: 100,
    paid_claim_deadline: null,
  },
};
export const profile = {
  name: "testnet",
  deployment_id: domain.deployment_id,
  zcash: { genesis_hash: "99".repeat(32), payment_profile_id: domain.policy.payment_profile_id },
  ethereum: {
    chain_id: "11155111",
    usdc: `0x${"11".repeat(20)}`,
    pool: `0x${"22".repeat(20)}`,
    registry: `0x${"33".repeat(20)}`,
  },
  policy: domain.policy,
};
export const options = {
  pairs: ["ZEC/USDC", "USDC/ZEC"],
  fee_bps: 50,
  quote_deadline: "15s",
  limits: {
    per_trade: { ZEC: { min: "0.1", max: "10" }, USDC: { min: "5", max: "500" } },
    max_committed: { ZEC: "10", USDC: "3000" },
    max_fee_per_swap: { ZEC: "0.001", ETH: "0.005", USDC: "1" },
    max_active_swaps: 20,
  },
};

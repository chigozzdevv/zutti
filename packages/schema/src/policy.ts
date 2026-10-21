import { z } from "zod";

export const digest = z
  .string()
  .regex(/^[0-9a-f]{64}$/)
  .refine((value) => value !== "0".repeat(64));
export const network = z.enum(["mainnet", "testnet"]);
export const timestamp = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const pair = z.enum(["ZEC/USDC", "USDC/ZEC"]);
export const swap_policy = z
  .strictObject({
    payment_profile_id: digest,
    source_policy_id: digest,
    proof_profile_id: digest,
    ethereum_finality: z.literal("finalized"),
    min_source_window_blocks: z.number().int().min(1).max(100_000),
    max_source_window_blocks: z.number().int().min(1).max(100_000),
    paid_claim_deadline: z.null(),
  })
  .refine((value) => value.min_source_window_blocks <= value.max_source_window_blocks);
export const rfq_domain = z.strictObject({ network, deployment_id: digest, policy: swap_policy });
export type Network = z.infer<typeof network>;
export type SwapPolicy = z.infer<typeof swap_policy>;
export type RfqDomain = z.infer<typeof rfq_domain>;
export type Pair = z.infer<typeof pair>;
const address = z
  .string()
  .regex(/^0x[0-9a-fA-F]{40}$/)
  .refine((value) => value !== `0x${"0".repeat(40)}`);
export const network_profile = z
  .strictObject({
    name: network,
    deployment_id: digest,
    zcash: z.strictObject({ genesis_hash: digest, payment_profile_id: digest }),
    ethereum: z.strictObject({
      chain_id: z.string().regex(/^[1-9][0-9]{0,19}$/),
      usdc: address,
      pool: address,
      registry: address,
    }),
    policy: swap_policy,
  })
  .refine((value) => value.zcash.payment_profile_id === value.policy.payment_profile_id);
export type NetworkProfile = z.infer<typeof network_profile>;

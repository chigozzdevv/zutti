import { z } from "zod";
import { ZuttiError } from "./error.js";

export const digest = z
  .string()
  .regex(/^[0-9a-f]{64}$/)
  .refine((value) => value !== "0".repeat(64));
export const network = z.enum(["mainnet", "testnet"]);
export const timestamp = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const pair = z.enum(["ZEC/USDC", "USDC/ZEC"]);
export const source_checkpoint = z.strictObject({
  commitment: digest,
  height: z.number().int().min(1).max(499_999_999),
  hash: digest,
  program_id: digest,
});
export const swap_policy = z
  .strictObject({
    payment_profile_id: digest,
    source_policy_id: digest,
    proof_profile_id: digest,
    ethereum_finality: z.literal("finalized"),
    min_source_window_blocks: z.number().int().min(1).max(100_000),
    max_source_window_blocks: z.number().int().min(1).max(100_000),
    paid_claim_deadline: z.null(),
    source_checkpoint: source_checkpoint.optional(),
  })
  .refine((value) => value.min_source_window_blocks <= value.max_source_window_blocks)
  .refine((value) => !value.source_checkpoint || value.max_source_window_blocks <= 512);
export const rfq_domain = z
  .strictObject({ network, deployment_id: digest, policy: swap_policy })
  .refine((value) => value.network === "testnet" || value.policy.source_checkpoint === undefined);
export type Network = z.infer<typeof network>;
export type SwapPolicy = z.infer<typeof swap_policy>;
export type RfqDomain = z.infer<typeof rfq_domain>;
export type Pair = z.infer<typeof pair>;
export function require_pair(policy: SwapPolicy, from: string, to: string): void {
  if (policy.source_checkpoint && (from !== "ZEC" || to !== "USDC"))
    throw new ZuttiError("invalid_input", "This deployment supports ZEC to USDC only");
}
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
  .refine((value) => value.zcash.payment_profile_id === value.policy.payment_profile_id)
  .refine((value) => value.name === "testnet" || value.policy.source_checkpoint === undefined)
  .refine((value) => !value.policy.source_checkpoint || value.ethereum.chain_id === "11155111");
export type NetworkProfile = z.infer<typeof network_profile>;

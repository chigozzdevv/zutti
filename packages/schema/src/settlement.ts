import { z } from "zod";
import { amount, asset_amount } from "./amount.js";
import { digest, timestamp } from "./policy.js";

const chain_hash = z.string().regex(/^(?:0x)?[0-9a-fA-F]{64}$/);
export const evidence_reference = z.strictObject({
  chain: z.enum(["zcash", "ethereum"]),
  block_hash: chain_hash,
  height: z.string().regex(/^[1-9][0-9]*$/),
  transaction_id: chain_hash,
  policy_id: digest,
});
export const settled_receipt = z.strictObject({
  outcome: z.literal("settled"),
  swap_id: digest,
  received: asset_amount,
  source_payment: evidence_reference,
  pool_settlement: evidence_reference,
  confirmed_at: timestamp,
});
export type SettledReceipt = z.infer<typeof settled_receipt>;
export const withdrawal_input = z.strictObject({
  asset: z.literal("USDC"),
  amount: amount.regex(/^[0-9]+(?:\.[0-9]{1,6})?$/).refine((value) => /[1-9]/.test(value)),
  to: z
    .string()
    .regex(/^0x[0-9a-fA-F]{40}$/)
    .refine((value) => !/^0x0{40}$/.test(value)),
});
export const withdrawal_receipt = z.strictObject({
  operation_id: digest,
  kind: z.literal("withdrawal"),
  amount: asset_amount.refine((value) => value.asset === "USDC"),
  output_mode: z.literal("public_erc20"),
  to: withdrawal_input.shape.to,
  evidence: evidence_reference,
  confirmed_at: timestamp,
});
export type WithdrawalInput = z.infer<typeof withdrawal_input>;
export type WithdrawalReceipt = z.infer<typeof withdrawal_receipt>;
export interface SettledSwap {
  readonly id: string;
  readonly receipt: Readonly<SettledReceipt>;
  summary(): Promise<{ id: string; stage: "settled"; receipt: Readonly<SettledReceipt> }>;
}

import { z } from "zod";
import { amount, asset_amount } from "./amount.js";
import { digest, timestamp } from "./policy.js";
import { maker_start_options } from "./maker.js";
import { evidence_reference, withdrawal_input } from "./settlement.js";

export const maker_inventory = z.strictObject({
  maker_id: digest,
  deployment_id: digest,
  available_usdc: amount.nullable(),
  reserved_usdc: amount.nullable(),
  available_zec: amount.nullable(),
  checked_at: timestamp,
});
export const maker_trade = z.strictObject({
  id: digest,
  input: asset_amount,
  output: asset_amount,
  state: z.enum(["reserved", "payment_confirming", "settling", "settled", "pending"]),
  updated_at: timestamp,
});
export const maker_trades = z.strictObject({
  maker_id: digest,
  deployment_id: digest,
  items: z.array(maker_trade).max(100),
  complete: z.boolean(),
  checked_at: timestamp,
});
export const maker_control = z.discriminatedUnion("action", [
  z.strictObject({ action: z.literal("start"), settings: maker_start_options }),
  z.strictObject({ action: z.literal("pause") }),
  z.strictObject({ action: z.literal("resume") }),
  z.strictObject({ action: z.literal("stop") }),
]);
export const maker_deposit_input = z.strictObject({
  id: digest,
  asset: z.literal("USDC"),
  amount: withdrawal_input.shape.amount,
});
export const maker_deposit_receipt = z.strictObject({
  id: digest,
  asset: z.literal("USDC"),
  amount: withdrawal_input.shape.amount,
  evidence: evidence_reference,
  confirmed_at: timestamp,
});
export const maker_deposit_operation = z.strictObject({
  id: digest,
  amount: withdrawal_input.shape.amount,
  state: z.enum(["pending", "confirmed", "paused"]),
  receipt: maker_deposit_receipt.nullable(),
  error: z.string().nullable(),
});
export const maker_snapshot = z.strictObject({
  available: z.boolean(),
  maker_id: digest.nullable(),
  state: z.enum(["unavailable", "stopped", "running", "paused"]),
  settings: maker_start_options.nullable(),
  live_offers: z.number().int().nonnegative().nullable(),
  active_trades: z.number().int().nonnegative().nullable(),
  inventory: maker_inventory.nullable(),
  trades: maker_trades.nullable(),
  can_deposit: z.boolean(),
  deposit: maker_deposit_operation.nullable(),
  pending_action: z.enum(["start", "pause", "resume", "stop"]).nullable(),
  error: z.string().nullable(),
  checked_at: timestamp,
});
export type MakerInventory = z.infer<typeof maker_inventory>;
export type MakerTrades = z.infer<typeof maker_trades>;
export type MakerSnapshot = z.infer<typeof maker_snapshot>;
export type MakerControl = z.infer<typeof maker_control>;
export type MakerDepositInput = z.infer<typeof maker_deposit_input>;
export type MakerDepositReceipt = z.infer<typeof maker_deposit_receipt>;
export type MakerDepositOperation = z.infer<typeof maker_deposit_operation>;

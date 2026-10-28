import { z } from "zod";
import { asset_amount, intent_input } from "./amount.js";
import { digest, network_profile } from "./policy.js";
import { settled_receipt, withdrawal_receipt } from "./settlement.js";
import type { OfferCollection } from "./rfq.js";
import type { StatusUpdate } from "./status.js";

export const activity_item = z.strictObject({
  id: digest,
  input: asset_amount,
  output: asset_amount,
  settled_at: z.number().int().nonnegative(),
  withdrawn_at: z.number().int().nonnegative().nullable(),
  source_transaction: z.string().regex(/^[0-9a-f]{64}$/),
  payout_transaction: z.string().regex(/^0x[0-9a-f]{64}$/),
  withdrawal_transaction: z
    .string()
    .regex(/^0x[0-9a-f]{64}$/)
    .nullable(),
  duration_seconds: z.number().nonnegative(),
});
export const app_information = z.strictObject({
  profile: network_profile,
  activity: z.array(activity_item).max(20),
  wallet_available: z.boolean(),
  witnesses: z.literal("public-testnet"),
  refunds: z.literal(false),
});
export const quote_input = intent_input.refine(
  (value) =>
    value.from === "ZEC" &&
    value.to === "USDC" &&
    /^(0|[1-9][0-9]{0,7})(\.[0-9]{1,8})?$/.test(value.amount) &&
    /[1-9]/.test(value.amount),
);
export const accept_input = z.strictObject({ offer_id: digest });
export const app_operation = z.object({
  id: digest,
  parent_id: digest.nullable(),
  input: asset_amount.nullable(),
  kind: z.enum(["swap", "withdrawal"]),
  state: z.enum(["pending", "settled", "confirmed", "paused"]),
  started_at: z.number().int(),
  updated_at: z.number().int(),
  events: z
    .array(
      z.object({
        stage: z.string(),
        message: z.string(),
        at: z.number().int(),
      }),
    )
    .max(128),
  receipt: settled_receipt.nullable(),
  withdrawal: withdrawal_receipt.nullable(),
  error: z.string().nullable(),
});
export type ActivityItem = z.infer<typeof activity_item>;
export type AppInformation = z.infer<typeof app_information>;
export type AppOperation = z.infer<typeof app_operation>;
export type AppEvent = AppOperation["events"][number];
export interface QuoteResponse {
  id: string;
  offers: OfferCollection;
}
export interface WalletSession {
  connected: boolean;
  label: string | null;
}
export type AppStatusHandler = (value: StatusUpdate) => void;

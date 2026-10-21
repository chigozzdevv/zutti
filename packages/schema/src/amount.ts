import { z } from "zod";

export const asset = z.enum(["ZEC", "USDC"]);
export const fee_asset = z.enum(["ZEC", "USDC", "ETH"]);
export const amount = z
  .string()
  .max(80)
  .regex(/^(0|[1-9][0-9]*)(\.[0-9]+)?$/);
export const asset_amount = z.strictObject({ asset, amount });
export const fee_cap = z.strictObject({ asset: fee_asset, amount });
export const intent_input = z.discriminatedUnion("from", [
  z.strictObject({
    from: z.literal("ZEC"),
    to: z.literal("USDC"),
    amount,
    min_output: amount.optional(),
  }),
  z.strictObject({
    from: z.literal("USDC"),
    to: z.literal("ZEC"),
    amount,
    min_output: amount.optional(),
  }),
]);
export type Asset = z.infer<typeof asset>;
export type FeeAsset = z.infer<typeof fee_asset>;
export type AssetAmount = z.infer<typeof asset_amount>;
export type IntentInput = z.infer<typeof intent_input>;

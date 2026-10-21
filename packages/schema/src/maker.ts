import { z } from "zod";
import { amount } from "./amount.js";
import { pair } from "./policy.js";

const range = z.strictObject({ min: amount, max: amount });
export const maker_start_options = z.strictObject({
  pairs: z
    .array(pair)
    .min(1)
    .max(2)
    .refine((value) => new Set(value).size === value.length),
  fee_bps: z.number().int().min(0).max(9999),
  quote_deadline: z.string().regex(/^(?:[1-9]|[1-5][0-9]|60)s$/),
  limits: z.strictObject({
    per_trade: z.strictObject({ ZEC: range, USDC: range }),
    max_committed: z.strictObject({ ZEC: amount, USDC: amount }),
    max_fee_per_swap: z.strictObject({ ZEC: amount, USDC: amount, ETH: amount }),
    max_active_swaps: z.number().int().min(1).max(1000),
  }),
});
export type MakerStartOptions = z.infer<typeof maker_start_options>;

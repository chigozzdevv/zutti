import { z } from "zod";
import { digest } from "./policy.js";

const address = z.string().regex(/^0x[0-9a-fA-F]{40}$/);
const hash = z.string().regex(/^0x[0-9a-fA-F]{64}$/);
export const pool_events = z.strictObject({
  events: z.strictObject({
    schema: z.literal(1),
    chain_id: z.literal(11155111),
    pool: address,
    domain: digest,
    root: hash,
    next_leaf: z.number().int().min(0).max(65536),
    execution_hash: hash,
    execution_height: z.number().int().positive(),
    outputs: z
      .array(
        z.strictObject({
          position: z.number().int().min(0),
          commitment: hash,
          ciphertext: z
            .string()
            .regex(/^0x(?:[0-9a-f]{2})+$/)
            .max(8194),
        }),
      )
      .max(65536),
  }),
  transactions: z
    .array(
      z.strictObject({
        id: hash,
        from: address,
        kind: z.enum(["deposit", "transfer", "withdrawal", "reservation", "settlement"]),
        amount: z.string().nullable(),
        at: z.number().int().positive(),
      }),
    )
    .max(65536),
  checked_at: z.number().int().positive(),
});
export type PoolEvents = z.infer<typeof pool_events>;

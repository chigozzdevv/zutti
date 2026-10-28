import { z } from "zod";
import { digest } from "./policy.js";

const ethereum_address = z
  .string()
  .regex(/^0x[0-9a-fA-F]{40}$/)
  .refine((value) => !/^0x0{40}$/.test(value));
export const browser_account = z.strictObject({
  version: z.literal(1),
  network: z.literal("testnet"),
  chain_id: z.literal(11155111),
  domain: digest,
  zcash_address: z.string().regex(/^utest1[023456789acdefghjklmnpqrstuvwxyz]{80,300}$/),
  usdc_receiver: z.strictObject({
    owner: z.string().regex(/^(02|03)[0-9a-f]{64}$/),
    viewing: digest,
    nullifier_key_commitment: digest,
  }),
});
export const browser_connection = z.strictObject({
  account: browser_account,
  ethereum_account: ethereum_address,
});
export type BrowserAccount = z.infer<typeof browser_account>;
export type BrowserConnection = z.infer<typeof browser_connection>;

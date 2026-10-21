import { z } from "zod";
import { asset_amount, amount, fee_asset, intent_input } from "./amount.js";
import { digest, network, pair, swap_policy, timestamp } from "./policy.js";

export const MAX_MESSAGE_BYTES = 65_536;
export const MAX_MAKERS = 3;
export const MAX_QUOTE_MS = 60_000;
export const MAX_ANNOUNCEMENT_MS = 900_000;
export const HPKE_SUITE = "DHKEM(X25519,HKDF-SHA256)/HKDF-SHA256/AES-128-GCM";
export const signature = z.string().regex(/^[0-9a-f]{128}$/);
export const maker_announcement_body = z.strictObject({
  version: z.literal(1),
  network,
  deployment_id: digest,
  maker_id: digest,
  signing_key: digest,
  encryption_key: digest,
  key_version: z.literal(1),
  pairs: z
    .array(pair)
    .min(1)
    .max(2)
    .refine((pairs) => new Set(pairs).size === pairs.length),
  policy: swap_policy,
  deadline: timestamp,
});
export const maker_announcement = maker_announcement_body.extend({ signature });
export const request_body = z.strictObject({
  version: z.literal(1),
  intent_id: digest,
  request_id: digest,
  request: intent_input,
  reply_key: digest,
  policy: swap_policy,
  deadline: timestamp,
});
export const fee_quote = z.strictObject({
  kind: z.enum(["source_network", "ethereum_execution", "submission"]),
  asset: fee_asset,
  paid_by: z.enum(["user", "maker"]),
  pay_from: z.enum(["public_wallet", "private_balance", "maker_inventory"]),
  max_amount: amount,
  estimated_amount: amount.nullable(),
});
export const offer_body = z.strictObject({
  intent_id: digest,
  request_id: digest,
  maker_id: digest,
  input: asset_amount,
  gross_output: asset_amount,
  maker_fee: z.strictObject({ bps: z.number().int().min(0).max(9999), amount: asset_amount }),
  output: asset_amount,
  fees: z.array(fee_quote).min(1).max(6),
  deadline: timestamp,
  policy: swap_policy,
  nonce: digest,
});
export const wire_offer = offer_body.extend({ id: digest });
export const decline = z.strictObject({
  request_id: digest,
  reason: z.enum(["limits", "unavailable", "unsupported"]),
});
export const envelope_header = z.strictObject({
  version: z.literal(1),
  suite: z.literal(HPKE_SUITE),
  network,
  deployment_id: digest,
  type: z.enum(["request", "offer", "decline"]),
  request_id: digest,
  sender_id: digest,
  recipient_id: digest,
  sender_key: digest,
  recipient_key: digest,
  nonce: digest,
  deadline: timestamp,
});
export const envelope_body = envelope_header.extend({
  enc: digest,
  ciphertext: z
    .string()
    .min(32)
    .max(MAX_MESSAGE_BYTES * 2)
    .regex(/^(?:[0-9a-f]{2})+$/),
});
export const envelope = envelope_body.extend({ signature });
export const mailbox_auth_body = z.strictObject({
  version: z.literal(1),
  purpose: z.literal("poll"),
  network,
  deployment_id: digest,
  recipient_id: digest,
  recipient_key: digest,
  nonce: digest,
  deadline: timestamp,
});
export const mailbox_auth = mailbox_auth_body.extend({ signature });
export type MakerAnnouncement = z.infer<typeof maker_announcement>;
export type MakerAnnouncementBody = z.infer<typeof maker_announcement_body>;
export type RequestBody = z.infer<typeof request_body>;
export type WireOffer = z.infer<typeof wire_offer>;
export type OfferBody = z.infer<typeof offer_body>;
export type Envelope = z.infer<typeof envelope>;
export type EnvelopeHeader = z.infer<typeof envelope_header>;
export type MailboxAuth = z.infer<typeof mailbox_auth>;
export type FeeQuote = z.infer<typeof fee_quote>;
export type Offer = Omit<WireOffer, "nonce" | "request_id"> & {
  readonly authentication: "verified";
};
export interface OfferCollection {
  readonly intent_id: string;
  readonly items: readonly Offer[];
  readonly requested_makers: readonly string[];
  readonly responded_makers: readonly string[];
  readonly collection: "complete" | "timed_out";
  readonly collected_at: number;
}

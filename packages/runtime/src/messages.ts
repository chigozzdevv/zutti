import {
  HPKE_SUITE,
  MAX_ANNOUNCEMENT_MS,
  MAX_MESSAGE_BYTES,
  MAX_QUOTE_MS,
  ZuttiError,
  envelope,
  envelope_body,
  envelope_header,
  mailbox_auth,
  maker_announcement,
  require_pair,
} from "@zutti/schema";
import type {
  Envelope,
  EnvelopeHeader,
  MailboxAuth,
  MakerAnnouncement,
  MakerAnnouncementBody,
  RfqDomain,
} from "@zutti/schema";
import { canonical, hex, random_id, text, unhex } from "./encoding.js";
import { MessagingIdentity, identity_id, suite, verify_signature } from "./identity.js";

export function check_domain(
  domain: RfqDomain,
  value: { network: string; deployment_id: string },
): void {
  if (domain.network !== value.network || domain.deployment_id !== value.deployment_id)
    throw new ZuttiError("invalid_message", "Message belongs to another deployment");
}
function fresh(deadline: number, now: number, lifetime: number): void {
  if (!Number.isSafeInteger(now) || deadline <= now || deadline - now > lifetime + 5_000)
    throw new ZuttiError("expired", "Message deadline is invalid or expired");
}
export async function announce(
  identity: MessagingIdentity,
  body: Omit<MakerAnnouncementBody, "version" | "maker_id" | "signing_key" | "encryption_key">,
): Promise<MakerAnnouncement> {
  const value = {
    ...body,
    version: 1 as const,
    maker_id: identity.id,
    signing_key: identity.signing_key,
    encryption_key: identity.encryption_key,
  };
  return maker_announcement.parse({
    ...value,
    signature: await identity.sign("announcement", value),
  });
}
export async function verify_announcement(
  value: unknown,
  domain: RfqDomain,
  now: number,
): Promise<MakerAnnouncement> {
  const announcement = maker_announcement.parse(value);
  for (const pair of announcement.pairs) {
    const [from = "", to = ""] = pair.split("/");
    require_pair(domain.policy, from, to);
  }
  check_domain(domain, announcement);
  fresh(announcement.deadline, now, MAX_ANNOUNCEMENT_MS);
  if (
    announcement.maker_id !== (await identity_id(announcement.signing_key)) ||
    canonical(announcement.policy) !== canonical(domain.policy)
  )
    throw new ZuttiError("invalid_message", "Maker identity or policy does not match");
  const { signature, ...body } = announcement;
  await verify_signature("announcement", body, announcement.signing_key, signature);
  return announcement;
}
export async function seal_message(
  identity: MessagingIdentity,
  header: Omit<EnvelopeHeader, "version" | "suite" | "sender_id" | "sender_key" | "nonce">,
  value: unknown,
): Promise<Envelope> {
  const metadata = envelope_header.parse({
    ...header,
    version: 1,
    suite: HPKE_SUITE,
    sender_id: identity.id,
    sender_key: identity.signing_key,
    nonce: random_id(),
  });
  const plaintext = text(canonical(value));
  if (plaintext.length > MAX_MESSAGE_BYTES / 2)
    throw new ZuttiError("invalid_message", "Message exceeds its bound");
  const public_key = await suite.kem.deserializePublicKey(unhex(metadata.recipient_key));
  let sealed;
  try {
    sealed = await suite.seal(
      { recipientPublicKey: public_key, info: text("zutti-rfq/v1") },
      plaintext,
      text(canonical(metadata)),
    );
  } finally {
    plaintext.fill(0);
  }
  const body = { ...metadata, enc: hex(sealed.enc), ciphertext: hex(sealed.ct) };
  return envelope.parse({ ...body, signature: await identity.sign("envelope", body) });
}
export async function verify_envelope(
  value: unknown,
  domain: RfqDomain,
  now: number,
): Promise<Envelope> {
  const message = envelope.parse(value);
  check_domain(domain, message);
  fresh(message.deadline, now, MAX_QUOTE_MS);
  if (message.sender_id !== (await identity_id(message.sender_key)))
    throw new ZuttiError("invalid_signature", "Sender identity does not match its key");
  const { signature, ...body } = message;
  await verify_signature("envelope", envelope_body.parse(body), message.sender_key, signature);
  return message;
}
export async function open_message(
  identity: MessagingIdentity,
  value: unknown,
  domain: RfqDomain,
  now: number,
): Promise<unknown> {
  const message = await verify_envelope(value, domain, now);
  if (message.recipient_id !== identity.id || message.recipient_key !== identity.encryption_key)
    throw new ZuttiError("invalid_message", "Message is addressed to another recipient");
  try {
    const header = envelope_header.parse(
      Object.fromEntries(
        Object.entries(message).filter(
          ([key]) => !["enc", "ciphertext", "signature"].includes(key),
        ),
      ),
    );
    const plaintext = new Uint8Array(
      await suite.open(
        { recipientKey: identity.encryption, enc: unhex(message.enc), info: text("zutti-rfq/v1") },
        unhex(message.ciphertext),
        text(canonical(header)),
      ),
    );
    try {
      return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(plaintext)) as unknown;
    } finally {
      plaintext.fill(0);
    }
  } catch {
    throw new ZuttiError("invalid_message", "Encrypted message could not be opened");
  }
}
export async function authorize_poll(
  identity: MessagingIdentity,
  domain: RfqDomain,
  now: number,
): Promise<MailboxAuth> {
  const body = {
    version: 1 as const,
    purpose: "poll" as const,
    network: domain.network,
    deployment_id: domain.deployment_id,
    recipient_id: identity.id,
    recipient_key: identity.signing_key,
    nonce: random_id(),
    deadline: now + 10_000,
  };
  return { ...body, signature: await identity.sign("mailbox", body) };
}
export async function verify_poll(
  value: unknown,
  domain: RfqDomain,
  now: number,
): Promise<MailboxAuth> {
  const auth = mailbox_auth.parse(value);
  check_domain(domain, auth);
  fresh(auth.deadline, now, 10_000);
  if (auth.recipient_id !== (await identity_id(auth.recipient_key)))
    throw new ZuttiError("invalid_signature", "Mailbox identity does not match");
  const { signature, ...body } = auth;
  await verify_signature("mailbox", body, auth.recipient_key, signature);
  return auth;
}

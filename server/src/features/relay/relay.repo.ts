import { ZuttiError } from "@zutti/schema";
import type { Envelope, MakerAnnouncement } from "@zutti/schema";
import { canonical } from "@zutti/runtime";

export class RelayRepository {
  private readonly announcements = new Map<string, MakerAnnouncement>();
  private readonly messages = new Map<string, Envelope>();
  private readonly requests = new Map<string, Envelope>();
  private readonly poll_nonces = new Map<string, number>();
  prune(now: number): void {
    for (const [id, value] of this.announcements)
      if (value.deadline <= now) this.announcements.delete(id);
    for (const [id, value] of this.messages) if (value.deadline <= now) this.messages.delete(id);
    for (const [id, value] of this.requests) if (value.deadline <= now) this.requests.delete(id);
    for (const [id, deadline] of this.poll_nonces) if (deadline <= now) this.poll_nonces.delete(id);
  }
  makers(): MakerAnnouncement[] {
    return structuredClone([...this.announcements.values()]);
  }
  announce(value: MakerAnnouncement): void {
    const existing = this.announcements.get(value.maker_id);
    if (
      existing &&
      (existing.encryption_key !== value.encryption_key ||
        existing.signing_key !== value.signing_key ||
        existing.deadline > value.deadline)
    )
      throw new ZuttiError("replay", "Announcement changes pinned keys or rolls back its deadline");
    if (!existing && this.announcements.size >= 1000)
      throw new ZuttiError("service_unavailable", "Directory is full");
    this.announcements.set(value.maker_id, structuredClone(value));
  }
  send(message: Envelope): void {
    const nonce_key = `${message.sender_id}/${message.nonce}`;
    const existing = this.messages.get(nonce_key);
    if (existing) {
      if (canonical(existing) !== canonical(message))
        throw new ZuttiError("replay", "Message nonce was reused");
      return;
    }
    const key =
      message.type === "request"
        ? `${message.request_id}/${message.recipient_id}`
        : `${message.request_id}/${message.sender_id}`;
    const request = this.requests.get(key);
    if (message.type === "request") {
      if (request && canonical(request) !== canonical(message))
        throw new ZuttiError("replay", "Request identifier was reused");
      const maker = this.announcements.get(message.recipient_id);
      if (!maker || maker.encryption_key !== message.recipient_key)
        throw new ZuttiError("invalid_message", "Request recipient is unavailable");
    } else if (
      !request ||
      request.sender_id !== message.recipient_id ||
      request.recipient_id !== message.sender_id ||
      message.deadline > request.deadline
    )
      throw new ZuttiError("invalid_message", "Response has no matching request");
    if (
      this.messages.size >= 256 ||
      [...this.messages.values()].filter((value) => value.recipient_id === message.recipient_id)
        .length >= 32
    )
      throw new ZuttiError("service_unavailable", "Relay queue is full");
    this.messages.set(nonce_key, structuredClone(message));
    if (message.type === "request") this.requests.set(key, structuredClone(message));
  }
  poll(recipient: string, nonce: string, deadline: number): Envelope[] {
    const key = `${recipient}/${nonce}`;
    if (this.poll_nonces.has(key))
      throw new ZuttiError("replay", "Mailbox authorization was already used");
    if (this.poll_nonces.size >= 4096)
      throw new ZuttiError("service_unavailable", "Mailbox authorization budget is full");
    this.poll_nonces.set(key, deadline);
    const result: Envelope[] = [];
    let bytes = 0;
    for (const message of this.messages.values()) {
      if (message.recipient_id !== recipient) continue;
      const size = canonical(message).length;
      if (bytes + size > 524_288) break;
      result.push(message);
      bytes += size;
    }
    return structuredClone(result);
  }
}

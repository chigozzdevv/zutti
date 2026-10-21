import { ZuttiError, rfq_domain } from "@zutti/schema";
import type { RfqDomain } from "@zutti/schema";
import { verify_announcement, verify_envelope, verify_poll } from "@zutti/runtime";
import { RelayRepository } from "./relay.repo.js";

export class RelayService {
  readonly domain: RfqDomain;
  constructor(
    domain: RfqDomain,
    private readonly repository = new RelayRepository(),
  ) {
    this.domain = rfq_domain.parse(domain);
  }
  makers() {
    this.repository.prune(Date.now());
    return this.repository.makers();
  }
  async announce(value: unknown): Promise<void> {
    const now = Date.now();
    this.repository.prune(now);
    this.repository.announce(await verify_announcement(value, this.domain, now));
  }
  async send(value: unknown): Promise<void> {
    if (JSON.stringify(value).length > 131_072)
      throw new ZuttiError("invalid_message", "Message exceeds relay bounds");
    const now = Date.now();
    this.repository.prune(now);
    this.repository.send(await verify_envelope(value, this.domain, now));
  }
  async poll(value: unknown) {
    const now = Date.now();
    this.repository.prune(now);
    const auth = await verify_poll(value, this.domain, now);
    return this.repository.poll(auth.recipient_id, auth.nonce, auth.deadline);
  }
}

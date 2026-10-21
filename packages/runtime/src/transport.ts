import { ZuttiError } from "@zutti/schema";
import type { Envelope, MailboxAuth, MakerAnnouncement } from "@zutti/schema";

export interface RelayTransport {
  makers(signal?: AbortSignal): Promise<unknown[]>;
  announce(value: MakerAnnouncement, signal?: AbortSignal): Promise<void>;
  send(value: Envelope, signal?: AbortSignal): Promise<void>;
  poll(value: MailboxAuth, signal?: AbortSignal): Promise<unknown[]>;
}
export class HttpRelay implements RelayTransport {
  private readonly url: string;
  constructor(url: string) {
    const parsed = new URL(url);
    if (
      parsed.username ||
      parsed.password ||
      parsed.search ||
      parsed.hash ||
      parsed.pathname !== "/" ||
      (parsed.protocol !== "https:" &&
        !(
          parsed.protocol === "http:" &&
          ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname)
        ))
    )
      throw new ZuttiError("invalid_input", "Relay requires HTTPS or a loopback HTTP endpoint");
    this.url = parsed.origin;
  }
  private async request(path: string, body?: unknown, signal?: AbortSignal): Promise<unknown> {
    const timeout = AbortSignal.timeout(2_000);
    try {
      const response = await fetch(this.url + path, {
        method: body === undefined ? "GET" : "POST",
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        headers: { "content-type": "application/json" },
        signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
        redirect: "error",
        credentials: "omit",
      });
      if (!response.ok || !response.body) throw new Error("response");
      const reader = response.body.getReader();
      const chunks: Uint8Array[] = [];
      let size = 0;
      try {
        for (;;) {
          const chunk = await reader.read();
          if (chunk.done) break;
          size += chunk.value.length;
          if (size > 1_048_576) throw new Error("size");
          chunks.push(chunk.value);
        }
      } finally {
        await reader.cancel();
      }
      const buffer = new Uint8Array(size);
      let offset = 0;
      for (const chunk of chunks) {
        buffer.set(chunk, offset);
        offset += chunk.length;
      }
      return JSON.parse(new TextDecoder().decode(buffer)) as unknown;
    } catch {
      if (signal?.aborted) throw new ZuttiError("aborted", "RFQ collection was interrupted");
      throw new ZuttiError("service_unavailable", "RFQ relay is unavailable");
    }
  }
  async makers(signal?: AbortSignal): Promise<unknown[]> {
    const value = await this.request("/v1/makers", undefined, signal);
    if (!Array.isArray(value) || value.length > 1_000)
      throw new ZuttiError("invalid_message", "Invalid maker directory");
    return value;
  }
  async announce(value: MakerAnnouncement, signal?: AbortSignal): Promise<void> {
    await this.request("/v1/makers", value, signal);
  }
  async send(value: Envelope, signal?: AbortSignal): Promise<void> {
    await this.request("/v1/messages", value, signal);
  }
  async poll(value: MailboxAuth, signal?: AbortSignal): Promise<unknown[]> {
    const result = await this.request("/v1/mailbox", value, signal);
    if (!Array.isArray(result) || result.length > 32)
      throw new ZuttiError("invalid_message", "Invalid mailbox response");
    return result;
  }
}

import { createServer } from "node:http";
import { ZuttiError } from "@zutti/schema";
import type { RfqDomain } from "@zutti/schema";
import { RelayService } from "./features/relay/relay.service.js";
import { relay_route } from "./features/relay/relay.route.js";
import { send_json } from "./shared/http.js";

export function create_relay_server(domain: RfqDomain) {
  const service = new RelayService(domain);
  const buckets = new Map<string, { requests: number; until: number }>();
  const server = createServer(
    { requestTimeout: 5_000, headersTimeout: 5_000, maxHeaderSize: 8_192 },
    (request, response) => {
      const now = Date.now();
      for (const [address, bucket] of buckets) if (bucket.until <= now) buckets.delete(address);
      const address = request.socket.remoteAddress ?? "unknown";
      const bucket = buckets.get(address) ?? { requests: 0, until: now + 10_000 };
      bucket.requests += 1;
      if (bucket.requests > 1000 || (!buckets.has(address) && buckets.size >= 10_000)) {
        send_json(response, 429, { error: "rate_limited" });
        return;
      }
      buckets.set(address, bucket);
      void relay_route(service, request, response).catch((error: unknown) => {
        if (!response.headersSent)
          send_json(
            response,
            error instanceof ZuttiError && error.code === "service_unavailable" ? 503 : 400,
            { error: error instanceof ZuttiError ? error.code : "invalid_message" },
          );
        else response.destroy();
      });
    },
  );
  server.maxConnections = 100;
  return server;
}

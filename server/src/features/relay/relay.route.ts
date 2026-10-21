import type { IncomingMessage, ServerResponse } from "node:http";
import { ZuttiError } from "@zutti/schema";
import { read_json, send_json } from "../../shared/http.js";
import type { RelayService } from "./relay.service.js";

export async function relay_route(
  service: RelayService,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const path = request.url;
  if (request.method === "GET" && path === "/health") {
    send_json(response, 200, { service: "rfq", status: "ok", settlement: "unavailable" });
    return;
  }
  if (request.method === "GET" && path === "/v1/makers") {
    send_json(response, 200, service.makers());
    return;
  }
  if (
    request.method !== "POST" ||
    !["/v1/makers", "/v1/messages", "/v1/mailbox"].includes(path ?? "")
  ) {
    send_json(response, 404, { error: "not_found" });
    return;
  }
  const value = await read_json(request);
  if (path === "/v1/makers") {
    await service.announce(value);
    send_json(response, 200, { accepted: true });
  } else if (path === "/v1/messages") {
    await service.send(value);
    send_json(response, 200, { accepted: true });
  } else if (path === "/v1/mailbox") send_json(response, 200, await service.poll(value));
  else throw new ZuttiError("invalid_message", "Unknown relay operation");
}

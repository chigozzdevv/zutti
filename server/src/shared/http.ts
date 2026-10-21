import type { IncomingMessage, ServerResponse } from "node:http";
import { ZuttiError } from "@zutti/schema";

export async function read_json(request: IncomingMessage): Promise<unknown> {
  if (request.headers["content-type"]?.split(";")[0]?.trim() !== "application/json")
    throw new ZuttiError("invalid_message", "JSON content type is required");
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of request) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as string);
    size += bytes.length;
    if (size > 131_072) throw new ZuttiError("invalid_message", "Request exceeds its bound");
    chunks.push(bytes);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8")) as unknown;
  } catch {
    throw new ZuttiError("invalid_message", "Invalid JSON message");
  }
}
export function send_json(response: ServerResponse, status: number, value: unknown): void {
  response.writeHead(status, {
    "content-type": "application/json",
    "cache-control": "no-store",
    "x-content-type-options": "nosniff",
  });
  response.end(JSON.stringify(value));
}

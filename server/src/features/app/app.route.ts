import type { IncomingMessage, ServerResponse } from "node:http";
import { ZuttiError } from "@zutti/schema";
import type { AppInformation } from "@zutti/schema";
import { WalletService } from "../wallet/wallet.service.js";
import { read_json, send_json } from "../../shared/http.js";

function session_token(request: IncomingMessage): string | undefined {
  return request.headers.cookie
    ?.split(";")
    .map((part) => part.trim())
    .find((part) => /^zutti_session=[0-9a-f]{64}$/.test(part))
    ?.slice("zutti_session=".length);
}
export async function app_route(
  wallet: WalletService,
  information: AppInformation,
  request: IncomingMessage,
  response: ServerResponse,
  origins: readonly string[],
): Promise<void> {
  const path = new URL(request.url ?? "/", "http://localhost").pathname;
  const token = session_token(request);
  if (
    request.method === "POST" &&
    (!origins.includes(request.headers.origin ?? "") || request.headers["x-zutti-request"] !== "1")
  ) {
    send_json(response, 403, {
      error: "forbidden",
      message: "Open Zutti from its configured app address.",
    });
    return;
  }
  if (request.method === "GET" && path === "/api/app") {
    send_json(response, 200, information);
  } else if (request.method === "GET" && path === "/api/wallet") {
    send_json(response, 200, wallet.summary(token));
  } else if (request.method === "POST" && path === "/api/wallet/connect") {
    await read_json(request);
    if (wallet.summary(token).connected) {
      send_json(response, 200, wallet.summary(token));
      return;
    }
    const connection = await wallet.connect();
    response.setHeader(
      "set-cookie",
      `zutti_session=${connection.token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age=86400`,
    );
    send_json(response, 200, connection.session);
  } else if (request.method === "POST" && path === "/api/wallet/disconnect") {
    await read_json(request);
    await wallet.disconnect(token);
    response.setHeader(
      "set-cookie",
      "zutti_session=; Path=/api; HttpOnly; SameSite=Strict; Max-Age=0",
    );
    send_json(response, 200, { connected: false, label: null });
  } else if (request.method === "POST" && path === "/api/intents") {
    send_json(response, 200, await wallet.quote(token, await read_json(request)));
  } else if (request.method === "GET" && path === "/api/operations") {
    send_json(response, 200, wallet.list(token));
  } else if (request.method === "GET" && path === "/api/maker") {
    send_json(response, 200, await wallet.maker_summary(token));
  } else if (request.method === "POST" && path === "/api/maker/control") {
    send_json(response, 202, await wallet.maker_control(token, await read_json(request)));
  } else if (request.method === "POST" && path === "/api/maker/deposits") {
    send_json(response, 202, await wallet.maker_deposit(token, await read_json(request)));
  } else {
    const accept = /^\/api\/intents\/([0-9a-f]{64})\/accept$/.exec(path);
    const operation = /^\/api\/operations\/([0-9a-f]{64})$/.exec(path);
    const withdrawal = /^\/api\/operations\/([0-9a-f]{64})\/withdraw$/.exec(path);
    if (request.method === "POST" && accept?.[1])
      send_json(response, 202, wallet.accept(token, accept[1], await read_json(request)));
    else if (request.method === "GET" && operation?.[1])
      send_json(response, 200, wallet.get(token, operation[1]));
    else if (request.method === "POST" && withdrawal?.[1])
      send_json(response, 202, wallet.withdraw(token, withdrawal[1], await read_json(request)));
    else throw new ZuttiError("invalid_input", "This page is unavailable.");
  }
}

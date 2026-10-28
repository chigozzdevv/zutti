import type { IncomingMessage, ServerResponse } from "node:http";
import { ZuttiError } from "@zutti/schema";
import type { AppInformation } from "@zutti/schema";
import { WalletService } from "../wallet/wallet.service.js";
import { read_json, send_json } from "../../shared/http.js";
import type { DepositService } from "../wallet/deposit.service.js";
import type { BrowserService } from "../wallet/browser.service.js";
import type { PoolService } from "../wallet/pool.service.js";

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
  deposits?: DepositService,
  browser?: BrowserService,
  pool?: PoolService,
): Promise<void> {
  const path = new URL(request.url ?? "/", "http://localhost").pathname;
  const token = session_token(request);
  const browser_token = request.headers.cookie
    ?.split(";")
    .map((value) => value.trim())
    .find((value) => /^zutti_browser=[0-9a-f]{64}$/.test(value))
    ?.slice("zutti_browser=".length);
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
  const browser_deposit = /^\/api\/browser\/deposits\/([0-9a-f]{64})$/.exec(path);
  const browser_response = /^\/api\/browser\/responses\/([0-9a-f]{64})$/.exec(path);
  if (pool && request.method === "GET" && path === "/api/pool/events") {
    send_json(response, 200, await pool.snapshot());
  } else if (browser && request.method === "POST" && path === "/api/browser/connect") {
    const session = await browser.connect(await read_json(request), browser_token);
    response.setHeader(
      "set-cookie",
      `zutti_browser=${session.id}; Path=/api/browser; HttpOnly; SameSite=Strict; Max-Age=86400${request.headers.origin?.startsWith("https:") ? "; Secure" : ""}`,
    );
    send_json(response, 200, { registered: true });
  } else if (browser && request.method === "GET" && path === "/api/browser/request") {
    send_json(response, 200, await browser.pending(browser_token));
  } else if (browser && request.method === "POST" && browser_response?.[1]) {
    await browser.complete(browser_token, browser_response[1], await read_json(request));
    send_json(response, 200, { saved: true });
  } else if (deposits && request.method === "POST" && path === "/api/browser/deposits") {
    send_json(response, 202, await deposits.prepare(await read_json(request)));
  } else if (deposits && request.method === "GET" && browser_deposit?.[1]) {
    send_json(response, 200, await deposits.get(browser_deposit[1]));
  } else if (request.method === "GET" && path === "/api/app") {
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

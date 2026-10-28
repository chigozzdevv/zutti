import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { ZuttiError } from "@zutti/schema";
import type { AppInformation } from "@zutti/schema";
import { app_route } from "./features/app/app.route.js";
import { WalletService } from "./features/wallet/wallet.service.js";
import type { ConnectWallet } from "./features/wallet/wallet.service.js";
import { send_json } from "./shared/http.js";
import { DepositService } from "./features/wallet/deposit.service.js";

export interface AppServerOptions {
  information: AppInformation;
  origins: readonly string[];
  connect_wallet?: ConnectWallet;
  assets?: string;
  deposit_directory?: string;
}
export function create_app_server(options: AppServerOptions) {
  const wallet = new WalletService(options.information, options.connect_wallet);
  const deposits = options.deposit_directory
    ? new DepositService(options.deposit_directory)
    : undefined;
  const allowed_hosts = new Set(options.origins.map((origin) => new URL(origin).host));
  const buckets = new Map<string, { count: number; until: number }>();
  const server = createServer(
    { requestTimeout: 10_000, headersTimeout: 5_000, maxHeaderSize: 8_192 },
    (request, response) => {
      response.setHeader("x-content-type-options", "nosniff");
      response.setHeader("referrer-policy", "no-referrer");
      response.setHeader("cross-origin-resource-policy", "same-origin");
      response.setHeader("x-frame-options", "DENY");
      if (!allowed_hosts.has(request.headers.host ?? "")) {
        send_json(response, 403, { error: "forbidden", message: "Unknown app address." });
        return;
      }
      const now = Date.now();
      for (const [key, bucket] of buckets) if (bucket.until <= now) buckets.delete(key);
      const key = request.socket.remoteAddress ?? "unknown";
      const bucket = buckets.get(key) ?? { count: 0, until: now + 10_000 };
      bucket.count += 1;
      buckets.set(key, bucket);
      if (bucket.count > 200 || buckets.size > 100) {
        send_json(response, 429, { error: "rate_limited", message: "Please wait a moment." });
        return;
      }
      void (async () => {
        const path = new URL(request.url ?? "/", "http://localhost").pathname;
        if (path.startsWith("/api/")) {
          await app_route(
            wallet,
            options.information,
            request,
            response,
            options.origins,
            deposits,
          );
          return;
        }
        if (request.method !== "GET" || !options.assets) {
          response.writeHead(404);
          response.end();
          return;
        }
        const asset = /^\/assets\/[a-zA-Z0-9_.-]+$/.test(path)
          ? path
          : ["/icon.svg", "/wordmark.svg", "/fonts-license.txt"].includes(path)
            ? path
            : ["/", "/activity", "/wallet", "/maker"].includes(path)
              ? "/index.html"
              : null;
        if (!asset) {
          response.writeHead(404);
          response.end();
          return;
        }
        const data = await readFile(join(options.assets, asset));
        const extension = asset.split(".").pop();
        const type = (
          {
            html: "text/html; charset=utf-8",
            js: "text/javascript",
            css: "text/css",
            svg: "image/svg+xml",
            woff2: "font/woff2",
            woff: "font/woff",
            txt: "text/plain; charset=utf-8",
          } as Record<string, string>
        )[extension ?? ""];
        response.writeHead(200, {
          "content-type": type ?? "application/octet-stream",
          "cache-control": asset.startsWith("/assets/")
            ? "public, max-age=31536000, immutable"
            : "no-store",
          "content-security-policy":
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        });
        response.end(data);
      })().catch((error: unknown) => {
        if (response.headersSent) {
          response.destroy();
          return;
        }
        const known = error instanceof ZuttiError;
        send_json(
          response,
          known && error.code === "wallet_unavailable" ? 409 : known ? 400 : 503,
          {
            error: known ? error.code : "service_unavailable",
            message: known ? error.message : "Zutti is temporarily unavailable. Please try again.",
          },
        );
      });
    },
  );
  server.maxConnections = 100;
  server.on("close", () => {
    void wallet.close();
  });
  return server;
}

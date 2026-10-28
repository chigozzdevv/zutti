import { createServer } from "node:http";
import { readFile } from "node:fs/promises";

const files = new Map([
  ["/snap.manifest.json", ["snap.manifest.json", "application/json"]],
  ["/bundle.js", ["bundle.js", "application/javascript"]],
  ["/files/wallet.wasm", ["files/wallet.wasm", "application/wasm"]],
]);

createServer(async (request, response) => {
  response.setHeader("access-control-allow-origin", "*");
  response.setHeader("cache-control", "no-store");
  response.setHeader("x-content-type-options", "nosniff");
  if (request.method === "OPTIONS") {
    response.writeHead(204).end();
    return;
  }
  const file = files.get(request.url);
  if (request.method !== "GET" || !file) {
    response.writeHead(404).end();
    return;
  }
  try {
    const bytes = await readFile(new URL(`./dist/${file[0]}`, import.meta.url));
    response.writeHead(200, { "content-type": file[1], "content-length": bytes.length });
    response.end(bytes);
  } catch {
    response.writeHead(503).end("Build the Testnet wallet first.");
  }
}).listen(8080, "127.0.0.1", () => {
  console.log("Zutti Testnet Snap: local:http://localhost:8080");
});

import { isAbsolute, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { homedir } from "node:os";
import { readFile, lstat, unlink, chmod } from "node:fs/promises";
import { createConnection } from "node:net";
import { load_app_information } from "./features/app/app.service.js";
import type { ConnectWallet } from "./features/wallet/wallet.service.js";
import { create_app_server } from "./ui-app.js";

const repository = resolve(import.meta.dirname, "../..");
const module_path = process.env.ZUTTI_WALLET_MODULE;
let connect_wallet: ConnectWallet | undefined;
if (module_path) {
  if (!isAbsolute(module_path))
    throw new Error("ZUTTI_WALLET_MODULE must be an absolute local path");
  const module = (await import(pathToFileURL(module_path).href)) as {
    connect_wallet?: ConnectWallet;
  };
  if (typeof module.connect_wallet !== "function")
    throw new Error("Wallet module must export connect_wallet");
  connect_wallet = module.connect_wallet;
}
const port = Number(process.env.ZUTTI_APP_PORT ?? "3002");
if (!Number.isSafeInteger(port) || port < 1 || port > 65535)
  throw new Error("Invalid ZUTTI_APP_PORT");
const origins = [
  `http://127.0.0.1:${port}`,
  `http://localhost:${port}`,
  "http://127.0.0.1:5173",
  "http://localhost:5173",
];
const public_origin = process.env.ZUTTI_APP_ORIGIN;
if (public_origin) {
  const url = new URL(public_origin);
  if (url.protocol !== "https:" || url.origin !== public_origin)
    throw new Error("ZUTTI_APP_ORIGIN must be an HTTPS origin");
  origins.push(public_origin);
}
const deployment = JSON.parse(
  await readFile(resolve(repository, "deploy/ethereum/optimized/sepolia.json"), "utf8"),
) as {
  deployment_transactions: { pool: { height: number } };
};
const server = create_app_server({
  information: await load_app_information(repository, !!connect_wallet),
  origins,
  assets: resolve(repository, "client/dist"),
  pool_deployment_height: deployment.deployment_transactions.pool.height,
  deposit_directory:
    process.env.ZUTTI_DEPOSIT_DIRECTORY ??
    resolve(homedir(), "Library/Application Support/Zutti/testnet/browser-deposits"),
  browser_directory:
    process.env.ZUTTI_BROWSER_DIRECTORY ??
    resolve(homedir(), "Library/Application Support/Zutti/testnet/browser-wallets"),
  ...(connect_wallet ? { connect_wallet } : {}),
});
const socket = process.env.ZUTTI_APP_SOCKET;
if (socket) {
  if (!isAbsolute(socket) || !socket.endsWith("/zutti/app.sock"))
    throw new Error("Invalid Zutti socket path");
  try {
    const stat = await lstat(socket);
    if (!stat.isSocket() || stat.isSymbolicLink()) throw new Error("Unsafe app socket");
    const active = await new Promise<boolean>((resolve) => {
      const probe = createConnection(socket);
      probe.setTimeout(1000);
      probe.once("connect", () => {
        probe.destroy();
        resolve(true);
      });
      probe.once("error", () => resolve(false));
      probe.once("timeout", () => {
        probe.destroy();
        resolve(true);
      });
    });
    if (active) throw new Error("Zutti socket is already active");
    await unlink(socket);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  server.listen(socket, () => {
    void chmod(socket, 0o660);
    console.log(`Zutti app API: ${public_origin}`);
  });
} else {
  server.listen(port, "127.0.0.1", () => console.log(`Zutti app API: http://127.0.0.1:${port}`));
}
for (const signal of ["SIGTERM", "SIGINT"] as const)
  process.on(signal, () => {
    server.close();
    server.closeIdleConnections();
  });

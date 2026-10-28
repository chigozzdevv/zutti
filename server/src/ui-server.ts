import { isAbsolute, resolve } from "node:path";
import { pathToFileURL } from "node:url";
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
const server = create_app_server({
  information: await load_app_information(repository, !!connect_wallet),
  origins,
  assets: resolve(repository, "client/dist"),
  ...(connect_wallet ? { connect_wallet } : {}),
});
server.listen(port, "127.0.0.1", () => console.log(`Zutti app API: http://127.0.0.1:${port}`));
for (const signal of ["SIGTERM", "SIGINT"] as const)
  process.on(signal, () => {
    server.close();
    server.closeIdleConnections();
  });

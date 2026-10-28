import { initSync, BrowserWallet } from "../generated/wallet.js";
import { create_handler, type Wallet } from "./handler.ts";

let ready: Promise<unknown> | undefined;

async function initialize(): Promise<unknown> {
  const file = await snap.request({
    method: "snap_getFile",
    params: { path: "./files/wallet.wasm", encoding: "hex" },
  });
  if (typeof file !== "string" || !/^0x(?:[0-9a-fA-F]{2})+$/.test(file)) {
    throw new Error("Invalid packaged wallet module");
  }
  const bytes = Uint8Array.from(file.slice(2).match(/../g)!, (pair) => parseInt(pair, 16));
  return initSync({ module: bytes });
}

export const onRpcRequest = create_handler(
  {
    async wallet(entropy) {
      ready ??= initialize().catch((error: unknown) => {
        ready = undefined;
        throw error;
      });
      await ready;
      return new BrowserWallet(entropy) as Wallet;
    },
  },
  snap,
);

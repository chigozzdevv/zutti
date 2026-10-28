import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFile } from "node:fs/promises";
import { webcrypto } from "node:crypto";
import { test } from "node:test";

const require = createRequire(import.meta.url);
await import(require.resolve("ses", { paths: [require.resolve("@metamask/snaps-utils")] }));
lockdown();

test("the built bundle runs in a restricted compartment with generated Testnet entropy", async () => {
  const source = await readFile(new URL("../dist/bundle.js", import.meta.url), "utf8");
  const wasm = await readFile(new URL("../dist/files/wallet.wasm", import.meta.url));
  const calls = [];
  const exports = {};
  const compartment = new Compartment({
    exports,
    WebAssembly,
    TextEncoder,
    TextDecoder,
    crypto: webcrypto,
    snap: {
      request: async (request) => {
        calls.push(request.method);
        if (request.method === "snap_dialog") return true;
        if (request.method === "snap_getEntropy") return `0x${"42".repeat(32)}`;
        if (request.method === "snap_getFile") {
          assert.deepEqual(request.params, { path: "./files/wallet.wasm", encoding: "hex" });
          return `0x${wasm.toString("hex")}`;
        }
        throw new Error("Unexpected Snap API");
      },
    },
  });
  compartment.evaluate(source);
  const request = {
    origin: "http://127.0.0.1:3002",
    originMetadata: null,
    request: { jsonrpc: "2.0", id: 1, method: "zutti_getAccount" },
  };
  const account = await exports.onRpcRequest(request);
  assert.equal(account.network, "testnet");
  assert.ok(account.zcash_address.startsWith("utest1"));
  assert.deepEqual(calls, ["snap_dialog", "snap_getEntropy", "snap_getFile"]);
  const deposit = await exports.onRpcRequest({
    ...request,
    request: {
      jsonrpc: "2.0",
      id: 2,
      method: "zutti_prepareDeposit",
      params: { amount: "1", owner: `0x${"22".repeat(20)}` },
    },
  });
  assert.equal(deposit.units, "1000000");
  assert.equal(deposit.owner, `0x${"22".repeat(20)}`);
  assert.match(deposit.witness, /^5a55545449535056/);
  assert.ok(!JSON.stringify(deposit).includes("spending_key"));
  await assert.rejects(exports.onRpcRequest({ ...request, origin: "https://unapproved.example" }));
});

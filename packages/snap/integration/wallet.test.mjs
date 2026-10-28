import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { initSync, BrowserWallet, verify_source } from "../generated/wallet.js";

initSync({ module: await readFile(new URL("../generated/wallet_bg.wasm", import.meta.url)) });

test("the packaged WASM verifies the genuine deployed source receipt", async () => {
  const receipt = await readFile(
    new URL("../../../deploy/ethereum/optimized/source-receipt.bin", import.meta.url),
  );
  assert.equal(verify_source(receipt), 4_550_367);
  const changed = Uint8Array.from(receipt);
  changed[changed.length - 1] ^= 1;
  assert.throws(() => verify_source(changed));
  const other_policy = await readFile(
    new URL("../../../deploy/source/testnet/source-receipt.bin", import.meta.url),
  );
  assert.throws(() => verify_source(other_policy), /policy mismatch/);
  const trailing = new Uint8Array(receipt.length + 1);
  trailing.set(receipt);
  assert.throws(() => verify_source(trailing));
});

test("the real WASM wallet exposes only public receiving data and cannot sign a boolean", () => {
  const wallet = new BrowserWallet(new Uint8Array(32).fill(42));
  try {
    const account = JSON.parse(wallet.account());
    assert.equal(account.network, "testnet");
    assert.equal(account.chain_id, 11155111);
    assert.ok(account.zcash_address.startsWith("utest1"));
    assert.deepEqual(Object.keys(account).sort(), [
      "chain_id",
      "domain",
      "network",
      "usdc_receiver",
      "version",
      "zcash_address",
    ]);
    assert.throws(() => wallet.authorize('{"authorized":true}', 1_793_200_000));
  } finally {
    wallet.free();
  }
});

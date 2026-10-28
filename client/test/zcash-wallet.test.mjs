import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { ZcashWallet, zutti_snap } from "../src/shared/zcash-wallet.ts";

const domain = "12".repeat(32);
const receiving = {
  version: 1,
  network: "testnet",
  chain_id: 11155111,
  domain,
  zcash_address: `utest1${"q".repeat(100)}`,
  usdc_receiver: {
    owner: `02${"34".repeat(32)}`,
    viewing: "45".repeat(32),
    nullifier_key_commitment: "56".repeat(32),
  },
};

class Provider extends EventEmitter {
  calls = [];
  version = "MetaMask/v13.48.0-flask.0";
  installed = { id: zutti_snap, version: "0.1.0", enabled: true, blocked: false };
  receiving = structuredClone(receiving);
  approve;
  request = async ({ method, params }) => {
    this.calls.push({ method, params });
    if (method === "web3_clientVersion") return this.version;
    if (method === "wallet_getSnaps") return { [zutti_snap]: this.installed };
    if (method === "wallet_requestSnaps") {
      if (this.approve) await this.approve;
      return { [zutti_snap]: this.installed };
    }
    if (method === "wallet_invokeSnap") return this.receiving;
    throw new Error("Unexpected wallet API");
  };
}

test("refresh restores a confirmed public account without installing or signing again", async () => {
  const provider = new Provider();
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  const first = new ZcashWallet(() => provider, storage);
  await first.connect(domain);
  first.stop();
  const offset = provider.calls.length;
  const restored = new ZcashWallet(() => provider, storage);
  await restored.restore(domain);
  assert.equal(restored.snapshot().account.zcash_address, receiving.zcash_address);
  assert.deepEqual(
    provider.calls.slice(offset).map((call) => call.method),
    ["wallet_getSnaps", "wallet_invokeSnap"],
  );
  assert.equal(provider.calls.at(-1).params.request.method, "zutti_restoreAccount");
  restored.disconnect();
  assert.equal(values.size, 0);
});

test("Snap connection is explicit and requests no transfer or signing method", async () => {
  const provider = new Provider();
  const wallet = new ZcashWallet(() => provider);
  assert.equal(provider.calls.length, 0);
  await wallet.connect(domain);
  assert.equal(wallet.snapshot().account.zcash_address, receiving.zcash_address);
  assert.deepEqual(provider.calls, [
    { method: "web3_clientVersion", params: undefined },
    { method: "wallet_requestSnaps", params: { [zutti_snap]: { version: "0.1.0" } } },
    {
      method: "wallet_invokeSnap",
      params: { snapId: zutti_snap, request: { method: "zutti_getAccount" } },
    },
  ]);
  assert.ok(Object.isFrozen(wallet.snapshot().account.usdc_receiver));
  provider.emit("disconnect");
  assert.equal(wallet.snapshot().account, null);
  assert.equal(provider.listenerCount("disconnect"), 0);
});

test("regular MetaMask and invalid deployments cannot trigger installation", async () => {
  const provider = new Provider();
  const wallet = new ZcashWallet(() => provider);
  await wallet.connect("invalid");
  assert.equal(provider.calls.length, 0);
  provider.version = "MetaMask/v13.48.0";
  await wallet.connect(domain);
  assert.equal(wallet.snapshot().account, null);
  assert.match(wallet.snapshot().error, /requires MetaMask Flask/);
  assert.deepEqual(
    provider.calls.map(({ method }) => method),
    ["web3_clientVersion"],
  );
});

test("blocked, disabled or unknown Snap versions never become connected", async () => {
  for (const change of [
    { blocked: true },
    { enabled: false },
    { version: "0.2.0" },
    { id: "npm:other" },
  ]) {
    const provider = new Provider();
    Object.assign(provider.installed, change);
    const wallet = new ZcashWallet(() => provider);
    await wallet.connect(domain);
    assert.equal(wallet.snapshot().account, null);
    assert.equal(provider.calls.length, 2);
  }
});

test("changed networks, deployments, extra keys and malformed receivers fail closed", async () => {
  for (const change of [
    { network: "mainnet" },
    { domain: "ab".repeat(32) },
    { seed: "do not accept" },
    { zcash_address: "u1mainnet" },
    { chain_id: 1 },
    { usdc_receiver: {} },
  ]) {
    const provider = new Provider();
    Object.assign(provider.receiving, change);
    const wallet = new ZcashWallet(() => provider);
    await wallet.connect(domain);
    assert.equal(wallet.snapshot().account, null);
    assert.match(wallet.snapshot().error, /invalid or mismatched/);
  }
});

test("a disconnect during installation prevents late account access", async () => {
  const provider = new Provider();
  let release;
  provider.approve = new Promise((resolve) => {
    release = resolve;
  });
  const wallet = new ZcashWallet(() => provider);
  const pending = wallet.connect(domain);
  await new Promise((resolve) => setImmediate(resolve));
  await wallet.connect(domain);
  assert.equal(provider.calls.length, 2);
  wallet.disconnect();
  release();
  await pending;
  assert.equal(wallet.snapshot().account, null);
  assert.equal(wallet.snapshot().pending, false);
  assert.equal(provider.calls.length, 2);
});

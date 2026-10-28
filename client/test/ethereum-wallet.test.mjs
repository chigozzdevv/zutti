import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { EthereumWallet, sepolia_chain } from "../src/shared/ethereum-wallet.ts";

const owner = `0x${"12".repeat(20)}`;
class Provider extends EventEmitter {
  isMetaMask = true;
  chain_id = sepolia_chain;
  accounts = [owner];
  calls = [];
  request = async ({ method, params }) => {
    this.calls.push({ method, params });
    if (method === "eth_requestAccounts" || method === "eth_accounts") return this.accounts;
    if (method === "eth_chainId") return this.chain_id;
    if (method === "wallet_switchEthereumChain") {
      this.chain_id = params[0].chainId;
      this.emit("chainChanged", this.chain_id);
      return null;
    }
    throw new Error(`Unexpected wallet request: ${method}`);
  };
}
class Browser extends EventTarget {
  records = new Map();
  sessionStorage = {
    getItem: (key) => this.records.get(key) ?? null,
    setItem: (key, value) => this.records.set(key, value),
    removeItem: (key) => this.records.delete(key),
  };
  announce(provider, rdns = "io.metamask") {
    this.dispatchEvent(
      new CustomEvent("eip6963:announceProvider", { detail: { info: { rdns }, provider } }),
    );
  }
}
function fixture() {
  const browser = new Browser();
  const provider = new Provider();
  browser.addEventListener("eip6963:requestProvider", () => browser.announce(provider));
  const wallet = new EthereumWallet(browser);
  return { browser, provider, wallet };
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

test("MetaMask Flask is discovered through its own EIP-6963 identity", async () => {
  const browser = new Browser();
  const provider = new Provider();
  browser.addEventListener("eip6963:requestProvider", () =>
    browser.announce(provider, "io.metamask.flask"),
  );
  const wallet = new EthereumWallet(browser);
  wallet.start();
  try {
    assert.equal(wallet.snapshot().available, true);
    assert.equal(wallet.extension(), provider);
    assert.deepEqual(provider.calls, []);
  } finally {
    wallet.stop();
  }
});

test("MetaMask discovery ignores a Zcash provider that shadows window.ethereum", async () => {
  const { browser, wallet, provider } = fixture();
  const unrelated = new Provider();
  unrelated.isNozyWallet = true;
  browser.ethereum = unrelated;
  wallet.start();
  try {
    assert.equal(wallet.snapshot().available, true);
    assert.equal(provider.calls.length, 0);
    await wallet.connect();
    assert.equal(wallet.snapshot().account, owner);
    assert.equal(unrelated.calls.length, 0);
    assert.deepEqual(
      provider.calls.map((call) => call.method),
      ["eth_requestAccounts", "eth_chainId", "eth_accounts"],
    );
  } finally {
    wallet.stop();
  }
});

test("connecting twice shares one wallet permission request", async () => {
  const { wallet, provider } = fixture();
  const request = provider.request;
  let release;
  const approved = new Promise((resolve) => {
    release = resolve;
  });
  provider.request = async (input) => {
    if (input.method === "eth_requestAccounts") await approved;
    return request(input);
  };
  wallet.start();
  try {
    const first = wallet.connect();
    await wallet.connect();
    assert.equal(wallet.snapshot().pending, true);
    release();
    await first;
    assert.equal(provider.calls.filter((call) => call.method === "eth_requestAccounts").length, 1);
    assert.equal(wallet.snapshot().pending, false);
  } finally {
    release();
    wallet.stop();
  }
});

test("restoring an approved session never opens a permission or signature prompt", async () => {
  const { wallet, provider, browser } = fixture();
  browser.records.set("zutti:metamask", "1");
  wallet.start();
  try {
    await tick();
    assert.equal(wallet.snapshot().account, owner);
    assert.deepEqual(
      provider.calls.map((call) => call.method),
      ["eth_accounts", "eth_chainId"],
    );
    wallet.disconnect();
    assert.equal(wallet.snapshot().account, null);
    assert.equal(browser.records.size, 0);
  } finally {
    wallet.stop();
  }
});

test("account and chain changes update the actual connected wallet", async () => {
  const { wallet, provider } = fixture();
  wallet.start();
  try {
    await wallet.connect();
    const other = `0x${"34".repeat(20)}`;
    provider.emit("accountsChanged", [other]);
    assert.equal(wallet.snapshot().account, other);
    provider.emit("chainChanged", "0x1");
    assert.equal(wallet.snapshot().chain_id, "0x1");
    provider.emit("accountsChanged", []);
    assert.equal(wallet.snapshot().account, null);
    provider.emit("accountsChanged", [owner]);
    assert.equal(wallet.snapshot().account, null);
  } finally {
    wallet.stop();
  }
});

test("a rejected permission request never creates a connected wallet", async () => {
  const { wallet, provider } = fixture();
  provider.request = async () => {
    throw Object.assign(new Error("Rejected"), { code: 4001 });
  };
  wallet.start();
  try {
    await wallet.connect();
    assert.equal(wallet.snapshot().account, null);
    assert.equal(wallet.snapshot().pending, false);
    assert.match(wallet.snapshot().error, /declined/);
  } finally {
    wallet.stop();
  }
});

test("disconnecting during approval prevents a late response from reconnecting", async () => {
  const { wallet, provider } = fixture();
  const request = provider.request;
  let release;
  const approved = new Promise((resolve) => {
    release = resolve;
  });
  provider.request = async (input) => {
    if (input.method === "eth_requestAccounts") await approved;
    return request(input);
  };
  wallet.start();
  try {
    const waiting = wallet.connect();
    provider.emit("disconnect");
    release();
    await waiting;
    assert.equal(wallet.snapshot().account, null);
  } finally {
    release();
    wallet.stop();
  }
});

test("Sepolia switching requires an explicit action and verifies the resulting chain", async () => {
  const { wallet, provider } = fixture();
  provider.chain_id = "0x1";
  wallet.start();
  try {
    await wallet.connect();
    assert.equal(wallet.snapshot().chain_id, "0x1");
    assert.equal(
      provider.calls.some((call) => call.method === "wallet_switchEthereumChain"),
      false,
    );
    await wallet.switch_network();
    assert.equal(wallet.snapshot().chain_id, sepolia_chain);
    assert.equal(
      provider.calls.filter((call) => call.method === "wallet_switchEthereumChain").length,
      1,
    );
    assert.equal(
      provider.calls.some((call) => /send|sign|approve/i.test(call.method)),
      false,
    );
  } finally {
    wallet.stop();
  }
});

test("malformed wallet responses never become connected accounts", async () => {
  const { wallet, provider } = fixture();
  provider.accounts = ["utest1not-an-ethereum-account"];
  wallet.start();
  try {
    await wallet.connect();
    assert.equal(wallet.snapshot().account, null);
    assert.ok(wallet.snapshot().error);
  } finally {
    wallet.stop();
  }
});

test("unmounting and remounting removes duplicate provider listeners", async () => {
  const { wallet, provider } = fixture();
  wallet.start();
  wallet.stop();
  wallet.start();
  try {
    assert.equal(provider.listenerCount("accountsChanged"), 1);
    await wallet.connect();
    assert.equal(wallet.snapshot().account, owner);
  } finally {
    wallet.stop();
    assert.equal(provider.listenerCount("accountsChanged"), 0);
  }
});

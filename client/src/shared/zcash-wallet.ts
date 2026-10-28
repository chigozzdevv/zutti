import type { EthereumProvider } from "./ethereum-wallet.js";

export const zutti_snap = "local:http://localhost:8080";

export interface ZcashAccount {
  version: 1;
  network: "testnet";
  chain_id: 11155111;
  domain: string;
  zcash_address: string;
  usdc_receiver: {
    owner: string;
    viewing: string;
    nullifier_key_commitment: string;
  };
}

export interface ZcashState {
  account: ZcashAccount | null;
  pending: boolean;
  error: string | null;
}

function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function keys(value: Record<string, unknown>, expected: string[]): boolean {
  return Object.keys(value).sort().join(",") === expected.sort().join(",");
}

function encoded(value: unknown, bytes: number): value is string {
  return (
    typeof value === "string" &&
    value.length === bytes * 2 &&
    /^[0-9a-f]+$/.test(value) &&
    /[1-9a-f]/.test(value)
  );
}

function account(value: unknown, domain: string): ZcashAccount {
  if (
    !object(value) ||
    !keys(value, ["version", "network", "chain_id", "domain", "zcash_address", "usdc_receiver"]) ||
    value.version !== 1 ||
    value.network !== "testnet" ||
    value.chain_id !== 11155111 ||
    !encoded(domain, 32) ||
    value.domain !== domain ||
    typeof value.zcash_address !== "string" ||
    !/^utest1[023456789acdefghjklmnpqrstuvwxyz]{80,300}$/.test(value.zcash_address) ||
    !object(value.usdc_receiver) ||
    !keys(value.usdc_receiver, ["owner", "viewing", "nullifier_key_commitment"]) ||
    !encoded(value.usdc_receiver.owner, 33) ||
    !/^(02|03)/.test(value.usdc_receiver.owner) ||
    !encoded(value.usdc_receiver.viewing, 32) ||
    !encoded(value.usdc_receiver.nullifier_key_commitment, 32)
  ) {
    throw new Error("Zutti returned an invalid or mismatched Testnet account.");
  }
  return Object.freeze({
    version: 1,
    network: "testnet",
    chain_id: 11155111,
    domain,
    zcash_address: value.zcash_address,
    usdc_receiver: Object.freeze({
      owner: value.usdc_receiver.owner,
      viewing: value.usdc_receiver.viewing,
      nullifier_key_commitment: value.usdc_receiver.nullifier_key_commitment,
    }),
  });
}

function failure(error: unknown): string {
  if (object(error) && error.code === 4001) return "Zcash wallet connection was declined.";
  if (object(error) && error.code === -32002) return "Finish the pending request in Flask.";
  if (error instanceof Error && error.message.startsWith("Zutti")) return error.message;
  if (object(error) && typeof error.message === "string" && error.message.length > 0) {
    return `Flask: ${error.message.slice(0, 300)}`;
  }
  return "Couldn’t connect the Zcash wallet. Open Flask and try again.";
}

export class ZcashWallet {
  private readonly provider: () => EthereumProvider | null;
  private current: ZcashState = { account: null, pending: false, error: null };
  private readonly listeners = new Set<() => void>();
  private pending = false;
  private revision = 0;
  private active_provider: EthereumProvider | null = null;

  constructor(provider: () => EthereumProvider | null) {
    this.provider = provider;
  }
  snapshot = (): ZcashState => this.current;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(value: Partial<ZcashState>): void {
    this.current = Object.freeze({ ...this.current, ...value });
    for (const listener of this.listeners) listener();
  }
  private detach(): void {
    this.active_provider?.removeListener?.("disconnect", this.disconnect);
    this.active_provider = null;
  }
  disconnect = (): void => {
    this.revision++;
    this.detach();
    this.update({ account: null, error: null });
  };
  stop(): void {
    this.disconnect();
  }

  async connect(domain: string): Promise<void> {
    if (this.pending) return;
    const provider = this.provider();
    if (!provider) {
      this.update({ error: "Zutti needs MetaMask Flask in this browser profile." });
      return;
    }
    if (!encoded(domain, 32)) {
      this.update({ error: "Zutti’s Testnet deployment is not ready." });
      return;
    }
    this.detach();
    this.active_provider = provider;
    provider.on?.("disconnect", this.disconnect);
    this.pending = true;
    const revision = ++this.revision;
    this.update({ account: null, pending: true, error: null });
    try {
      const version = await provider.request({ method: "web3_clientVersion" });
      if (typeof version !== "string" || !version.toLowerCase().includes("flask")) {
        throw new Error("Zutti’s development wallet requires MetaMask Flask.");
      }
      if (revision !== this.revision) return;
      const installed = await provider.request({
        method: "wallet_requestSnaps",
        params: { [zutti_snap]: { version: "0.1.0" } },
      });
      if (revision !== this.revision) return;
      const snap = object(installed) ? installed[zutti_snap] : null;
      if (
        !object(snap) ||
        snap.id !== zutti_snap ||
        snap.version !== "0.1.0" ||
        snap.enabled !== true ||
        snap.blocked === true
      ) {
        throw new Error("Zutti’s Testnet Snap is unavailable or disabled.");
      }
      const value = await provider.request({
        method: "wallet_invokeSnap",
        params: { snapId: zutti_snap, request: { method: "zutti_getAccount" } },
      });
      if (revision !== this.revision) return;
      this.update({ account: account(value, domain), error: null });
    } catch (error) {
      if (revision === this.revision) this.update({ account: null, error: failure(error) });
    } finally {
      this.pending = false;
      this.update({ pending: false });
      if (!this.current.account) this.detach();
    }
  }
}

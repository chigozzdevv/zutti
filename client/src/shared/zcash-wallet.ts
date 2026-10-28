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

export interface PreparedDeposit {
  version: 1;
  chain_id: 11155111;
  domain: string;
  pool: string;
  token: string;
  owner: string;
  amount: string;
  units: string;
  commitment: string;
  witness: string;
}
export interface DepositCall {
  chain_id: 11155111;
  to: string;
  token: string;
  from: string;
  units: string;
  value: "0x0";
  data: string;
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
  private readonly storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> | undefined;
  private domain: string | null = null;

  constructor(
    provider: () => EthereumProvider | null,
    storage?: Pick<Storage, "getItem" | "setItem" | "removeItem">,
  ) {
    this.provider = provider;
    this.storage = storage;
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
    this.remember(null);
    this.revision++;
    this.detach();
    this.update({ account: null, error: null });
  };
  stop(): void {
    this.revision++;
    this.detach();
    this.update({ account: null, error: null });
  }
  private remember(value: ZcashAccount | null): void {
    if (!this.domain) return;
    try {
      if (value) this.storage?.setItem(`zutti:zcash:${this.domain}`, JSON.stringify(value));
      else this.storage?.removeItem(`zutti:zcash:${this.domain}`);
    } catch {
      return;
    }
  }

  async restore(domain: string): Promise<void> {
    if (this.pending || this.current.account || !encoded(domain, 32)) return;
    const provider = this.provider();
    if (!provider || !this.storage) return;
    let cached: ZcashAccount;
    try {
      const value = this.storage.getItem(`zutti:zcash:${domain}`);
      if (!value) return;
      cached = account(JSON.parse(value), domain);
    } catch {
      return;
    }
    this.domain = domain;
    this.pending = true;
    const revision = ++this.revision;
    this.update({ pending: true });
    try {
      const installed = await provider.request({ method: "wallet_getSnaps" });
      const snap = object(installed) ? installed[zutti_snap] : null;
      if (!object(snap) || snap.enabled !== true || snap.blocked === true)
        throw new Error("Reconnect the Zutti Snap.");
      const value = await provider.request({
        method: "wallet_invokeSnap",
        params: {
          snapId: zutti_snap,
          request: { method: "zutti_restoreAccount", params: { account: JSON.stringify(cached) } },
        },
      });
      if (revision !== this.revision || provider !== this.provider()) return;
      this.active_provider = provider;
      provider.on?.("disconnect", this.disconnect);
      this.update({ account: account(value, domain), error: null });
    } catch (error) {
      if (revision === this.revision) this.update({ account: null, error: failure(error) });
    } finally {
      this.pending = false;
      this.update({ pending: false });
    }
  }

  private async invoke(method: string, params: Record<string, unknown>): Promise<unknown> {
    const provider = this.active_provider;
    const revision = this.revision;
    if (!provider || !this.current.account) throw new Error("Connect your Zcash wallet first.");
    const result = await provider.request({
      method: "wallet_invokeSnap",
      params: { snapId: zutti_snap, request: { method, params } },
    });
    if (revision !== this.revision || provider !== this.provider())
      throw new Error("The connected Zcash wallet changed. Reconnect before continuing.");
    return result;
  }

  async prepare_deposit(amount: string, owner: string): Promise<PreparedDeposit> {
    const result = await this.invoke("zutti_prepareDeposit", { amount, owner });
    if (
      !object(result) ||
      result.version !== 1 ||
      result.chain_id !== 11155111 ||
      result.domain !== this.current.account?.domain ||
      result.owner !== owner.toLowerCase() ||
      typeof result.units !== "string" ||
      !/^[1-9][0-9]{0,19}$/.test(result.units) ||
      typeof result.amount !== "string" ||
      typeof result.pool !== "string" ||
      !/^0x[0-9a-f]{40}$/.test(result.pool) ||
      typeof result.token !== "string" ||
      !/^0x[0-9a-f]{40}$/.test(result.token) ||
      !encoded(result.commitment, 32) ||
      typeof result.witness !== "string" ||
      !/^(?:[0-9a-f]{2})+$/.test(result.witness) ||
      result.witness.length > 32768
    )
      throw new Error("The wallet returned an invalid prepared deposit.");
    return result as unknown as PreparedDeposit;
  }

  async execute(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (
      ![
        "zutti_scanSource",
        "zutti_prepareOrder",
        "zutti_proveOrder",
        "zutti_signTransaction",
        "zutti_pool",
      ].includes(method)
    )
      throw new Error("Unsupported browser wallet request.");
    return this.invoke(method, params);
  }

  async verify_deposit(witness: string, receipt: string): Promise<DepositCall> {
    const result = await this.invoke("zutti_verifyDeposit", { witness, receipt });
    if (
      !object(result) ||
      result.chain_id !== 11155111 ||
      result.value !== "0x0" ||
      typeof result.to !== "string" ||
      !/^0x[0-9a-f]{40}$/.test(result.to) ||
      typeof result.token !== "string" ||
      !/^0x[0-9a-f]{40}$/.test(result.token) ||
      typeof result.from !== "string" ||
      !/^0x[0-9a-f]{40}$/.test(result.from) ||
      typeof result.units !== "string" ||
      !/^[1-9][0-9]{0,19}$/.test(result.units) ||
      typeof result.data !== "string" ||
      !/^0x1d0a9b2e(?:[0-9a-f]{2})+$/.test(result.data) ||
      result.data.length > 32768
    )
      throw new Error("The wallet returned an invalid verified deposit.");
    return result as unknown as DepositCall;
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
      this.domain = domain;
      this.remember(this.current.account);
    } catch (error) {
      if (revision === this.revision) this.update({ account: null, error: failure(error) });
    } finally {
      this.pending = false;
      this.update({ pending: false });
      if (!this.current.account) this.detach();
    }
  }
}

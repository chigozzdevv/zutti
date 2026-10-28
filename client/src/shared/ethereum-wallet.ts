export interface EthereumProvider {
  request(input: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: string, listener: (...values: unknown[]) => void): void;
  removeListener?(event: string, listener: (...values: unknown[]) => void): void;
  isMetaMask?: boolean;
  isNozyWallet?: boolean;
  isRabby?: boolean;
  isBraveWallet?: boolean;
  providers?: EthereumProvider[];
}

export interface EthereumState {
  available: boolean;
  account: string | null;
  chain_id: string | null;
  pending: boolean;
  error: string | null;
}

export interface WalletBrowser extends EventTarget {
  ethereum?: EthereumProvider;
  sessionStorage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
}

const selection_key = "zutti:metamask";
export const sepolia_chain = "0xaa36a7";
const address = (value: unknown): value is string =>
  typeof value === "string" && /^0x[0-9a-fA-F]{40}$/.test(value) && !/^0x0{40}$/.test(value);
function accounts(value: unknown): string | null {
  if (!Array.isArray(value) || value.length > 100 || value.some((item) => !address(item)))
    throw new Error("MetaMask returned an invalid account.");
  return value[0] ?? null;
}
function chain(value: unknown): string {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{1,32}$/.test(value))
    throw new Error("MetaMask returned an invalid network.");
  return `0x${BigInt(value).toString(16)}`;
}
function message(error: unknown): string {
  const code = error && typeof error === "object" && "code" in error ? error.code : null;
  if (code === 4001) return "Connection was declined in MetaMask.";
  if (code === -32002) return "Open MetaMask to finish the pending request.";
  return "Couldn’t connect. Open MetaMask and try again.";
}

export class EthereumWallet {
  private readonly browser: WalletBrowser;
  private provider: EthereumProvider | null = null;
  private current: EthereumState = {
    available: false,
    account: null,
    chain_id: null,
    pending: false,
    error: null,
  };
  private readonly listeners = new Set<() => void>();
  private active = false;
  private revision = 0;
  private epoch = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private request_pending = false;

  constructor(browser: WalletBrowser) {
    this.browser = browser;
  }
  snapshot = (): EthereumState => this.current;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(value: Partial<EthereumState>): void {
    this.current = Object.freeze({ ...this.current, ...value });
    for (const listener of this.listeners) listener();
  }
  private selected(): boolean {
    try {
      return this.browser.sessionStorage.getItem(selection_key) === "1";
    } catch {
      return false;
    }
  }
  private remember(selected: boolean): void {
    try {
      if (selected) this.browser.sessionStorage.setItem(selection_key, "1");
      else this.browser.sessionStorage.removeItem(selection_key);
    } catch {
      return;
    }
  }
  private detach(): void {
    this.provider?.removeListener?.("accountsChanged", this.changed_accounts);
    this.provider?.removeListener?.("chainChanged", this.changed_chain);
    this.provider?.removeListener?.("disconnect", this.disconnected);
  }
  private attach(provider: EthereumProvider): void {
    if (this.provider === provider) return;
    if (this.provider) return;
    this.provider = provider;
    provider.on?.("accountsChanged", this.changed_accounts);
    provider.on?.("chainChanged", this.changed_chain);
    provider.on?.("disconnect", this.disconnected);
    this.update({ available: true });
    if (this.selected()) void this.restore();
  }
  private announce = (event: Event): void => {
    const detail: unknown = (event as CustomEvent).detail;
    if (!detail || typeof detail !== "object" || !("info" in detail) || !("provider" in detail))
      return;
    const info = detail.info;
    if (!info || typeof info !== "object" || !("rdns" in info) || info.rdns !== "io.metamask")
      return;
    const provider = detail.provider;
    if (
      provider &&
      typeof provider === "object" &&
      "request" in provider &&
      typeof provider.request === "function"
    )
      this.attach(provider as EthereumProvider);
  };
  private legacy = (): void => {
    const root = this.browser.ethereum;
    const options = root?.providers?.slice(0, 20) ?? (root ? [root] : []);
    const provider = options.find(
      (item) =>
        item.isMetaMask &&
        !item.isNozyWallet &&
        !item.isRabby &&
        !item.isBraveWallet &&
        typeof item.request === "function",
    );
    if (provider) this.attach(provider);
  };
  start(): void {
    if (this.active) return;
    this.active = true;
    this.browser.addEventListener("eip6963:announceProvider", this.announce);
    this.browser.addEventListener("ethereum#initialized", this.legacy);
    this.browser.dispatchEvent(new Event("eip6963:requestProvider"));
    this.timer = setTimeout(this.legacy, 250);
  }
  stop(): void {
    this.active = false;
    this.revision++;
    this.epoch++;
    if (this.timer) clearTimeout(this.timer);
    this.browser.removeEventListener("eip6963:announceProvider", this.announce);
    this.browser.removeEventListener("ethereum#initialized", this.legacy);
    this.detach();
    this.provider = null;
  }
  private changed_accounts = (value: unknown): void => {
    if (!this.current.account && !this.request_pending && !this.selected()) return;
    this.revision++;
    try {
      const account = accounts(value);
      this.update({ account, pending: this.request_pending, error: null });
      this.remember(account !== null);
    } catch {
      this.update({
        account: null,
        chain_id: null,
        error: "MetaMask returned an invalid account.",
      });
      this.remember(false);
    }
  };
  private changed_chain = (value: unknown): void => {
    this.revision++;
    if (!this.current.account) return;
    try {
      this.update({ chain_id: chain(value), error: null });
    } catch {
      this.update({ chain_id: null, error: "Check your network in MetaMask." });
    }
  };
  private disconnected = (): void => {
    this.revision++;
    this.epoch++;
    this.remember(false);
    this.update({ account: null, chain_id: null, pending: this.request_pending, error: null });
  };
  private async restore(): Promise<void> {
    const provider = this.provider;
    const revision = this.revision;
    if (!provider) return;
    try {
      const account = accounts(await provider.request({ method: "eth_accounts" }));
      const chain_id = account ? chain(await provider.request({ method: "eth_chainId" })) : null;
      if (
        !this.active ||
        provider !== this.provider ||
        revision !== this.revision ||
        this.request_pending
      )
        return;
      this.update({ account, chain_id, error: null });
      this.remember(account !== null);
    } catch {
      if (revision === this.revision) this.update({ account: null, chain_id: null });
    }
  }
  async connect(): Promise<void> {
    if (this.request_pending) return;
    this.legacy();
    const provider = this.provider;
    if (!provider) {
      this.update({ error: "Install MetaMask, then reload Zutti." });
      return;
    }
    this.request_pending = true;
    this.revision++;
    const epoch = this.epoch;
    this.update({ pending: true, error: null });
    try {
      const account = accounts(await provider.request({ method: "eth_requestAccounts" }));
      if (!account) throw new Error("No account");
      if (epoch !== this.epoch) return;
      const revision = this.revision;
      const chain_id = chain(await provider.request({ method: "eth_chainId" }));
      const current = accounts(await provider.request({ method: "eth_accounts" }));
      if (!this.active || provider !== this.provider || epoch !== this.epoch) return;
      if (!current) {
        this.update({ account: null, chain_id: null });
        this.remember(false);
        return;
      }
      if (revision !== this.revision) throw new Error("Wallet changed during connection");
      this.update({ account: current, chain_id, error: null });
      this.remember(true);
    } catch (error) {
      if (this.active && provider === this.provider && epoch === this.epoch)
        this.update({ account: null, chain_id: null, error: message(error) });
      this.remember(false);
    } finally {
      this.request_pending = false;
      this.update({ pending: false });
    }
  }
  async switch_network(): Promise<void> {
    const provider = this.provider;
    if (!provider || !this.current.account || this.request_pending) return;
    this.request_pending = true;
    const epoch = this.epoch;
    this.update({ pending: true, error: null });
    try {
      await provider.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: sepolia_chain }],
      });
      const revision = this.revision;
      const chain_id = chain(await provider.request({ method: "eth_chainId" }));
      const account = accounts(await provider.request({ method: "eth_accounts" }));
      if (provider !== this.provider || !this.active || epoch !== this.epoch) return;
      if (revision !== this.revision) throw new Error("Wallet changed during network switch");
      this.update({
        chain_id,
        account,
        error: chain_id === sepolia_chain ? null : "Select Sepolia in MetaMask.",
      });
      this.remember(account !== null);
    } catch (error) {
      if (provider === this.provider && this.active && epoch === this.epoch)
        this.update({ error: message(error) });
    } finally {
      this.request_pending = false;
      this.update({ pending: false });
    }
  }
  disconnect(): void {
    if (this.request_pending) return;
    this.disconnected();
  }
}

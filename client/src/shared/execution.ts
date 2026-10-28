import type { NetworkProfile } from "@zutti/schema";
import type { BrowserRequest } from "./api.js";
import type { EthereumWallet } from "./ethereum-wallet.js";
import type { ZcashWallet } from "./zcash-wallet.js";

interface Response {
  result: unknown;
  error: string | null;
}
interface Retained {
  phase: "executing" | "complete";
  response?: Response;
}

export class BrowserExecution {
  private active = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly owner: string;
  private readonly receiver: string;
  private readonly ethereum: Pick<EthereumWallet, "snapshot" | "extension">;
  private readonly zcash: Pick<ZcashWallet, "snapshot" | "execute">;
  private readonly profile: NetworkProfile;
  private readonly storage: Pick<Storage, "getItem" | "setItem">;
  private readonly transport: {
    request(): Promise<BrowserRequest | null>;
    respond(id: string, result: unknown, error: string | null): Promise<unknown>;
  };
  private readonly report: (message: string) => void;
  constructor(
    ethereum: Pick<EthereumWallet, "snapshot" | "extension">,
    zcash: Pick<ZcashWallet, "snapshot" | "execute">,
    profile: NetworkProfile,
    storage: Pick<Storage, "getItem" | "setItem">,
    transport: {
      request(): Promise<BrowserRequest | null>;
      respond(id: string, result: unknown, error: string | null): Promise<unknown>;
    },
    report: (message: string) => void,
  ) {
    this.ethereum = ethereum;
    this.zcash = zcash;
    this.profile = profile;
    this.storage = storage;
    this.transport = transport;
    this.report = report;
    this.owner = ethereum.snapshot().account?.toLowerCase() ?? "";
    this.receiver = zcash.snapshot().account?.usdc_receiver.owner ?? "";
  }
  start(): void {
    if (this.active) return;
    this.active = true;
    void this.poll();
  }
  stop(): void {
    this.active = false;
    clearTimeout(this.timer);
  }
  private async identity() {
    const provider = this.ethereum.extension();
    if (
      !this.active ||
      !provider ||
      this.ethereum.snapshot().account?.toLowerCase() !== this.owner ||
      this.zcash.snapshot().account?.usdc_receiver.owner !== this.receiver ||
      this.zcash.snapshot().account?.domain !== this.profile.deployment_id
    )
      throw new Error("Reconnect the original Testnet wallets to continue.");
    const chain = await provider.request({ method: "eth_chainId" });
    const accounts = await provider.request({ method: "eth_accounts" });
    if (
      chain !== "0xaa36a7" ||
      !Array.isArray(accounts) ||
      typeof accounts[0] !== "string" ||
      accounts[0].toLowerCase() !== this.owner
    )
      throw new Error("Connect the original wallet on Ethereum Sepolia.");
    return provider;
  }
  private async execute(request: BrowserRequest): Promise<unknown> {
    await this.identity();
    if (request.method !== "pool_transaction")
      return this.zcash.execute(request.method, request.params);
    if (
      typeof request.params.witness !== "string" ||
      typeof request.params.receipt !== "string" ||
      Object.keys(request.params).sort().join(",") !== "receipt,witness"
    )
      throw new Error("A genuine pool witness and proof are required.");
    const verified = (await this.zcash.execute("zutti_pool", {
      request: JSON.stringify({
        action: "verify_operation",
        witness: request.params.witness,
        receipt: request.params.receipt,
      }),
    })) as { chain_id: number; to: string; data: string; value: string; record: string | null };
    if (
      verified.chain_id !== 11155111 ||
      verified.to.toLowerCase() !== this.profile.ethereum.pool.toLowerCase() ||
      verified.value !== "0x0" ||
      !/^0x1d0a9b2e(?:[0-9a-f]{2})+$/.test(verified.data) ||
      verified.data.length > 131000
    )
      throw new Error("Wallet pool verification returned an invalid transaction.");
    const provider = await this.identity();
    const transaction = { from: this.owner, to: verified.to, data: verified.data, value: "0x0" };
    await provider.request({ method: "eth_call", params: [transaction, "latest"] });
    const hash = await provider.request({ method: "eth_sendTransaction", params: [transaction] });
    if (typeof hash !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(hash))
      throw new Error("Transaction submission is uncertain. Inspect MetaMask before continuing.");
    return { hash, data: verified.data, record: verified.record };
  }
  private async poll(): Promise<void> {
    try {
      const request = await this.transport.request();
      if (!this.active || request === null) return;
      if (!/^[0-9a-f]{64}$/.test(request.id))
        throw new Error("Invalid browser operation identifier.");
      const key = `zutti:execution:${this.profile.deployment_id}:${this.owner}:${this.receiver}:${request.id}`;
      const raw = this.storage.getItem(key);
      let retained: Retained | null = raw ? (JSON.parse(raw) as Retained) : null;
      if (retained?.phase === "executing") {
        this.report(
          "A wallet request was interrupted. Inspect your wallet before retrying; Zutti will not submit it again automatically.",
        );
        return;
      }
      if (!retained) {
        this.storage.setItem(key, JSON.stringify({ phase: "executing" }));
        let response: Response;
        try {
          response = { result: await this.execute(request), error: null };
        } catch (error) {
          response = {
            result: null,
            error: error instanceof Error ? error.message.slice(0, 500) : "Wallet request failed.",
          };
        }
        retained = { phase: "complete", response };
        this.storage.setItem(key, JSON.stringify(retained));
      }
      if (retained.phase !== "complete" || !retained.response)
        throw new Error("Invalid wallet operation recovery record.");
      if (!this.active) return;
      await this.transport.respond(request.id, retained.response.result, retained.response.error);
      if (retained.response.error) this.report(retained.response.error);
    } catch (error) {
      if (this.active)
        this.report(
          error instanceof Error ? error.message : "Browser swap connection interrupted.",
        );
    } finally {
      if (this.active)
        this.timer = setTimeout(() => {
          void this.poll();
        }, 2000);
    }
  }
}

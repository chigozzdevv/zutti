import type { NetworkProfile } from "@zutti/schema";
import type { BrowserDeposit } from "./api.js";
import type { EthereumWallet, EthereumProvider } from "./ethereum-wallet.js";
import type { ZcashWallet } from "./zcash-wallet.js";

export interface FundingState {
  stage:
    | "idle"
    | "preparing"
    | "awaiting_proof"
    | "proof_ready"
    | "approving"
    | "depositing"
    | "confirming"
    | "confirmed"
    | "inspect";
  amount: string | null;
  balance: string | null;
  error: string | null;
  transaction: string | null;
}
interface Retained {
  id: string;
  amount: string;
  units: string;
  approval: string | null;
  transaction: string | null;
  submitting: boolean;
}
const hash = (value: unknown): value is string =>
  typeof value === "string" && /^0x[0-9a-fA-F]{64}$/.test(value);
const quantity = (value: unknown): bigint => {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]+$/.test(value))
    throw new Error("Invalid wallet RPC response.");
  return BigInt(value);
};
const word = (value: bigint) => value.toString(16).padStart(64, "0");
const address_word = (value: string) => value.slice(2).toLowerCase().padStart(64, "0");
const decimal = (units: bigint) =>
  `${units / 1_000_000n}.${(units % 1_000_000n).toString().padStart(6, "0")}`.replace(/\.?0+$/, "");
export function usdc_units(amount: string): bigint {
  if (!/^(0|[1-9][0-9]{0,7})(\.[0-9]{1,6})?$/.test(amount))
    throw new Error("Enter a valid USDC amount.");
  const [whole, fraction = ""] = amount.split(".");
  const units = BigInt(whole!) * 1_000_000n + BigInt(fraction.padEnd(6, "0"));
  if (units <= 0n) throw new Error("Enter a USDC amount greater than zero.");
  return units;
}

export class BrowserFunding {
  private current: FundingState = {
    stage: "idle",
    amount: null,
    balance: null,
    error: null,
    transaction: null,
  };
  private readonly listeners = new Set<() => void>();
  private retained: Retained | null = null;
  private busy = false;
  private active = false;
  private epoch = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly owner: string;
  private readonly receiver: string;
  private readonly key: string;
  private readonly ethereum: Pick<EthereumWallet, "snapshot" | "extension">;
  private readonly zcash: Pick<ZcashWallet, "snapshot" | "prepare_deposit" | "verify_deposit">;
  private readonly profile: NetworkProfile;
  private readonly storage: Pick<Storage, "getItem" | "setItem">;
  private readonly transport: {
    prepare(witness: string): Promise<BrowserDeposit>;
    get(id: string): Promise<BrowserDeposit>;
  };

  constructor(
    ethereum: Pick<EthereumWallet, "snapshot" | "extension">,
    zcash: Pick<ZcashWallet, "snapshot" | "prepare_deposit" | "verify_deposit">,
    profile: NetworkProfile,
    storage: Pick<Storage, "getItem" | "setItem">,
    transport: {
      prepare(witness: string): Promise<BrowserDeposit>;
      get(id: string): Promise<BrowserDeposit>;
    },
  ) {
    this.ethereum = ethereum;
    this.zcash = zcash;
    this.profile = profile;
    this.storage = storage;
    this.transport = transport;
    this.owner = ethereum.snapshot().account?.toLowerCase() ?? "";
    this.receiver = zcash.snapshot().account?.usdc_receiver.owner ?? "";
    if (
      !/^0x[0-9a-f]{40}$/.test(this.owner) ||
      !this.receiver ||
      profile.name !== "testnet" ||
      profile.ethereum.chain_id !== "11155111" ||
      zcash.snapshot().account?.domain !== profile.deployment_id
    )
      throw new Error("Connect both Testnet wallets before funding.");
    this.key = `zutti:deposit:${profile.deployment_id}:${this.owner}:${this.receiver}`;
    try {
      const raw = storage.getItem(this.key);
      if (raw) {
        const value = JSON.parse(raw) as Retained;
        if (
          /^[0-9a-f]{64}$/.test(value.id) &&
          usdc_units(value.amount).toString() === value.units &&
          (value.approval === null || hash(value.approval)) &&
          (value.transaction === null || hash(value.transaction)) &&
          typeof value.submitting === "boolean"
        ) {
          this.retained = value;
          this.current = {
            ...this.current,
            amount: value.amount,
            transaction: value.transaction,
            stage: value.submitting
              ? "inspect"
              : value.transaction
                ? "confirming"
                : "awaiting_proof",
          };
        }
      }
    } catch {
      this.current = {
        ...this.current,
        stage: "inspect",
        error: "Deposit recovery data is unavailable. Check your wallet activity before retrying.",
      };
    }
  }
  snapshot = () => this.current;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(value: Partial<FundingState>) {
    if (!this.active) return;
    this.current = Object.freeze({ ...this.current, ...value });
    for (const listener of this.listeners) listener();
  }
  private save() {
    this.storage.setItem(this.key, JSON.stringify(this.retained));
  }
  private failure(error: unknown) {
    const message =
      error instanceof Error ? error.message : "Complete the pending request in your wallet.";
    this.update({ error: message.slice(0, 300) });
  }
  private async provider(): Promise<EthereumProvider> {
    const provider = this.ethereum.extension();
    if (
      !this.active ||
      !provider ||
      this.ethereum.snapshot().account?.toLowerCase() !== this.owner ||
      this.zcash.snapshot().account?.usdc_receiver.owner !== this.receiver ||
      this.zcash.snapshot().account?.domain !== this.profile.deployment_id
    )
      throw new Error("Your wallet changed. Reconnect the original wallet to continue.");
    const chain = await provider.request({ method: "eth_chainId" });
    if (quantity(chain) !== 11155111n) throw new Error("Switch MetaMask to Ethereum Sepolia.");
    const accounts = await provider.request({ method: "eth_accounts" });
    if (
      !Array.isArray(accounts) ||
      typeof accounts[0] !== "string" ||
      accounts[0].toLowerCase() !== this.owner ||
      !this.active ||
      provider !== this.ethereum.extension()
    )
      throw new Error("The selected MetaMask account changed.");
    return provider;
  }
  private async call(data: string): Promise<bigint> {
    const provider = await this.provider();
    return quantity(
      await provider.request({
        method: "eth_call",
        params: [{ to: this.profile.ethereum.usdc, data }, "latest"],
      }),
    );
  }
  private async preflight(units: bigint) {
    const balance = await this.call(`0x70a08231${address_word(this.owner)}`);
    this.update({ balance: decimal(balance) });
    if (balance < units)
      throw new Error(
        `Your connected wallet has ${decimal(balance)} Sepolia USDC. Fund it with test USDC first.`,
      );
    const provider = await this.provider();
    if (
      quantity(
        await provider.request({ method: "eth_getBalance", params: [this.owner, "latest"] }),
      ) === 0n
    )
      throw new Error("Fund your connected wallet with Sepolia ETH for transaction fees.");
  }
  start() {
    if (this.active) return;
    this.active = true;
    const epoch = ++this.epoch;
    const poll = async () => {
      try {
        const balance = await this.call(`0x70a08231${address_word(this.owner)}`);
        if (!this.active || epoch !== this.epoch) return;
        this.update({ balance: decimal(balance) });
        if (
          !this.busy &&
          this.retained &&
          this.current.stage !== "inspect" &&
          this.current.stage !== "confirmed"
        ) {
          if (this.retained.transaction) await this.confirm();
          else {
            const deposit = await this.transport.get(this.retained.id);
            if (!this.active || epoch !== this.epoch) return;
            this.validate(deposit);
            this.update({
              stage: deposit.state === "proof_ready" ? "proof_ready" : "awaiting_proof",
            });
          }
        }
      } catch (error) {
        if (this.active) this.failure(error);
      }
      if (this.active && epoch === this.epoch)
        this.timer = setTimeout(() => {
          void poll();
        }, 5000);
    };
    void poll();
  }
  stop() {
    this.active = false;
    this.epoch++;
    clearTimeout(this.timer);
  }
  private validate(value: BrowserDeposit) {
    if (
      !this.retained ||
      value.id !== this.retained.id ||
      !/^(?:[0-9a-f]{2})+$/.test(value.witness) ||
      value.witness.length > 32768 ||
      !["awaiting_proof", "proof_ready"].includes(value.state) ||
      (value.state === "proof_ready" &&
        (typeof value.receipt !== "string" ||
          !/^(?:[0-9a-f]{2})+$/.test(value.receipt) ||
          value.receipt.length > 32 * 1024 * 1024))
    )
      throw new Error("Invalid deposit proof response.");
  }
  async prepare(amount: string) {
    if (
      this.busy ||
      (this.retained && this.current.stage !== "confirmed") ||
      this.current.stage === "inspect"
    )
      return;
    if (this.current.stage === "confirmed") this.retained = null;
    this.busy = true;
    this.update({ stage: "preparing", error: null });
    try {
      const units = usdc_units(amount);
      await this.preflight(units);
      const prepared = await this.zcash.prepare_deposit(amount, this.owner);
      await this.provider();
      if (
        prepared.units !== units.toString() ||
        prepared.pool.toLowerCase() !== this.profile.ethereum.pool.toLowerCase() ||
        prepared.token.toLowerCase() !== this.profile.ethereum.usdc.toLowerCase()
      )
        throw new Error("Deposit deployment or amount mismatch.");
      const value = await this.transport.prepare(prepared.witness);
      if (!/^[0-9a-f]{64}$/.test(value.id) || value.witness !== prepared.witness)
        throw new Error("Prepared deposit identity mismatch.");
      this.retained = {
        id: value.id,
        amount: prepared.amount,
        units: units.toString(),
        approval: null,
        transaction: null,
        submitting: false,
      };
      this.save();
      this.validate(value);
      this.update({
        stage: value.state === "proof_ready" ? "proof_ready" : "awaiting_proof",
        amount: prepared.amount,
      });
    } catch (error) {
      this.update({ stage: this.retained ? "awaiting_proof" : "idle" });
      this.failure(error);
    } finally {
      this.busy = false;
    }
  }
  async complete() {
    if (this.busy || !this.retained || this.current.stage !== "proof_ready") return;
    this.busy = true;
    this.update({ error: null });
    try {
      const retained = this.retained;
      const deposit = await this.transport.get(retained.id);
      this.validate(deposit);
      if (!deposit.receipt) throw new Error("Deposit proof remains pending.");
      const call = await this.zcash.verify_deposit(deposit.witness, deposit.receipt);
      if (
        call.from !== this.owner ||
        call.units !== retained.units ||
        call.to.toLowerCase() !== this.profile.ethereum.pool.toLowerCase() ||
        call.token.toLowerCase() !== this.profile.ethereum.usdc.toLowerCase()
      )
        throw new Error("Verified deposit differs from your approved deposit.");
      await this.preflight(BigInt(retained.units));
      const allowance = await this.call(
        `0xdd62ed3e${address_word(this.owner)}${address_word(this.profile.ethereum.pool)}`,
      );
      if (allowance < BigInt(retained.units)) {
        if (!retained.approval) {
          this.update({ stage: "approving" });
          retained.approval = await this.send(
            this.profile.ethereum.usdc,
            `0x095ea7b3${address_word(this.profile.ethereum.pool)}${word(BigInt(retained.units))}`,
          );
          this.save();
        }
        this.update({ stage: "approving" });
        const deadline = Date.now() + 180_000;
        while (!(await this.receipt(retained.approval))) {
          if (Date.now() >= deadline)
            throw new Error(
              "Token approval is still pending. Continue this deposit after it is mined.",
            );
          await new Promise((resolve) => setTimeout(resolve, 3000));
        }
      }
      this.update({ stage: "depositing" });
      retained.transaction = await this.send(call.to, call.data);
      this.save();
      this.update({ stage: "confirming", transaction: retained.transaction });
      await this.confirm();
    } catch (error) {
      if (this.snapshot().stage !== "inspect") {
        this.update({ stage: this.retained?.transaction ? "confirming" : "proof_ready" });
        this.failure(error);
      }
    } finally {
      this.busy = false;
    }
  }
  private async send(to: string, data: string): Promise<string> {
    const provider = await this.provider();
    this.retained!.submitting = true;
    this.save();
    try {
      const result = await provider.request({
        method: "eth_sendTransaction",
        params: [{ from: this.owner, to, data, value: "0x0", chainId: "0xaa36a7" }],
      });
      if (!hash(result)) throw new Error("Wallet returned no transaction hash.");
      this.retained!.submitting = false;
      return result;
    } catch (error) {
      if (typeof error === "object" && error !== null && "code" in error && error.code === 4001) {
        this.retained!.submitting = false;
        this.save();
      } else
        this.update({
          stage: "inspect",
          error: "Transaction outcome is unknown. Inspect MetaMask activity before retrying.",
        });
      throw error;
    }
  }
  private async receipt(transaction: string): Promise<Record<string, unknown> | null> {
    const provider = await this.provider();
    const receipt = await provider.request({
      method: "eth_getTransactionReceipt",
      params: [transaction],
    });
    if (receipt === null) return null;
    if (
      typeof receipt !== "object" ||
      Array.isArray(receipt) ||
      !("transactionHash" in receipt) ||
      String(receipt.transactionHash).toLowerCase() !== transaction.toLowerCase()
    )
      throw new Error("Invalid transaction receipt.");
    const value = receipt as Record<string, unknown>;
    const expected =
      transaction === this.retained?.transaction
        ? this.profile.ethereum.pool
        : this.profile.ethereum.usdc;
    if (
      String(value.from).toLowerCase() !== this.owner ||
      String(value.to).toLowerCase() !== expected.toLowerCase()
    )
      throw new Error("Transaction receipt sender or destination mismatch.");
    if (quantity(value.status) !== 1n)
      throw new Error("The transaction reverted. Inspect it in MetaMask.");
    return value;
  }
  private async confirm() {
    const transaction = this.retained?.transaction;
    if (!transaction) return;
    const receipt = await this.receipt(transaction);
    if (!receipt) return;
    const provider = await this.provider();
    const finalized = await provider.request({
      method: "eth_getBlockByNumber",
      params: ["finalized", false],
    });
    if (
      typeof finalized !== "object" ||
      finalized === null ||
      !("number" in finalized) ||
      quantity(finalized.number) < quantity(receipt.blockNumber)
    )
      return;
    const canonical = await provider.request({
      method: "eth_getBlockByNumber",
      params: [receipt.blockNumber, false],
    });
    if (
      typeof canonical !== "object" ||
      canonical === null ||
      !("hash" in canonical) ||
      canonical.hash !== receipt.blockHash
    )
      throw new Error("Deposit block changed. Waiting for canonical confirmation.");
    this.update({ stage: "confirmed", error: null, transaction });
  }
}

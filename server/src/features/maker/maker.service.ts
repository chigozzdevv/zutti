import { create_zutti_maker } from "@zutti/maker-sdk";
import type { MakerWalletProvider } from "@zutti/maker-sdk";
import { canonical } from "@zutti/runtime";
import {
  ZuttiError,
  maker_control,
  maker_deposit_input,
  maker_deposit_receipt,
  maker_inventory,
  maker_snapshot,
  maker_start_options,
  maker_trades,
} from "@zutti/schema";
import type {
  MakerControl,
  MakerDepositInput,
  MakerDepositOperation,
  MakerDepositReceipt,
  MakerInventory,
  MakerSnapshot,
  MakerStartOptions,
  MakerTrades,
  NetworkProfile,
} from "@zutti/schema";

export interface MakerConnection {
  wallet: MakerWalletProvider;
  settings?: MakerStartOptions;
  inventory?: () => Promise<MakerInventory>;
  trades?: () => Promise<MakerTrades>;
  deposit?: (input: Readonly<MakerDepositInput>) => Promise<MakerDepositReceipt>;
}
export class MakerService {
  private readonly sdk: ReturnType<typeof create_zutti_maker>;
  private settings: MakerStartOptions | null;
  private pending: MakerControl | null = null;
  private action: Promise<void> | null = null;
  private closed = false;
  private error: string | null = null;
  private readonly deposits = new Map<string, MakerDepositOperation>();
  constructor(
    private readonly connection: MakerConnection,
    private readonly profile: NetworkProfile,
  ) {
    this.sdk = create_zutti_maker({ network: "testnet", wallet: connection.wallet });
    if (canonical(this.sdk.network) !== canonical(profile))
      throw new ZuttiError("deployment_unavailable", "Maker wallet uses a different deployment.");
    this.settings = connection.settings ? maker_start_options.parse(connection.settings) : null;
  }
  static unavailable(): MakerSnapshot {
    return {
      available: false,
      maker_id: null,
      state: "unavailable",
      settings: null,
      live_offers: null,
      active_trades: null,
      inventory: null,
      trades: null,
      can_deposit: false,
      deposit: null,
      pending_action: null,
      error: null,
      checked_at: Date.now(),
    };
  }
  busy(): boolean {
    return (
      this.pending !== null ||
      [...this.deposits.values()].some((item) => item.state !== "confirmed")
    );
  }
  private fresh(
    value: { checked_at: number; deployment_id: string; maker_id: string },
    id: string,
  ): boolean {
    return (
      value.deployment_id === this.profile.deployment_id &&
      value.maker_id === id &&
      value.checked_at <= Date.now() + 30_000 &&
      Date.now() - value.checked_at <= 300_000
    );
  }
  async snapshot(): Promise<MakerSnapshot> {
    const summary = await this.sdk.summary();
    const [inventory_result, trade_result] = await Promise.allSettled([
      Promise.resolve().then(() => this.connection.inventory?.()),
      Promise.resolve().then(() => this.connection.trades?.()),
    ]);
    const inventory =
      inventory_result.status === "fulfilled" && inventory_result.value !== undefined
        ? maker_inventory.safeParse(inventory_result.value)
        : null;
    const trades =
      trade_result.status === "fulfilled" && trade_result.value !== undefined
        ? maker_trades.safeParse(trade_result.value)
        : null;
    const admitted_inventory =
      inventory?.success && this.fresh(inventory.data, summary.maker_id) ? inventory.data : null;
    const admitted_trades =
      trades?.success && this.fresh(trades.data, summary.maker_id) ? trades.data : null;
    return maker_snapshot.parse({
      available: true,
      maker_id: summary.maker_id,
      state: summary.state,
      settings: this.settings,
      live_offers: summary.live_offers,
      active_trades: admitted_trades?.complete
        ? admitted_trades.items.filter((item) => item.state !== "settled").length
        : null,
      inventory: admitted_inventory,
      trades: admitted_trades,
      can_deposit: !!this.connection.deposit,
      deposit: [...this.deposits.values()].at(-1) ?? null,
      pending_action: this.pending?.action ?? null,
      error: this.error,
      checked_at: Date.now(),
    });
  }
  async control(value: unknown): Promise<MakerSnapshot> {
    if (this.closed) throw new ZuttiError("wallet_unavailable", "Reconnect your maker wallet.");
    const parsed = maker_control.safeParse(value);
    if (!parsed.success) throw new ZuttiError("invalid_input", "Check your maker settings.");
    const input = parsed.data;
    if (
      input.action === "start" &&
      (input.settings.pairs.length !== 1 || input.settings.pairs[0] !== "ZEC/USDC")
    )
      throw new ZuttiError("invalid_input", "This maker supplies USDC for ZEC to USDC swaps.");
    if (this.pending) {
      if (canonical(this.pending) !== canonical(input))
        throw new ZuttiError("service_unavailable", "A maker action is already running.");
      return this.snapshot();
    }
    this.pending = input;
    this.error = null;
    this.action = Promise.resolve()
      .then(async () => {
        if (input.action === "stop") {
          await this.sdk.stop();
          return;
        }
        const state = (await this.sdk.summary()).state;
        if (
          input.action === "start" &&
          state === "running" &&
          canonical(this.settings) === canonical(input.settings)
        )
          return;
        if (
          (input.action === "pause" && state === "paused") ||
          (input.action === "resume" && state === "running")
        )
          return;
        if (input.action === "start") {
          await this.sdk.start(input.settings);
          this.settings = input.settings;
        } else if (input.action === "pause") await this.sdk.pause();
        else if (input.action === "resume") await this.sdk.resume();
      })
      .catch(() => {
        this.error = "The maker action did not complete. Check your wallet and try again.";
      })
      .finally(() => {
        this.pending = null;
        this.action = null;
      });
    return this.snapshot();
  }
  async deposit(value: unknown): Promise<MakerSnapshot> {
    if (this.closed) throw new ZuttiError("wallet_unavailable", "Reconnect your maker wallet.");
    const parsed = maker_deposit_input.safeParse(value);
    if (!parsed.success) throw new ZuttiError("invalid_input", "Enter a valid USDC amount.");
    const input = parsed.data;
    if (!this.connection.deposit)
      throw new ZuttiError(
        "wallet_unavailable",
        "This maker wallet does not provide inventory funding.",
      );
    const previous = this.deposits.get(input.id);
    if (previous) {
      if (previous.amount !== input.amount)
        throw new ZuttiError("invalid_input", "This deposit already has a different amount.");
      return this.snapshot();
    }
    if ([...this.deposits.values()].some((item) => item.state !== "confirmed"))
      throw new ZuttiError("service_unavailable", "A deposit is already pending in your wallet.");
    if (this.deposits.size >= 100)
      throw new ZuttiError("service_unavailable", "Deposit history limit reached.");
    const operation: MakerDepositOperation = {
      id: input.id,
      amount: input.amount,
      state: "pending",
      receipt: null,
      error: null,
    };
    this.deposits.set(input.id, operation);
    void Promise.resolve()
      .then(() => this.connection.deposit!(Object.freeze(input)))
      .then((value) => {
        const receipt = maker_deposit_receipt.parse(value);
        if (
          receipt.id !== input.id ||
          receipt.amount !== input.amount ||
          receipt.evidence.chain !== "ethereum" ||
          receipt.evidence.policy_id !== this.profile.policy.source_policy_id ||
          receipt.confirmed_at > Date.now() + 30_000
        )
          throw new Error("Deposit does not match the authorized input");
        operation.state = "confirmed";
        operation.receipt = receipt;
      })
      .catch(() => {
        operation.state = "paused";
        operation.error = "Deposit pending in your wallet. It has not been submitted again.";
      });
    return this.snapshot();
  }
  async close(): Promise<void> {
    this.closed = true;
    await this.action;
    await this.sdk.stop();
  }
}

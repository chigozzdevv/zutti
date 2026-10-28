import { randomBytes } from "node:crypto";
import { create_zutti } from "@zutti/sdk";
import type { Intent, WalletProvider, Zutti } from "@zutti/sdk";
import { ZuttiError, accept_input, quote_input, withdrawal_input } from "@zutti/schema";
import type {
  AppInformation,
  AppOperation,
  OfferCollection,
  StatusUpdate,
  WalletSession,
} from "@zutti/schema";
import { canonical } from "@zutti/runtime";
import { MakerService } from "../maker/maker.service.js";
import type { MakerConnection } from "../maker/maker.service.js";

export interface WalletConnection {
  wallet: WalletProvider;
  label: string;
  maker?: MakerConnection;
}
export type ConnectWallet = () => Promise<WalletConnection>;
interface RetainedIntent {
  intent: Intent;
  offers: OfferCollection;
  operation?: string;
  offer_id?: string;
}
interface Session {
  sdk: Zutti;
  maker: MakerService | null;
  label: string;
  expires: number;
  intents: Map<string, RetainedIntent>;
  operations: Map<string, AppOperation>;
  withdrawals: Map<string, { id: string; terms: string }>;
}
const identifier = (): string => randomBytes(32).toString("hex");

export class WalletService {
  private readonly sessions = new Map<string, Session>();
  constructor(
    private readonly information: AppInformation,
    private readonly factory?: ConnectWallet,
  ) {}

  async connect(): Promise<{ token: string; session: WalletSession }> {
    if (!this.factory)
      throw new ZuttiError(
        "wallet_unavailable",
        "Wallet connection is not available in this build yet.",
      );
    for (const [key, session] of this.sessions) {
      if (
        session.expires < Date.now() &&
        ![...session.operations.values()].some(
          (item) => item.state === "pending" || item.state === "paused",
        )
      ) {
        await session.maker?.close();
        await session.sdk.close();
        this.sessions.delete(key);
      }
    }
    if (this.sessions.size >= 8)
      throw new ZuttiError("service_unavailable", "Wallet connection limit reached.");
    const connection = await this.factory();
    const sdk = create_zutti({ network: "testnet", wallet: connection.wallet });
    if (canonical(sdk.network) !== canonical(this.information.profile)) {
      await sdk.close();
      throw new ZuttiError(
        "deployment_unavailable",
        "Wallet is connected to a different deployment.",
      );
    }
    let maker: MakerService | null = null;
    try {
      if (connection.maker) maker = new MakerService(connection.maker, this.information.profile);
    } catch (error) {
      await sdk.close();
      throw error;
    }
    const token = identifier();
    this.sessions.set(token, {
      sdk,
      maker,
      label: connection.label.slice(0, 60),
      expires: Date.now() + 86_400_000,
      intents: new Map(),
      operations: new Map(),
      withdrawals: new Map(),
    });
    return { token, session: { connected: true, label: connection.label.slice(0, 60) } };
  }
  private session(token: string | undefined): Session {
    const session = token ? this.sessions.get(token) : undefined;
    if (!session || session.expires <= Date.now())
      throw new ZuttiError("wallet_unavailable", "Connect your wallet to continue.");
    return session;
  }
  summary(token: string | undefined): WalletSession {
    try {
      return { connected: true, label: this.session(token).label };
    } catch {
      return { connected: false, label: null };
    }
  }
  async disconnect(token: string | undefined): Promise<void> {
    if (!token) return;
    const session = this.sessions.get(token);
    if (!session) return;
    if (session.maker) {
      const maker = await session.maker.snapshot();
      if (session.maker.busy() || maker.state !== "stopped")
        throw new ZuttiError(
          "service_unavailable",
          "Stop quoting before disconnecting your maker wallet.",
        );
    }
    if (
      [...session.operations.values()].some(
        (item) => item.state === "pending" || item.state === "paused",
      )
    )
      throw new ZuttiError(
        "service_unavailable",
        "Your swap is still running. Keep this wallet session connected.",
      );
    await session.maker?.close();
    await session.sdk.close();
    this.sessions.delete(token);
  }
  async quote(token: string | undefined, value: unknown) {
    const session = this.session(token);
    if (session.intents.size >= 100)
      throw new ZuttiError("service_unavailable", "Reconnect your wallet to refresh quotes.");
    if (
      [...session.operations.values()].some(
        (item) => item.state === "pending" || item.state === "paused",
      )
    )
      throw new ZuttiError(
        "service_unavailable",
        "Finish your current operation before starting another.",
      );
    const intent = session.sdk.intent(quote_input.parse(value));
    const offers = await intent.offers();
    session.intents.set(intent.id, { intent, offers });
    return { id: intent.id, offers };
  }
  private operation(
    session: Session,
    kind: AppOperation["kind"],
    parent_id: string | null = null,
  ): AppOperation {
    const now = Date.now();
    const operation: AppOperation = {
      id: identifier(),
      parent_id,
      input: null,
      kind,
      state: "pending",
      started_at: now,
      updated_at: now,
      events: [],
      receipt: null,
      withdrawal: null,
      error: null,
    };
    session.operations.set(operation.id, operation);
    return operation;
  }
  private update(operation: AppOperation, status: StatusUpdate): void {
    operation.updated_at = Date.now();
    operation.events.push({
      stage: status.stage,
      message: status.message,
      at: operation.updated_at,
    });
    if (operation.events.length > 128) operation.events.shift();
  }
  private pause(operation: AppOperation): void {
    operation.state = "paused";
    operation.updated_at = Date.now();
    operation.error =
      "The operation is pending in your wallet. No refund or cancellation has been assumed.";
  }
  accept(token: string | undefined, id: string, value: unknown): AppOperation {
    const session = this.session(token);
    const input = accept_input.parse(value);
    const retained = session.intents.get(id);
    if (!retained) throw new ZuttiError("invalid_input", "Request a fresh quote.");
    if (retained.operation) {
      if (retained.offer_id !== input.offer_id)
        throw new ZuttiError("invalid_input", "Another offer already owns this swap.");
      return this.get(token, retained.operation);
    }
    const offer = retained.offers.items.find((item) => item.id === input.offer_id);
    if (!offer || offer.deadline <= Date.now())
      throw new ZuttiError("expired", "This quote expired. Request another.");
    if (
      [...session.operations.values()].some(
        (item) => item.state === "pending" || item.state === "paused",
      )
    )
      throw new ZuttiError("service_unavailable", "Another operation is already running.");
    const operation = this.operation(session, "swap");
    operation.input = offer.input;
    retained.operation = operation.id;
    retained.offer_id = input.offer_id;
    void retained.intent
      .accept({ offer_id: input.offer_id, on_status: (status) => this.update(operation, status) })
      .then((swap) => {
        operation.receipt = swap.receipt;
        operation.state = "settled";
        operation.updated_at = Date.now();
      })
      .catch(() => this.pause(operation));
    return structuredClone(operation);
  }
  withdraw(token: string | undefined, parent: string, value: unknown): AppOperation {
    const session = this.session(token);
    const swap = this.get(token, parent);
    const input = withdrawal_input.parse(value);
    if (swap.state !== "settled" || !swap.receipt || input.amount !== swap.receipt.received.amount)
      throw new ZuttiError("invalid_input", "Withdraw the verified amount from a completed swap.");
    const terms = canonical({ ...input, to: input.to.toLowerCase() });
    const previous = session.withdrawals.get(parent);
    if (previous) {
      if (previous.terms !== terms)
        throw new ZuttiError("invalid_input", "This swap already has a different withdrawal.");
      return this.get(token, previous.id);
    }
    if (
      [...session.operations.values()].some(
        (item) => item.state === "pending" || item.state === "paused",
      )
    )
      throw new ZuttiError("service_unavailable", "Another operation is already running.");
    const operation = this.operation(session, "withdrawal", parent);
    operation.input = { asset: "USDC", amount: input.amount };
    session.withdrawals.set(parent, { id: operation.id, terms });
    void session.sdk
      .account()
      .withdraw({ ...input, on_status: (status) => this.update(operation, status) })
      .then((receipt) => {
        operation.withdrawal = receipt;
        operation.state = "confirmed";
        operation.updated_at = Date.now();
      })
      .catch(() => this.pause(operation));
    return structuredClone(operation);
  }
  get(token: string | undefined, id: string): AppOperation {
    const operation = this.session(token).operations.get(id);
    if (!operation)
      throw new ZuttiError(
        "invalid_input",
        "This operation is not in the connected wallet session.",
      );
    return structuredClone(operation);
  }
  list(token: string | undefined): AppOperation[] {
    return structuredClone([...this.session(token).operations.values()]);
  }
  maker_summary(token: string | undefined) {
    return this.session(token).maker?.snapshot() ?? Promise.resolve(MakerService.unavailable());
  }
  maker_control(token: string | undefined, input: unknown) {
    const maker = this.session(token).maker;
    if (!maker)
      throw new ZuttiError("wallet_unavailable", "Connect a maker wallet to manage quoting.");
    return maker.control(input);
  }
  maker_deposit(token: string | undefined, input: unknown) {
    const maker = this.session(token).maker;
    if (!maker)
      throw new ZuttiError("wallet_unavailable", "Connect a maker wallet to fund inventory.");
    return maker.deposit(input);
  }
  async close(): Promise<void> {
    await Promise.all(
      [...this.sessions.values()].map(async (session) => {
        await session.maker?.close();
        await session.sdk.close();
      }),
    );
    this.sessions.clear();
  }
}

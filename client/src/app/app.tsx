import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  ArrowRight,
  ArrowUpRight,
  ChevronRight,
  LoaderCircle,
  ShieldCheck,
  Wallet,
} from "lucide-react";
import type {
  ActivityItem,
  AppInformation,
  AppOperation,
  Offer,
  QuoteResponse,
  WalletSession,
} from "@zutti/schema";
import { withdrawal_input } from "@zutti/schema";
import { api, error_message } from "../shared/api.js";
import { amount_error, duration, route } from "../shared/format.js";
import { Activity } from "../components/activity.js";
import { Dialog } from "../components/dialog.js";
import { Progress } from "../components/progress.js";
import { Swap } from "../components/swap.js";
import { Token } from "../components/token.js";
import { Maker } from "../components/maker.js";
import { WalletDialog } from "../components/wallet.js";
import { EthereumWallet } from "../shared/ethereum-wallet.js";
import type { WalletBrowser } from "../shared/ethereum-wallet.js";
import { ZcashWallet } from "../shared/zcash-wallet.js";
import { BrowserFunding } from "../shared/funding.js";

type Modal = "wallet" | "review" | "withdraw" | null;
export function App() {
  const [ethereum_wallet] = useState(() => new EthereumWallet(window as unknown as WalletBrowser));
  const ethereum = useSyncExternalStore(ethereum_wallet.subscribe, ethereum_wallet.snapshot);
  const [zcash_wallet] = useState(() => new ZcashWallet(ethereum_wallet.extension));
  const zcash = useSyncExternalStore(zcash_wallet.subscribe, zcash_wallet.snapshot);
  const [path, set_path] = useState(location.pathname);
  const [information, set_information] = useState<AppInformation | null>(null);
  const [session, set_session] = useState<WalletSession>({ connected: false, label: null });
  const [amount, set_amount] = useState("0.01");
  const [quotes, set_quotes] = useState<QuoteResponse | null>(null);
  const [selected_id, set_selected_id] = useState<string | null>(null);
  const [operation, set_operation] = useState<AppOperation | null>(null);
  const [retained_operations, set_retained_operations] = useState<AppOperation[]>([]);
  const [swap_operation, set_swap_operation] = useState<AppOperation | null>(null);
  const [modal, set_modal] = useState<Modal>(null);
  const [error, set_error] = useState<string | null>(null);
  const [modal_error, set_modal_error] = useState<string | null>(null);
  const [initial_error, set_initial_error] = useState(false);
  const [busy, set_busy] = useState(false);
  const [modal_busy, set_modal_busy] = useState(false);
  const [to, set_to] = useState("");
  const [now, set_now] = useState(Date.now());
  const funding = useMemo(
    () =>
      information && ethereum.account && zcash.account
        ? new BrowserFunding(
            ethereum_wallet,
            zcash_wallet,
            information.profile,
            window.localStorage,
            { prepare: api.prepare_deposit, get: api.deposit },
          )
        : null,
    [information, ethereum.account, zcash.account, ethereum_wallet, zcash_wallet],
  );
  useEffect(() => {
    funding?.start();
    return () => funding?.stop();
  }, [funding]);
  const revision = useRef(0);
  const selected: Offer | null =
    quotes?.offers.items.find((item) => item.id === selected_id) ?? null;

  useEffect(() => {
    ethereum_wallet.start();
    return () => {
      zcash_wallet.stop();
      ethereum_wallet.stop();
    };
  }, [ethereum_wallet, zcash_wallet]);
  useEffect(() => {
    const navigate = () => set_path(location.pathname);
    addEventListener("popstate", navigate);
    const tick = setInterval(() => set_now(Date.now()), 1000);
    return () => {
      removeEventListener("popstate", navigate);
      clearInterval(tick);
    };
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    void Promise.all([api.information(controller.signal), api.wallet(controller.signal)])
      .then(async ([info, wallet]) => {
        set_information(info);
        set_session(wallet);
        if (wallet.connected) {
          const retained = await api.operations(controller.signal);
          if (controller.signal.aborted) return;
          set_retained_operations(retained);
          const latest = retained.at(-1);
          if (latest) set_operation(latest);
          set_swap_operation(retained.filter((item) => item.kind === "swap").at(-1) ?? null);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) set_initial_error(true);
      });
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (!operation || operation.state !== "pending") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const id = operation.id;
    const poll = async () => {
      try {
        const result = await api.operation(id, controller.signal);
        if (controller.signal.aborted) return;
        remember(result);
        set_error(null);
        if (result.state !== "pending") return;
      } catch {
        if (controller.signal.aborted) return;
        set_error("Connection interrupted. Your wallet keeps the operation. Reconnecting…");
      }
      timer = setTimeout(() => {
        void poll();
      }, 2500);
    };
    timer = setTimeout(() => {
      void poll();
    }, 1000);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [operation?.id, operation?.state]);

  function remember(value: AppOperation) {
    set_operation(value);
    if (value.kind === "swap") set_swap_operation(value);
    set_retained_operations((items) => [...items.filter((item) => item.id !== value.id), value]);
  }
  const wallet_activity: ActivityItem[] = retained_operations.flatMap((item) => {
    if (item.kind !== "swap" || !item.receipt || !item.input) return [];
    const receipt = item.receipt;
    const withdrawal = retained_operations.find(
      (candidate) => candidate.parent_id === item.id && candidate.state === "confirmed",
    )?.withdrawal;
    return [
      {
        id: receipt.swap_id,
        input: item.input,
        output: receipt.received,
        settled_at: receipt.confirmed_at,
        withdrawn_at: withdrawal?.confirmed_at ?? null,
        source_transaction: receipt.source_payment.transaction_id.replace(/^0x/, ""),
        payout_transaction: receipt.pool_settlement.transaction_id,
        withdrawal_transaction: withdrawal?.evidence.transaction_id ?? null,
        duration_seconds: Math.max(0, (receipt.confirmed_at - item.started_at) / 1000),
      },
    ];
  });
  function open_modal(value: Modal) {
    if (value === "withdraw" && !to && ethereum.account) set_to(ethereum.account);
    set_modal_error(null);
    set_modal(value);
  }
  async function connect() {
    set_modal_busy(true);
    set_modal_error(null);
    try {
      set_session(await api.connect());
      set_modal(null);
    } catch (reason) {
      set_modal_error(error_message(reason));
    } finally {
      set_modal_busy(false);
    }
  }
  async function request_quotes() {
    const invalid = amount_error(amount);
    if (invalid) {
      set_error(invalid);
      return;
    }
    const current_revision = ++revision.current;
    set_busy(true);
    set_error(null);
    set_quotes(null);
    set_selected_id(null);
    try {
      const result = await api.quote(amount);
      if (revision.current !== current_revision) return;
      set_quotes(result);
      set_selected_id(result.offers.items[0]?.id ?? null);
    } catch (reason) {
      if (revision.current === current_revision) set_error(error_message(reason));
    } finally {
      if (revision.current === current_revision) set_busy(false);
    }
  }
  async function accept_offer() {
    if (!quotes || !selected || selected.deadline <= Date.now()) {
      set_modal_error("This quote expired. Close this window and refresh your quotes.");
      return;
    }
    set_modal_busy(true);
    set_modal_error(null);
    try {
      const result = await api.accept(quotes.id, selected.id);
      remember(result);
      set_modal(null);
    } catch (reason) {
      set_modal_error(error_message(reason));
    } finally {
      set_modal_busy(false);
    }
  }
  async function withdraw() {
    if (!swap_operation?.receipt) return;
    const terms = withdrawal_input.safeParse({
      asset: "USDC",
      amount: swap_operation.receipt.received.amount,
      to: to.trim(),
    });
    if (!terms.success) {
      set_modal_error("Enter a valid Ethereum address.");
      return;
    }
    set_modal_busy(true);
    set_modal_error(null);
    try {
      remember(await api.withdraw(swap_operation.id, terms.data.amount, terms.data.to));
      set_modal(null);
    } catch (reason) {
      set_modal_error(error_message(reason));
    } finally {
      set_modal_busy(false);
    }
  }
  async function disconnect() {
    set_modal_busy(true);
    set_modal_error(null);
    try {
      set_session(await api.disconnect());
      set_operation(null);
      set_swap_operation(null);
      set_retained_operations([]);
      set_quotes(null);
      set_selected_id(null);
      set_modal(null);
    } catch (reason) {
      set_modal_error(error_message(reason));
    } finally {
      set_modal_busy(false);
    }
  }
  const pending = operation?.state === "pending" || operation?.state === "paused";
  const is_activity = path === "/activity";
  const is_maker = path === "/maker";
  return (
    <div className="app-shell">
      <header className="header">
        <a
          className="wordmark"
          href="/"
          aria-label="Zutti home"
          onClick={(event) => {
            event.preventDefault();
            route("/");
          }}
        >
          <img src="/wordmark.svg" alt="Zutti" width="102" height="40" />
        </a>
        <nav aria-label="Main navigation">
          <a
            href="/"
            aria-current={!is_activity && !is_maker ? "page" : undefined}
            onClick={(event) => {
              event.preventDefault();
              route("/");
            }}
          >
            Swap
          </a>
          <a
            href="/activity"
            aria-current={is_activity ? "page" : undefined}
            onClick={(event) => {
              event.preventDefault();
              route("/activity");
            }}
          >
            Activity
          </a>
          <a
            href="/maker"
            aria-current={is_maker ? "page" : undefined}
            onClick={(event) => {
              event.preventDefault();
              route("/maker");
            }}
          >
            Maker
          </a>
        </nav>
        <div className="header-actions">
          <button
            className={`wallet-button${session.connected || ethereum.account || zcash.account ? " connected" : ""}`}
            onClick={() => open_modal("wallet")}
          >
            <Wallet size={16} />
            <span>
              {session.connected || ethereum.account || zcash.account
                ? "My wallet"
                : "Connect wallet"}
            </span>
          </button>
        </div>
      </header>
      <main className={is_activity || is_maker ? "activity-main" : undefined}>
        {initial_error && (
          <div className="initial-error" role="alert">
            <p>Couldn’t reach Zutti.</p>
            <button className="text-button" onClick={() => location.reload()}>
              Try again <ArrowRight size={14} />
            </button>
          </div>
        )}
        {is_maker ? (
          <Maker connected={session.connected} funding={funding} />
        ) : is_activity ? (
          <Activity
            items={session.connected ? wallet_activity : (information?.activity ?? [])}
            pending={operation?.state === "pending" || operation?.state === "paused"}
          />
        ) : operation ? (
          <section className="operation-view">
            <button className="back-link" onClick={() => route("/activity")}>
              Your activity <ChevronRight size={13} />
            </button>
            <Progress
              operation={operation}
              now={now}
              on_withdraw={() => open_modal("withdraw")}
              new_swap={() => {
                set_operation(null);
                set_quotes(null);
                set_selected_id(null);
                set_error(null);
              }}
            />
            {error && (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
          </section>
        ) : (
          <Swap
            amount={amount}
            set_amount={(value) => {
              revision.current += 1;
              set_amount(value);
              set_quotes(null);
              set_selected_id(null);
              set_error(null);
              set_busy(false);
            }}
            connected={session.connected}
            ethereum_connected={!!ethereum.account}
            zcash_connected={!!zcash.account}
            loading={busy}
            ready={!!information}
            now={now}
            quotes={quotes}
            selected={selected}
            select={set_selected_id}
            request={() => {
              void request_quotes();
            }}
            review={() => open_modal("review")}
            connect={() => open_modal("wallet")}
            error={error}
          />
        )}
      </main>
      {modal === "wallet" && (
        <WalletDialog
          ethereum={ethereum}
          zcash={zcash}
          connect_zcash={() => {
            if (information) void zcash_wallet.connect(information.profile.deployment_id);
          }}
          disconnect_zcash={zcash_wallet.disconnect}
          connect_ethereum={() => {
            void ethereum_wallet.connect();
          }}
          disconnect_ethereum={() => ethereum_wallet.disconnect()}
          switch_network={() => {
            void ethereum_wallet.switch_network();
          }}
          session={session}
          native_available={!!information?.wallet_available}
          connect_native={() => {
            void connect();
          }}
          disconnect_native={() => {
            void disconnect();
          }}
          busy={modal_busy}
          pending_swap={pending}
          error={modal_error}
          received={swap_operation?.receipt?.received.amount ?? null}
          close={() => set_modal(null)}
        />
      )}
      {modal === "review" && selected && (
        <Dialog
          title="Review swap"
          close={() => {
            if (!modal_busy) set_modal(null);
          }}
        >
          <div className="review-route">
            <div>
              <Token asset="ZEC" />
              <strong>{selected.input.amount} ZEC</strong>
              <span>You send</span>
            </div>
            <ArrowRight size={20} />
            <div>
              <Token asset="USDC" />
              <strong>{selected.output.amount} USDC</strong>
              <span>You receive privately</span>
            </div>
          </div>
          <dl className="detail-list">
            <div>
              <dt>Maker fee</dt>
              <dd>
                {selected.maker_fee.amount.amount} {selected.maker_fee.amount.asset}
              </dd>
            </div>
            {selected.fees
              .filter((fee) => fee.paid_by === "user")
              .map((fee, index) => (
                <div key={index}>
                  <dt>{fee.kind === "source_network" ? "Zcash network fee" : "Execution fee"}</dt>
                  <dd>
                    {fee.estimated_amount === null
                      ? `Up to ${fee.max_amount}`
                      : fee.estimated_amount}{" "}
                    {fee.asset}
                  </dd>
                </div>
              ))}
            <div>
              <dt>Quote expires</dt>
              <dd>{Math.max(0, Math.ceil((selected.deadline - now) / 1000))}s</dd>
            </div>
          </dl>
          <p className="review-notice">
            <ShieldCheck size={17} /> USDC is reserved before your ZEC is sent.
          </p>
          <p className="small-print">
            The last run took about {duration(information?.activity[0]?.duration_seconds ?? 9728)}{" "}
            to private USDC. Timing varies. Testnet proving inputs are public; nonpayment refunds
            are unavailable.
          </p>
          {modal_error && (
            <p className="form-error" role="alert">
              {modal_error}
            </p>
          )}
          <button
            className="primary-button"
            disabled={modal_busy || selected.deadline <= now}
            onClick={() => {
              void accept_offer();
            }}
          >
            {modal_busy ? <LoaderCircle size={17} className="spin" /> : null}
            {selected.deadline <= now ? "Quote expired" : "Confirm swap"}
            <ArrowRight size={17} />
          </button>
        </Dialog>
      )}
      {modal === "withdraw" && swap_operation?.receipt && (
        <Dialog
          title="Withdraw USDC"
          close={() => {
            if (!modal_busy) set_modal(null);
          }}
        >
          <div className="withdraw-amount">
            <Token asset="USDC" />
            <span>
              {swap_operation.receipt.received.amount} <small>USDC</small>
            </span>
          </div>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void withdraw();
            }}
          >
            <label className="field-label" htmlFor="withdraw-address">
              Sepolia receiving address
            </label>
            <input
              className="address-input"
              id="withdraw-address"
              type="text"
              autoComplete="off"
              spellCheck={false}
              placeholder="0x…"
              value={to}
              onChange={(event) => set_to(event.target.value)}
              maxLength={42}
              required
            />
            <p className="small-print">
              This withdrawal sends public ERC-20 USDC. Your receiving address and amount will be
              visible.
            </p>
            {modal_error && (
              <p className="form-error" role="alert">
                {modal_error}
              </p>
            )}
            <button className="primary-button" disabled={modal_busy || !to.trim()} type="submit">
              {modal_busy ? <LoaderCircle className="spin" size={17} /> : null}Withdraw USDC{" "}
              <ArrowUpRight size={17} />
            </button>
          </form>
        </Dialog>
      )}
    </div>
  );
}

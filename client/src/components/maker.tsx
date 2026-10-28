import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { ArrowDownLeft, LoaderCircle, Pause, Play, Square } from "lucide-react";
import { maker_start_options } from "@zutti/schema";
import type { MakerControl, MakerSnapshot, MakerStartOptions } from "@zutti/schema";
import { api, error_message } from "../shared/api.js";
import { date } from "../shared/format.js";
import { Dialog } from "./dialog.js";
import type { BrowserFunding, FundingState } from "../shared/funding.js";

const idle_funding: FundingState = {
  stage: "idle",
  amount: null,
  balance: null,
  error: null,
  transaction: null,
};
const empty_subscribe = () => () => {};
const empty_snapshot = () => idle_funding;

const defaults: MakerStartOptions = {
  pairs: ["ZEC/USDC"],
  fee_bps: 50,
  quote_deadline: "15s",
  limits: {
    per_trade: { ZEC: { min: "0.001", max: "0.1" }, USDC: { min: "0.1", max: "10" } },
    max_committed: { ZEC: "0", USDC: "10" },
    max_fee_per_swap: { ZEC: "0", USDC: "0", ETH: "0.005" },
    max_active_swaps: 1,
  },
};
export function Maker({
  connected,
  funding,
}: {
  connected: boolean;
  funding: BrowserFunding | null;
}) {
  const browser = useSyncExternalStore(
    funding?.subscribe ?? empty_subscribe,
    funding?.snapshot ?? empty_snapshot,
  );
  const [snapshot, set_snapshot] = useState<MakerSnapshot | null>(null);
  const [settings, set_settings] = useState<MakerStartOptions>(() => structuredClone(defaults));
  const [busy, set_busy] = useState(false);
  const [error, set_error] = useState<string | null>(null);
  const [amount, set_amount] = useState("");
  const [deposit_review, set_deposit_review] = useState(false);
  const dirty = useRef(false);
  const deposit_id = useRef<string | null>(null);
  function apply(value: MakerSnapshot) {
    set_snapshot(value);
    if (value.settings && !dirty.current) set_settings(structuredClone(value.settings));
  }
  useEffect(() => {
    if (!connected) {
      set_snapshot(null);
      return;
    }
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const value = await api.maker(controller.signal);
        if (controller.signal.aborted) return;
        apply(value);
        set_error(null);
      } catch (reason) {
        if (controller.signal.aborted) return;
        set_snapshot((value) =>
          value
            ? { ...value, inventory: null, trades: null, live_offers: null, active_trades: null }
            : null,
        );
        set_error(error_message(reason));
      }
      timer = setTimeout(() => {
        void poll();
      }, 5000);
    };
    void poll();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [connected]);
  function edit(update: (next: MakerStartOptions) => void) {
    dirty.current = true;
    set_settings((value) => {
      const next = structuredClone(value);
      update(next);
      return next;
    });
  }
  async function control(action: MakerControl["action"]) {
    if (busy) return;
    const parsed = maker_start_options.safeParse(settings);
    if (action === "start" && !parsed.success) {
      set_error("Check your fees and limits.");
      return;
    }
    set_busy(true);
    set_error(null);
    try {
      const value = await api.maker_control(
        action === "start" ? { action, settings: parsed.data! } : { action },
      );
      dirty.current = false;
      apply(value);
    } catch (reason) {
      set_error(error_message(reason));
    } finally {
      set_busy(false);
    }
  }
  async function deposit() {
    if (busy) return;
    if (!connected && funding) {
      await funding.prepare(amount);
      set_deposit_review(false);
      return;
    }
    if (!/^(0|[1-9][0-9]{0,7})(\.[0-9]{1,6})?$/.test(amount) || !/[1-9]/.test(amount)) {
      set_error("Enter a USDC amount greater than zero.");
      return;
    }
    deposit_id.current ??= Array.from(crypto.getRandomValues(new Uint8Array(32)), (value) =>
      value.toString(16).padStart(2, "0"),
    ).join("");
    set_busy(true);
    set_error(null);
    try {
      apply(await api.maker_deposit({ id: deposit_id.current, asset: "USDC", amount }));
      set_deposit_review(false);
    } catch (reason) {
      set_error(error_message(reason));
    } finally {
      set_busy(false);
    }
  }
  const state = snapshot?.state ?? "unavailable";
  const working = busy || !!snapshot?.pending_action;
  const editable = !working && (state === "stopped" || state === "unavailable");
  const enabled = connected && !!snapshot?.available && !working && !error;
  const action = state === "running" ? "pause" : state === "paused" ? "resume" : "start";
  const label = !snapshot?.available
    ? "Maker unavailable"
    : working
      ? "Updating…"
      : state === "running"
        ? "Pause quoting"
        : state === "paused"
          ? "Resume quoting"
          : "Start quoting";
  const browser_pending = !!funding && browser.stage !== "idle" && browser.stage !== "confirmed";
  const funding_pending =
    (!!snapshot?.deposit && snapshot.deposit.state !== "confirmed") || browser_pending;
  const trades = snapshot?.trades;
  return (
    <section className="maker-view" aria-labelledby="maker-title">
      <div className="maker-heading">
        <div>
          <h1 id="maker-title">Maker</h1>
          {connected && (
            <span className="maker-state">
              {working
                ? "Updating"
                : snapshot?.available
                  ? state === "running"
                    ? "Quoting"
                    : state === "paused"
                      ? "Paused"
                      : "Stopped"
                  : "Not configured"}
            </span>
          )}
        </div>
        {connected && (
          <div className="maker-controls">
            {(state === "running" || state === "paused") && (
              <button
                className="secondary-button"
                disabled={working}
                onClick={() => {
                  void control("stop");
                }}
              >
                <Square size={12} /> Stop
              </button>
            )}
            <button
              className="primary-button"
              disabled={!enabled}
              onClick={() => {
                void control(action);
              }}
            >
              {working ? (
                <LoaderCircle className="spin" size={14} />
              ) : state === "running" ? (
                <Pause size={13} />
              ) : (
                <Play size={13} />
              )}
              {label}
            </button>
          </div>
        )}
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {snapshot?.error && (
        <p className="form-error" role="alert">
          {snapshot.error}
        </p>
      )}
      <dl className="maker-stats">
        <div>
          <dt>Available USDC</dt>
          <dd>{snapshot?.inventory?.available_usdc ?? "—"}</dd>
        </div>
        <div>
          <dt>Reserved USDC</dt>
          <dd>{snapshot?.inventory?.reserved_usdc ?? "—"}</dd>
        </div>
        <div>
          <dt>Active trades</dt>
          <dd>{snapshot?.active_trades ?? "—"}</dd>
        </div>
        <div>
          <dt>Live quotes</dt>
          <dd>{snapshot?.live_offers ?? "—"}</dd>
        </div>
      </dl>
      <div className="maker-panels">
        <section className="maker-settings" aria-labelledby="maker-settings-title">
          <div className="maker-section-heading">
            <h2 id="maker-settings-title">Quote settings</h2>
            <span>ZEC → USDC</span>
          </div>
          {!editable && !snapshot?.settings ? (
            <p className="maker-muted">Settings unavailable.</p>
          ) : (
            <fieldset disabled={!editable} className="maker-fields">
              <label>
                Fee (bps)
                <input
                  type="number"
                  min="0"
                  max="9999"
                  step="1"
                  value={Number.isNaN(settings.fee_bps) ? "" : settings.fee_bps}
                  onChange={(event) =>
                    edit((value) => {
                      value.fee_bps = event.target.value === "" ? NaN : Number(event.target.value);
                    })
                  }
                />
              </label>
              <label>
                Quote expiry (s)
                <input
                  type="number"
                  min="1"
                  max="60"
                  step="1"
                  value={settings.quote_deadline.replace(/s$/, "")}
                  onChange={(event) =>
                    edit((value) => {
                      value.quote_deadline = `${event.target.value}s`;
                    })
                  }
                />
              </label>
              <label>
                Min trade (ZEC)
                <input
                  type="text"
                  inputMode="decimal"
                  value={settings.limits.per_trade.ZEC.min}
                  onChange={(event) =>
                    edit((value) => {
                      value.limits.per_trade.ZEC.min = event.target.value;
                    })
                  }
                />
              </label>
              <label>
                Max trade (ZEC)
                <input
                  type="text"
                  inputMode="decimal"
                  value={settings.limits.per_trade.ZEC.max}
                  onChange={(event) =>
                    edit((value) => {
                      value.limits.per_trade.ZEC.max = event.target.value;
                    })
                  }
                />
              </label>
              <details className="maker-limits">
                <summary>Limits</summary>
                <div className="maker-fields">
                  <label>
                    Min output (USDC)
                    <input
                      type="text"
                      inputMode="decimal"
                      value={settings.limits.per_trade.USDC.min}
                      onChange={(event) =>
                        edit((value) => {
                          value.limits.per_trade.USDC.min = event.target.value;
                        })
                      }
                    />
                  </label>
                  <label>
                    Max output (USDC)
                    <input
                      type="text"
                      inputMode="decimal"
                      value={settings.limits.per_trade.USDC.max}
                      onChange={(event) =>
                        edit((value) => {
                          value.limits.per_trade.USDC.max = event.target.value;
                        })
                      }
                    />
                  </label>
                  <label>
                    Inventory cap (USDC)
                    <input
                      type="text"
                      inputMode="decimal"
                      value={settings.limits.max_committed.USDC}
                      onChange={(event) =>
                        edit((value) => {
                          value.limits.max_committed.USDC = event.target.value;
                        })
                      }
                    />
                  </label>
                  <label>
                    Max active trades
                    <input
                      type="number"
                      min="1"
                      max="1000"
                      step="1"
                      value={settings.limits.max_active_swaps}
                      onChange={(event) =>
                        edit((value) => {
                          value.limits.max_active_swaps = Number(event.target.value);
                        })
                      }
                    />
                  </label>
                  {(["USDC", "ETH"] as const).map((asset) => (
                    <label key={asset}>
                      Max fee ({asset})
                      <input
                        type="text"
                        inputMode="decimal"
                        value={settings.limits.max_fee_per_swap[asset]}
                        onChange={(event) =>
                          edit((value) => {
                            value.limits.max_fee_per_swap[asset] = event.target.value;
                          })
                        }
                      />
                    </label>
                  ))}
                </div>
              </details>
            </fieldset>
          )}
          <p className="maker-muted">
            {editable ? "Applies when quoting starts." : "Stop quoting to edit settings."}
          </p>
        </section>
        <section className="maker-funding" aria-labelledby="maker-inventory-title">
          <div className="maker-section-heading">
            <h2 id="maker-inventory-title">Inventory</h2>
            <span>USDC</span>
          </div>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if ((!connected && !funding) || funding_pending || busy) return;
              set_deposit_review(true);
            }}
          >
            <label className="maker-funding-label" htmlFor="maker-deposit">
              Amount
            </label>
            <div className="maker-deposit-row">
              <input
                id="maker-deposit"
                type="text"
                inputMode="decimal"
                placeholder="0.00"
                autoComplete="off"
                value={browser_pending ? (browser.amount ?? amount) : amount}
                disabled={funding_pending || busy}
                onChange={(event) => {
                  set_amount(event.target.value);
                  deposit_id.current = null;
                }}
              />
              <button
                className="secondary-button"
                type="submit"
                disabled={
                  (!(connected && snapshot?.can_deposit) && !funding) ||
                  !amount ||
                  funding_pending ||
                  busy
                }
              >
                <ArrowDownLeft size={14} />
                Deposit
              </button>
            </div>
          </form>
          <p className="maker-muted">
            {funding && !connected
              ? `Wallet: ${browser.balance ?? "—"} USDC`
              : snapshot?.can_deposit
                ? "Funds your maker inventory."
                : "Connect your Ethereum and Zcash wallets to fund inventory."}
          </p>
          {funding && !connected && (
            <>
              <a
                className="text-button"
                href="https://faucet.circle.com/"
                target="_blank"
                rel="noreferrer"
              >
                Get test USDC
              </a>
              {browser.error && (
                <p className="form-error" role="alert">
                  {browser.error}
                </p>
              )}
              {browser.stage !== "idle" && (
                <div className="maker-deposit-status" role="status">
                  {
                    {
                      preparing: "Preparing deposit…",
                      awaiting_proof: "Deposit prepared · awaiting prover authorization",
                      proof_ready: "Deposit proof ready",
                      approving: "Approve USDC in MetaMask",
                      depositing: "Confirm deposit in MetaMask",
                      confirming: "Waiting for Ethereum finality…",
                      confirmed: `${browser.amount} USDC deposited`,
                      inspect: "Inspect MetaMask activity before continuing",
                      idle: "",
                    }[browser.stage]
                  }
                  {browser.stage === "proof_ready" && (
                    <button
                      className="secondary-button"
                      type="button"
                      onClick={() => {
                        void funding.complete();
                      }}
                    >
                      Complete deposit
                    </button>
                  )}
                  {browser.transaction && (
                    <a
                      className="text-button"
                      href={`https://sepolia.etherscan.io/tx/${browser.transaction}`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      View transaction
                    </a>
                  )}
                </div>
              )}
            </>
          )}
          {snapshot?.deposit && (
            <div className="maker-deposit-status" role="status">
              {snapshot.deposit.state === "confirmed"
                ? `${snapshot.deposit.amount} USDC deposited`
                : snapshot.deposit.state === "paused"
                  ? "Deposit pending in your wallet"
                  : "Confirming deposit…"}
            </div>
          )}
        </section>
      </div>
      <section className="maker-trades" aria-labelledby="maker-trades-title">
        <div className="maker-section-heading">
          <h2 id="maker-trades-title">Trades</h2>
        </div>
        <div className="activity-table-wrap">
          <table className="activity-table" aria-label="Maker trades">
            <thead>
              <tr>
                <th scope="col">Swap</th>
                <th scope="col" className="activity-date">
                  Updated
                </th>
                <th scope="col">Status</th>
              </tr>
            </thead>
            <tbody>
              {!trades || trades.items.length === 0 ? (
                <tr>
                  <td colSpan={3} className="activity-empty">
                    {!connected && !funding
                      ? "Connect a maker wallet to see trades."
                      : !trades
                        ? "Trade history unavailable."
                        : trades.complete
                          ? "No trades yet."
                          : "Trade history incomplete."}
                  </td>
                </tr>
              ) : (
                trades.items.map((trade) => (
                  <tr key={trade.id}>
                    <td>
                      {trade.input.amount} {trade.input.asset}{" "}
                      <span className="activity-arrow">→</span> {trade.output.amount}{" "}
                      {trade.output.asset}
                    </td>
                    <td className="activity-date">{date(trade.updated_at)}</td>
                    <td>
                      {
                        {
                          reserved: "Reserved",
                          payment_confirming: "Confirming ZEC",
                          settling: "Settling",
                          settled: "Settled",
                          pending: "Pending",
                        }[trade.state]
                      }
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </section>
      {deposit_review && (
        <Dialog
          title="Fund maker inventory"
          close={() => {
            if (!busy) set_deposit_review(false);
          }}
        >
          <div className="withdraw-amount">
            <span>
              {amount} <small>USDC</small>
            </span>
          </div>
          <p className="dialog-caption">
            {funding && !connected
              ? "Prepare your private USDC deposit in Flask, then confirm in MetaMask after the proof verifies."
              : "Deposit to your maker’s USDC pool on Sepolia. Your wallet will verify the resulting inventory."}
          </p>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <button
            className="primary-button"
            disabled={busy}
            onClick={() => {
              void deposit();
            }}
          >
            {busy && <LoaderCircle size={15} className="spin" />}
            {funding && !connected ? "Prepare deposit" : "Confirm deposit"}
          </button>
        </Dialog>
      )}
    </section>
  );
}

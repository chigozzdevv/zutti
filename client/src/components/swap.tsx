import { ArrowDown, ArrowRight, Check, Clock3, LoaderCircle, RefreshCw } from "lucide-react";
import type { Offer, QuoteResponse } from "@zutti/schema";
import { amount_error, short_id } from "../shared/format.js";
import { Token } from "./token.js";

interface SwapProps {
  amount: string;
  set_amount: (value: string) => void;
  connected: boolean;
  ethereum_connected: boolean;
  loading: boolean;
  ready: boolean;
  now: number;
  quotes: QuoteResponse | null;
  selected: Offer | null;
  select: (id: string) => void;
  request: () => void;
  review: () => void;
  connect: () => void;
  error: string | null;
}
export function Swap(props: SwapProps) {
  const { amount, connected, loading, ready, quotes, selected, now, error } = props;
  const expired = !!selected && selected.deadline <= now;
  const invalid = amount_error(amount);
  const label = loading
    ? "Finding your quotes"
    : !connected
      ? props.ethereum_connected
        ? "Connect Zcash wallet"
        : "Connect wallet"
      : expired
        ? "Refresh quotes"
        : selected
          ? "Review swap"
          : "Get quotes";
  return (
    <section className="swap-view" aria-label="Swap ZEC for USDC">
      <div className="swap-panel">
        <div className="panel-heading">
          <h2>Swap</h2>
        </div>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!ready || loading) return;
            if (!connected) props.connect();
            else if (selected && !expired) props.review();
            else props.request();
          }}
        >
          <div className="amount-box">
            <div className="amount-label">
              <label htmlFor="zec-amount">You send</label>
              <span>Zcash Testnet</span>
            </div>
            <div className="amount-input-row">
              <input
                id="zec-amount"
                aria-label="ZEC amount"
                aria-describedby={error ? "swap-error" : undefined}
                aria-invalid={!!invalid && !!amount}
                type="text"
                inputMode="decimal"
                placeholder="0.00"
                autoComplete="off"
                maxLength={18}
                value={amount}
                onChange={(event) => props.set_amount(event.target.value)}
              />
              <span className="asset-pill">
                <Token asset="ZEC" small />
                <strong>ZEC</strong>
              </span>
            </div>
          </div>
          <div className="direction-divider">
            <span>
              <ArrowDown size={17} />
            </span>
          </div>
          <div className="amount-box receive-box">
            <div className="amount-label">
              <span>You receive</span>
              <span>Ethereum Sepolia</span>
            </div>
            <div className="amount-input-row">
              <output
                className={selected ? "output-amount" : "output-amount empty-output"}
                aria-live="polite"
                aria-label="USDC quote"
              >
                {selected?.output.amount ?? "—"}
              </output>
              <span className="asset-pill">
                <Token asset="USDC" small />
                <strong>USDC</strong>
              </span>
            </div>
          </div>
          {quotes && (
            <div className="quotes-section">
              <div className="quote-heading">
                <span>
                  {quotes.offers.items.length === 0 ? "No offers right now" : "Maker quotes"}
                </span>
                <button
                  type="button"
                  className="text-button"
                  disabled={loading}
                  onClick={props.request}
                  aria-label="Refresh quotes"
                >
                  <RefreshCw size={13} /> Refresh
                </button>
              </div>
              {quotes.offers.items.length === 0 ? (
                <p className="no-quotes">Try another amount or request again in a moment.</p>
              ) : (
                quotes.offers.items.map((offer) => (
                  <button
                    type="button"
                    className={`quote-row${selected?.id === offer.id ? " selected" : ""}`}
                    key={offer.id}
                    onClick={() => props.select(offer.id)}
                    aria-pressed={selected?.id === offer.id}
                  >
                    <span className="quote-maker">
                      <span className="maker-avatar">
                        {offer.maker_id.slice(0, 2).toUpperCase()}
                      </span>
                      <span>Maker {short_id(offer.maker_id)}</span>
                    </span>
                    <strong>
                      {offer.output.amount} <small>USDC</small>
                    </strong>
                    {selected?.id === offer.id ? (
                      <Check size={15} />
                    ) : (
                      <span className="quote-radio" />
                    )}
                  </button>
                ))
              )}
            </div>
          )}
          {selected && (
            <div className="quote-expiry">
              <Clock3 size={12} />
              {expired
                ? "Quote expired"
                : `Quote expires in ${Math.max(0, Math.ceil((selected.deadline - now) / 1000))}s`}
              <span>Maker fee {(selected.maker_fee.bps / 100).toFixed(2)}%</span>
            </div>
          )}
          {error && (
            <p className="form-error" id="swap-error" role="alert">
              {error}
            </p>
          )}
          <button
            type="submit"
            className="primary-button swap-button"
            disabled={!ready || loading || (connected && !!invalid)}
          >
            {loading ? <LoaderCircle size={18} className="spin" /> : null}
            {label}
            {!loading && <ArrowRight size={18} />}
          </button>
        </form>
      </div>
    </section>
  );
}

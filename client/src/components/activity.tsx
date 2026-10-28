import { useState } from "react";
import { ArrowDownLeft, ArrowUpRight, Check, Copy, ExternalLink, Inbox } from "lucide-react";
import type { ActivityItem } from "@zutti/schema";
import { date, duration, route, short_id } from "../shared/format.js";
import { Dialog } from "./dialog.js";
import { Token } from "./token.js";

export function Activity({
  items,
  recorded = false,
  pending = false,
}: {
  items: readonly ActivityItem[];
  recorded?: boolean;
  pending?: boolean;
}) {
  const [selected, set_selected] = useState<ActivityItem | null>(null);
  const [copied, set_copied] = useState(false);
  return (
    <section className="activity-view" aria-labelledby="activity-title">
      <div className="view-heading">
        <div>
          <h1 id="activity-title">Activity</h1>
        </div>
        <span className="small-label">
          {recorded ? "Recorded Testnet swaps" : "Your connected wallet"}
        </span>
      </div>
      {pending && (
        <button className="ongoing-swap" onClick={() => route("/")}>
          Your swap is in progress <ArrowUpRight size={15} />
        </button>
      )}
      <div className="activity-list">
        {items.length === 0 ? (
          <div className="empty-state">
            <Inbox size={26} />
            <h2>Nothing here yet.</h2>
            <p>Completed swaps will appear here.</p>
          </div>
        ) : (
          items.map((item) => (
            <button
              className="activity-row"
              key={item.id}
              onClick={() => {
                set_selected(item);
                set_copied(false);
              }}
            >
              <span className="activity-symbol">
                <ArrowDownLeft size={21} />
              </span>
              <span className="activity-description">
                <strong>
                  {item.input.amount} ZEC <span>→</span> {item.output.amount} USDC
                </strong>
                <span>{date(item.settled_at)}</span>
              </span>
              <span className="activity-state">
                <span className="status-tag">
                  <Check size={12} /> {item.withdrawn_at ? "Withdrawn" : "Settled"}
                </span>
                <span className="row-chevron">
                  <ArrowUpRight size={17} />
                </span>
              </span>
            </button>
          ))
        )}
      </div>
      <p className="activity-caption">
        {recorded ? "Real swaps from the Zutti Testnet runs." : "Your wallet’s completed swaps."}
      </p>
      {selected && (
        <Dialog title="Swap details" close={() => set_selected(null)}>
          <div className="receipt-amount">
            <div className="token-pair">
              <Token asset="ZEC" />
              <Token asset="USDC" />
            </div>
            <h3>
              {selected.input.amount} ZEC <span>→</span> {selected.output.amount} USDC
            </h3>
            <span className="status-tag">
              <Check size={12} /> {selected.withdrawn_at ? "Withdrawn" : "Settled"}
            </span>
          </div>
          <dl className="detail-list">
            <div>
              <dt>Completed</dt>
              <dd>{date(selected.settled_at)}</dd>
            </div>
            {selected.duration_seconds > 0 && (
              <div>
                <dt>Time to private USDC</dt>
                <dd>{duration(selected.duration_seconds)}</dd>
              </div>
            )}
            <div>
              <dt>Network</dt>
              <dd>Zcash Testnet · Sepolia</dd>
            </div>
          </dl>
          <div className="transaction-links">
            <button
              onClick={() => {
                void navigator.clipboard
                  .writeText(selected.source_transaction)
                  .then(() => set_copied(true))
                  .catch(() => set_copied(false));
              }}
            >
              <span>
                Zcash payment <small>{short_id(selected.source_transaction)}</small>
              </span>
              {copied ? <Check size={16} /> : <Copy size={16} />}
            </button>
            <a
              href={`https://sepolia.etherscan.io/tx/${selected.payout_transaction}`}
              target="_blank"
              rel="noreferrer"
            >
              USDC payout <ExternalLink size={16} />
            </a>
            {selected.withdrawal_transaction && (
              <a
                href={`https://sepolia.etherscan.io/tx/${selected.withdrawal_transaction}`}
                target="_blank"
                rel="noreferrer"
              >
                USDC withdrawal <ExternalLink size={16} />
              </a>
            )}
          </div>
        </Dialog>
      )}
    </section>
  );
}

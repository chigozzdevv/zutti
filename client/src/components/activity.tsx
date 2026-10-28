import { useState } from "react";
import { ArrowUpRight, Check, Copy, ExternalLink } from "lucide-react";
import type { ActivityItem } from "@zutti/schema";
import { date, duration, route, short_id } from "../shared/format.js";
import { Dialog } from "./dialog.js";
import { Token } from "./token.js";

export function Activity({
  items,
  pending = false,
}: {
  items: readonly ActivityItem[];
  pending?: boolean;
}) {
  const [selected, set_selected] = useState<ActivityItem | null>(null);
  const [copied, set_copied] = useState(false);
  return (
    <section className="activity-view" aria-labelledby="activity-title">
      <div className="view-heading">
        <h1 id="activity-title">Zutti activities</h1>
      </div>
      {pending && (
        <button className="ongoing-swap" onClick={() => route("/")}>
          Your swap is in progress <ArrowUpRight size={15} />
        </button>
      )}
      <div className="activity-table-wrap">
        <table className="activity-table" aria-label="Zutti activities">
          <thead>
            <tr>
              <th scope="col">Swap</th>
              <th scope="col" className="activity-date">
                Date
              </th>
              <th scope="col">Status</th>
              <th scope="col">
                <span className="sr-only">Details</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {items.length === 0 ? (
              <tr>
                <td colSpan={4} className="activity-empty">
                  No activities yet.
                </td>
              </tr>
            ) : (
              items.map((item) => (
                <tr key={item.id}>
                  <td>
                    <button
                      className="activity-swap"
                      onClick={() => {
                        set_selected(item);
                        set_copied(false);
                      }}
                    >
                      <span>
                        {item.input.amount} ZEC <span className="activity-arrow">→</span>{" "}
                        {item.output.amount} USDC
                      </span>
                      <time
                        className="activity-mobile-date"
                        dateTime={new Date(item.settled_at).toISOString()}
                      >
                        {date(item.settled_at)}
                      </time>
                    </button>
                  </td>
                  <td className="activity-date">
                    <time dateTime={new Date(item.settled_at).toISOString()}>
                      {date(item.settled_at)}
                    </time>
                  </td>
                  <td>
                    <span className="status-tag">
                      <Check size={12} />
                      {item.withdrawn_at ? "Withdrawn" : "Settled"}
                    </span>
                  </td>
                  <td className="activity-action">
                    <button
                      className="icon-button"
                      aria-label={`View ${item.input.amount} ZEC swap details`}
                      onClick={() => {
                        set_selected(item);
                        set_copied(false);
                      }}
                    >
                      <ArrowUpRight size={16} />
                    </button>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
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

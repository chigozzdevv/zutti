import { ArrowUpRight, Check, Circle, Clock3, LoaderCircle, ShieldCheck } from "lucide-react";
import type { AppOperation } from "@zutti/schema";
import { duration, stage_label } from "../shared/format.js";

const stages = [
  { name: "Reserve USDC", values: ["preparing", "awaiting_reservation", "reserved"] },
  {
    name: "Send shielded ZEC",
    values: ["awaiting_authorization", "payment_submitted", "payment_confirming"],
  },
  { name: "Prove payment", values: ["claimable"] },
  { name: "Receive private USDC", values: ["settling", "settled"] },
];
export function Progress({
  operation,
  now,
  on_withdraw,
  new_swap,
}: {
  operation: AppOperation;
  now: number;
  on_withdraw: () => void;
  new_swap: () => void;
}) {
  const latest = operation.events.at(-1);
  const complete = operation.state === "settled" || operation.state === "confirmed";
  const paused = operation.state === "paused";
  const stage = latest?.stage ?? "preparing";
  const current = Math.max(
    0,
    stages.findIndex((item) => item.values.includes(stage)),
  );
  return (
    <div className="progress-card">
      <div className={`progress-emblem${complete ? " is-complete" : ""}`}>
        {complete ? (
          <Check size={31} />
        ) : paused ? (
          <Clock3 size={28} />
        ) : (
          <LoaderCircle className="spin" size={29} />
        )}
      </div>
      <span className="eyebrow">{operation.kind === "swap" ? "YOUR SWAP" : "YOUR WITHDRAWAL"}</span>
      <h2>
        {paused
          ? "Still pending."
          : complete
            ? operation.kind === "swap"
              ? "USDC received"
              : "Withdrawal complete"
            : stage_label(stage)}
      </h2>
      <p className="progress-description">
        {complete
          ? operation.kind === "swap"
            ? `${operation.receipt?.received.amount ?? ""} USDC is in your private balance.`
            : "Your USDC arrived on Sepolia."
          : paused
            ? "Your wallet has saved this operation."
            : "You can leave this tab open. We’ll keep you updated."}
      </p>
      {operation.kind === "swap" && (
        <ol className="progress-steps">
          {stages.map((item, index) => {
            const done = complete || index < current;
            const active = !complete && index === current;
            return (
              <li
                className={done ? "done" : active ? "active" : ""}
                key={item.name}
                aria-current={active ? "step" : undefined}
              >
                {done ? (
                  <Check size={16} />
                ) : active ? (
                  <LoaderCircle size={16} className={paused ? "" : "spin"} />
                ) : (
                  <Circle size={15} />
                )}
                <span>{item.name}</span>
                {active && <span className="step-now">{paused ? "Pending" : "In progress"}</span>}
              </li>
            );
          })}
        </ol>
      )}
      {!complete && (
        <div className="elapsed">
          <Clock3 size={13} /> {duration((now - operation.started_at) / 1000)} elapsed{" "}
          <span>·</span> Finality can take time
        </div>
      )}
      {paused && <p className="inline-notice">{operation.error}</p>}
      {complete && operation.receipt && (
        <button className="primary-button" onClick={on_withdraw}>
          Withdraw USDC <ArrowUpRight size={18} />
        </button>
      )}
      {complete && operation.withdrawal && (
        <a
          className="primary-button"
          href={`https://sepolia.etherscan.io/tx/${operation.withdrawal.evidence.transaction_id}`}
          target="_blank"
          rel="noreferrer"
        >
          View transaction <ArrowUpRight size={17} />
        </a>
      )}
      {complete && (
        <button className="text-button new-swap" onClick={new_swap}>
          Make another swap
        </button>
      )}
      <div className="progress-footnote">
        <ShieldCheck size={13} />{" "}
        {complete ? "Finality independently verified" : "Waiting preserves your operation"}
      </div>
    </div>
  );
}

export function amount_error(value: string): string | null {
  if (!value) return "Enter a ZEC amount.";
  if (!/^(0|[1-9][0-9]{0,7})(\.[0-9]{1,8})?$/.test(value)) return "Use up to 8 decimal places.";
  if (!/[1-9]/.test(value)) return "Enter an amount greater than zero.";
  return null;
}
export function short_id(value: string): string {
  return `${value.slice(0, 6)}…${value.slice(-4)}`;
}
export function duration(seconds: number): string {
  const minutes = Math.max(0, Math.floor(seconds / 60));
  return minutes >= 60 ? `${Math.floor(minutes / 60)}h ${minutes % 60}m` : `${minutes}m`;
}
export function date(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(value);
}
export function route(path: "/" | "/activity"): void {
  if (location.pathname === path) return;
  history.pushState(null, "", path);
  dispatchEvent(new PopStateEvent("popstate"));
}
export function stage_label(stage: string): string {
  const labels: Record<string, string> = {
    preparing: "Preparing your swap",
    awaiting_reservation: "Reserving USDC",
    reserved: "USDC reserved",
    awaiting_authorization: "Checking your payment",
    payment_submitted: "ZEC sent",
    payment_confirming: "Confirming ZEC",
    claimable: "Proving your payment",
    settling: "Settling USDC",
    settled: "USDC received",
    submitted: "Withdrawal submitted",
    confirming: "Confirming withdrawal",
    confirmed: "Withdrawal complete",
    failed: "Needs attention",
  };
  return labels[stage] ?? "Waiting for your wallet";
}

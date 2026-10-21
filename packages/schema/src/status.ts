export type SwapStage =
  | "preparing"
  | "awaiting_reservation"
  | "reserved"
  | "awaiting_authorization"
  | "payment_submitted"
  | "payment_confirming"
  | "claimable"
  | "refund_provable"
  | "settling"
  | "settled"
  | "refunded"
  | "unfunded";
export interface StatusUpdate {
  readonly operation_id: string;
  readonly swap_id: string | null;
  readonly stage: SwapStage | "submitted" | "confirming" | "confirmed" | "failed";
  readonly message: string;
  readonly blockers: readonly (
    "evidence_unavailable" | "action_required" | "service_unavailable" | "source_conflict"
  )[];
  readonly next_action:
    "authorize" | "reconnect" | "claim" | "refund" | "restore_capability" | "wait" | "none";
  readonly sequence: string;
}
export type StatusHandler = (status: StatusUpdate) => void;
export interface OperationOptions {
  wait_ms?: number;
  signal?: AbortSignal;
  on_status?: StatusHandler;
}

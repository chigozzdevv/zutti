export type ErrorCode =
  | "invalid_input"
  | "invalid_message"
  | "invalid_signature"
  | "expired"
  | "replay"
  | "persistence_failure"
  | "service_unavailable"
  | "pricing_unavailable"
  | "deployment_unavailable"
  | "offer_unavailable"
  | "wallet_unavailable"
  | "aborted";
export class ZuttiError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "ZuttiError";
    this.code = code;
  }
}

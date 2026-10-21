import type { FeeAsset, IntentInput } from "@zutti/schema";

export interface AmountResult {
  readonly amount: string;
  readonly units: string;
}
export interface QuoteResult {
  readonly input: string;
  readonly gross: string;
  readonly net: string;
  readonly fee: string;
}
export interface ProtocolCore {
  amount(asset: FeeAsset, amount: string, positive?: boolean): AmountResult;
  compare(asset: FeeAsset, left: string, right: string): -1 | 0 | 1;
  net(asset: FeeAsset, gross: string, fee_bps: number): QuoteResult;
  quote(input: IntentInput, rate: string, fee_bps: number): QuoteResult;
}

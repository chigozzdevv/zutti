import { app_information, app_operation, maker_snapshot, pool_events } from "@zutti/schema";
import type { MakerControl, MakerDepositInput, MakerSnapshot } from "@zutti/schema";
import type { AppInformation, AppOperation, QuoteResponse, WalletSession } from "@zutti/schema";
import type { BrowserConnection } from "@zutti/schema";

export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
async function request<T>(path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method: body === undefined ? "GET" : "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", "x-zutti-request": "1" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    signal: signal
      ? AbortSignal.any([signal, AbortSignal.timeout(20_000)])
      : AbortSignal.timeout(20_000),
  });
  const result = (await response.json()) as T & { error?: string; message?: string };
  if (!response.ok)
    throw new ApiError(
      result.error ?? "service_unavailable",
      result.message ?? "Zutti is unavailable. Try again.",
    );
  return result;
}
export const api = {
  pool: async () => pool_events.parse(await request("/pool/events")),
  browser_request: () => request<BrowserRequest | null>("/browser/request"),
  browser_response: (id: string, result: unknown, error: string | null) =>
    request("/browser/responses/" + id, { result, error }),
  register_browser: (input: BrowserConnection) =>
    request<{ registered: true }>("/browser/connect", input),
  prepare_deposit: (witness: string) => request<BrowserDeposit>("/browser/deposits", { witness }),
  deposit: (id: string, signal?: AbortSignal) =>
    request<BrowserDeposit>(`/browser/deposits/${id}`, undefined, signal),
  maker: async (signal?: AbortSignal): Promise<MakerSnapshot> =>
    maker_snapshot.parse(await request("/maker", undefined, signal)),
  maker_control: async (input: MakerControl): Promise<MakerSnapshot> =>
    maker_snapshot.parse(await request("/maker/control", input)),
  maker_deposit: async (input: MakerDepositInput): Promise<MakerSnapshot> =>
    maker_snapshot.parse(await request("/maker/deposits", input)),
  information: async (signal?: AbortSignal): Promise<AppInformation> =>
    app_information.parse(await request("/app", undefined, signal)),
  wallet: (signal?: AbortSignal) => request<WalletSession>("/wallet", undefined, signal),
  connect: () => request<WalletSession>("/wallet/connect", {}),
  disconnect: () => request<WalletSession>("/wallet/disconnect", {}),
  quote: (amount: string) =>
    request<QuoteResponse>("/intents", { from: "ZEC", to: "USDC", amount }),
  accept: async (id: string, offer_id: string): Promise<AppOperation> =>
    app_operation.parse(await request(`/intents/${id}/accept`, { offer_id })),
  operation: async (id: string, signal?: AbortSignal): Promise<AppOperation> =>
    app_operation.parse(await request(`/operations/${id}`, undefined, signal)),
  operations: async (signal?: AbortSignal): Promise<AppOperation[]> =>
    app_operation.array().parse(await request("/operations", undefined, signal)),
  withdraw: async (id: string, amount: string, to: string): Promise<AppOperation> =>
    app_operation.parse(await request(`/operations/${id}/withdraw`, { asset: "USDC", amount, to })),
};
export interface BrowserRequest {
  id: string;
  method:
    | "zutti_scanSource"
    | "zutti_prepareOrder"
    | "zutti_proveOrder"
    | "zutti_signTransaction"
    | "zutti_pool"
    | "pool_transaction";
  params: Record<string, unknown>;
}
export interface BrowserDeposit {
  id: string;
  witness: string;
  state: "awaiting_proof" | "proof_ready";
  receipt: string | null;
}
export const error_message = (error: unknown): string =>
  error instanceof ApiError
    ? error.message
    : "Couldn’t reach Zutti. Check your connection and try again.";

import { app_information, app_operation } from "@zutti/schema";
import type { AppInformation, AppOperation, QuoteResponse, WalletSession } from "@zutti/schema";

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
export const error_message = (error: unknown): string =>
  error instanceof ApiError
    ? error.message
    : "Couldn’t reach Zutti. Check your connection and try again.";

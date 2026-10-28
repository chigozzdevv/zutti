import {
  ZuttiError,
  digest,
  network_profile,
  settled_receipt,
  withdrawal_input,
  withdrawal_receipt,
} from "@zutti/schema";
import type {
  IntentInput,
  Network,
  NetworkProfile,
  OfferCollection,
  OperationOptions,
  WireOffer,
  SettledReceipt,
  SettledSwap,
  WithdrawalInput,
  WithdrawalReceipt,
} from "@zutti/schema";
import { canonical, immutable, random_id } from "@zutti/runtime";
import type { RfqClient } from "@zutti/runtime";

export type {
  IntentInput,
  Offer,
  OfferCollection,
  StatusUpdate,
  StatusHandler,
  OperationOptions,
  NetworkProfile,
  SettledReceipt,
  SettledSwap,
  WithdrawalInput,
  WithdrawalReceipt,
} from "@zutti/schema";
export { ZuttiError } from "@zutti/schema";
export interface WalletProvider {
  zutti(network: Network): {
    profile: NetworkProfile;
    rfq?: RfqClient;
    execution?: {
      accept(offer: Readonly<WireOffer>, options: OperationOptions): Promise<SettledReceipt>;
    };
    account?: {
      withdraw(
        input: Readonly<WithdrawalInput>,
        options: OperationOptions,
      ): Promise<WithdrawalReceipt>;
    };
  };
}
export interface ClientConfig {
  network: Network;
  wallet: WalletProvider;
}
export interface Intent {
  readonly id: string;
  readonly request: Readonly<IntentInput>;
  offers(options?: { signal?: AbortSignal }): Promise<OfferCollection>;
  accept(input: OperationOptions & { offer_id: string }): Promise<SettledSwap>;
}
export interface Zutti {
  readonly network: Readonly<NetworkProfile>;
  intent(input: IntentInput): Intent;
  account(): PrivateAccount;
  close(): Promise<void>;
}
export interface PrivateAccount {
  withdraw(input: WithdrawalInput & OperationOptions): Promise<Readonly<WithdrawalReceipt>>;
}
export function create_zutti(config: ClientConfig): Zutti {
  if (!["testnet", "mainnet"].includes(config.network))
    throw new ZuttiError("invalid_input", "Unsupported network");
  if (!config.wallet || typeof config.wallet.zutti !== "function")
    throw new ZuttiError(
      "wallet_unavailable",
      "Connected wallet does not provide Zutti capabilities",
    );
  const session = config.wallet.zutti(config.network);
  const profile = immutable(network_profile.parse(session.profile));
  if (
    profile.name !== config.network ||
    (session.rfq &&
      (profile.deployment_id !== session.rfq.domain.deployment_id ||
        canonical(profile.policy) !== canonical(session.rfq.domain.policy) ||
        session.rfq.domain.network !== config.network))
  )
    throw new ZuttiError("invalid_input", "Wallet and SDK network profiles do not match");
  const controller = new AbortController();
  return Object.freeze({
    network: profile,
    account(): PrivateAccount {
      return Object.freeze({
        async withdraw(input: WithdrawalInput & OperationOptions) {
          if (controller.signal.aborted || input.signal?.aborted)
            throw new ZuttiError("aborted", "Withdrawal was interrupted");
          const { on_status, signal, wait_ms, ...terms } = input;
          const request = immutable(withdrawal_input.parse(terms));
          if (!session.account)
            throw new ZuttiError(
              "wallet_unavailable",
              "Wallet does not provide private USDC withdrawals",
            );
          const receipt = withdrawal_receipt.parse(
            await session.account.withdraw(request, {
              ...(on_status ? { on_status } : {}),
              ...(wait_ms === undefined ? {} : { wait_ms }),
              signal: signal ? AbortSignal.any([signal, controller.signal]) : controller.signal,
            }),
          );
          if (
            receipt.amount.amount !== request.amount ||
            receipt.to.toLowerCase() !== request.to.toLowerCase() ||
            receipt.evidence.chain !== "ethereum" ||
            receipt.evidence.policy_id !== profile.policy.source_policy_id ||
            receipt.confirmed_at > Date.now() + 300_000
          )
            throw new ZuttiError(
              "invalid_message",
              "Wallet withdrawal does not match the authorized terms",
            );
          return immutable(receipt);
        },
      });
    },
    intent(input: IntentInput): Intent {
      const rfq = session.rfq;
      if (!rfq)
        throw new ZuttiError(
          "wallet_unavailable",
          "Wallet does not provide encrypted RFQ capabilities",
        );
      if (controller.signal.aborted) throw new ZuttiError("wallet_unavailable", "SDK is closed");
      const id = random_id();
      const request = rfq.normalize(input);
      return Object.freeze({
        id,
        request,
        offers(options?: { signal?: AbortSignal }) {
          return rfq.offers(
            id,
            request,
            options?.signal
              ? AbortSignal.any([options.signal, controller.signal])
              : controller.signal,
          );
        },
        async accept(options: OperationOptions & { offer_id: string }): Promise<SettledSwap> {
          digest.parse(options.offer_id);
          if (controller.signal.aborted || options.signal?.aborted)
            throw new ZuttiError("aborted", "Acceptance was interrupted");
          const offer = await rfq.select(id, request, options.offer_id);
          if (!session.execution)
            throw new ZuttiError(
              "deployment_unavailable",
              "Settlement requires an admitted deployment and the native wallet execution adapter; no payment was authorized",
            );
          const receipt = settled_receipt.parse(
            await session.execution.accept(immutable(offer), {
              ...options,
              signal: options.signal
                ? AbortSignal.any([options.signal, controller.signal])
                : controller.signal,
            }),
          );
          if (
            receipt.received.asset !== offer.output.asset ||
            receipt.received.amount !== offer.output.amount ||
            receipt.source_payment.policy_id !== profile.policy.source_policy_id ||
            receipt.pool_settlement.policy_id !== profile.policy.source_policy_id ||
            receipt.source_payment.chain !== "zcash" ||
            receipt.pool_settlement.chain !== "ethereum" ||
            receipt.confirmed_at > Date.now() + 300_000
          )
            throw new ZuttiError(
              "invalid_message",
              "Wallet settlement does not match the accepted offer",
            );
          const fixed = immutable(receipt);
          return Object.freeze({
            id: fixed.swap_id,
            receipt: fixed,
            summary: async () =>
              Object.freeze({ id: fixed.swap_id, stage: "settled" as const, receipt: fixed }),
          });
        },
      });
    },
    async close() {
      controller.abort();
    },
  });
}

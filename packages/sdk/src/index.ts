import { ZuttiError, digest, network_profile } from "@zutti/schema";
import type {
  IntentInput,
  Network,
  NetworkProfile,
  OfferCollection,
  OperationOptions,
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
} from "@zutti/schema";
export { ZuttiError } from "@zutti/schema";
export interface WalletProvider {
  zutti(network: Network): { profile: NetworkProfile; rfq: RfqClient };
}
export interface ClientConfig {
  network: Network;
  wallet: WalletProvider;
}
export interface Intent {
  readonly id: string;
  readonly request: Readonly<IntentInput>;
  offers(options?: { signal?: AbortSignal }): Promise<OfferCollection>;
  accept(input: OperationOptions & { offer_id: string }): Promise<never>;
}
export interface Zutti {
  readonly network: Readonly<NetworkProfile>;
  intent(input: IntentInput): Intent;
  close(): Promise<void>;
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
    profile.deployment_id !== session.rfq.domain.deployment_id ||
    canonical(profile.policy) !== canonical(session.rfq.domain.policy) ||
    session.rfq.domain.network !== config.network
  )
    throw new ZuttiError("invalid_input", "Wallet and SDK network profiles do not match");
  const controller = new AbortController();
  return Object.freeze({
    network: profile,
    intent(input: IntentInput): Intent {
      if (controller.signal.aborted) throw new ZuttiError("wallet_unavailable", "SDK is closed");
      const id = random_id();
      const request = session.rfq.normalize(input);
      return Object.freeze({
        id,
        request,
        offers(options?: { signal?: AbortSignal }) {
          return session.rfq.offers(
            id,
            request,
            options?.signal
              ? AbortSignal.any([options.signal, controller.signal])
              : controller.signal,
          );
        },
        async accept(options: OperationOptions & { offer_id: string }): Promise<never> {
          digest.parse(options.offer_id);
          if (controller.signal.aborted || options.signal?.aborted)
            throw new ZuttiError("aborted", "Acceptance was interrupted");
          await session.rfq.select(id, request, options.offer_id);
          throw new ZuttiError(
            "deployment_unavailable",
            "Settlement requires an admitted deployment and the native wallet execution adapter; no payment was authorized",
          );
        },
      });
    },
    async close() {
      controller.abort();
    },
  });
}

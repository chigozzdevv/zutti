import { ZuttiError, network_profile } from "@zutti/schema";
import type { MakerStartOptions, Network, NetworkProfile } from "@zutti/schema";
import { canonical, immutable } from "@zutti/runtime";
import type { RfqMaker } from "@zutti/runtime";

export type { MakerStartOptions, NetworkProfile } from "@zutti/schema";
export interface MakerWalletProvider {
  zutti_maker(network: Network): { profile: NetworkProfile; maker: RfqMaker };
}
export interface MakerConfig {
  network: Network;
  wallet: MakerWalletProvider;
}
export function create_zutti_maker(config: MakerConfig) {
  if (!["testnet", "mainnet"].includes(config.network))
    throw new ZuttiError("invalid_input", "Unsupported network");
  if (!config.wallet || typeof config.wallet.zutti_maker !== "function")
    throw new ZuttiError(
      "wallet_unavailable",
      "Connected wallet does not provide Zutti capabilities",
    );
  const session = config.wallet.zutti_maker(config.network);
  const profile = immutable(network_profile.parse(session.profile));
  if (
    profile.name !== config.network ||
    session.maker.domain.network !== config.network ||
    profile.deployment_id !== session.maker.domain.deployment_id ||
    canonical(profile.policy) !== canonical(session.maker.domain.policy)
  )
    throw new ZuttiError("invalid_input", "Maker wallet and SDK network profiles do not match");
  return Object.freeze({
    network: profile,
    start: (options: MakerStartOptions) => session.maker.start(options),
    pause: () => session.maker.pause(),
    resume: () => session.maker.resume(),
    stop: () => session.maker.stop(),
    summary: () => session.maker.summary(),
  });
}

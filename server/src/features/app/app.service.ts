import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { activity_item, app_information } from "@zutti/schema";
import type { ActivityItem, AppInformation } from "@zutti/schema";

export async function load_app_information(
  repository: string,
  wallet_available = false,
): Promise<AppInformation> {
  const read = async (path: string): Promise<Record<string, unknown>> =>
    JSON.parse(await readFile(resolve(repository, path), "utf8")) as Record<string, unknown>;
  const deployment = await read("deploy/ethereum/optimized/sepolia.json");
  const source = await read("deploy/ethereum/optimized/source-profile.json");
  const encoded = Buffer.from(String(deployment.domain_encoding), "hex");
  const payment_profile_id = encoded.subarray(-96, -64).toString("hex");
  const checkpoint = source.checkpoint as Record<string, unknown>;
  const policy = source.policy as Record<string, unknown>;
  const activity: ActivityItem[] = [];
  for (const path of [
    "deploy/ethereum/optimized/swap-proof.json",
    "deploy/ethereum/swap-proof.json",
  ]) {
    const evidence = await read(path);
    if (!evidence.sdk_swap_settled || !evidence.private_payout_ownership_verified) continue;
    const metrics = evidence.metrics as Record<string, number> | undefined;
    const payment = evidence.source_payment as Record<string, unknown>;
    const payout = evidence.pool_settlement as Record<string, unknown>;
    const withdrawal = evidence.withdrawal as Record<string, unknown> | undefined;
    const withdrawal_evidence = withdrawal?.evidence as Record<string, unknown> | undefined;
    const statuses = evidence.statuses as Array<Record<string, unknown>> | undefined;
    const settled_at =
      metrics?.private_payout_finality_verified_at_ms ??
      Number(statuses?.find((item) => item.stage === "settled")?.saved_at_ms);
    if (!Number.isSafeInteger(settled_at)) continue;
    activity.push(
      activity_item.parse({
        id: evidence.swap_id,
        input: evidence.input,
        output: evidence.output,
        settled_at,
        withdrawn_at: withdrawal?.confirmed_at ?? null,
        source_transaction: payment.transaction_id,
        payout_transaction: payout.transaction_id,
        withdrawal_transaction: withdrawal_evidence?.transaction_id ?? null,
        duration_seconds: metrics?.accept_to_private_payout_seconds ?? 0,
      }),
    );
  }
  return app_information.parse({
    profile: {
      name: "testnet",
      deployment_id: deployment.domain,
      zcash: { genesis_hash: encoded.subarray(10, 42).toString("hex"), payment_profile_id },
      ethereum: {
        chain_id: "11155111",
        usdc: deployment.token,
        pool: deployment.pool,
        registry: deployment.registry,
      },
      policy: {
        payment_profile_id,
        source_policy_id: deployment.source_policy_id,
        proof_profile_id: deployment.proof_profile_id,
        ethereum_finality: "finalized",
        min_source_window_blocks: source.minimum_remaining,
        max_source_window_blocks: policy.window_blocks,
        paid_claim_deadline: null,
        source_checkpoint: {
          commitment: checkpoint.commitment,
          height: checkpoint.height,
          hash: checkpoint.hash,
          program_id: deployment.source_image,
        },
      },
    },
    activity,
    wallet_available,
    witnesses: "public-testnet",
    refunds: false,
  });
}

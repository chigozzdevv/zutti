import { parseArgs } from "node:util";
import { readFile, writeFile, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";
import { createWriteStream } from "node:fs";
import { create_zutti } from "../packages/sdk/dist/index.js";

const { values } = parseArgs({
  options: Object.fromEntries(
    ["config", "operation", "amount", "to"].map((name) => [name, { type: "string" }]),
  ),
});
for (const name of ["config", "operation", "amount", "to"])
  if (!values[name]) throw new Error(`Missing --${name}`);
const config_path = resolve(values.config);
if ((await stat(config_path)).mode & 0o077) throw new Error("Private owner configuration required");
const config = JSON.parse(await readFile(config_path, "utf8"));
const deployment = JSON.parse(await readFile(config.deployment, "utf8"));
const source = JSON.parse(await readFile(join(config.root, "profile.json"), "utf8"));
const operation = resolve(values.operation);
const encoded = Buffer.from(deployment.domain_encoding, "hex");
const policy = {
  payment_profile_id: encoded.subarray(-96, -64).toString("hex"),
  source_policy_id: deployment.source_policy_id,
  proof_profile_id: deployment.proof_profile_id,
  ethereum_finality: "finalized",
  min_source_window_blocks: source.minimum_remaining,
  max_source_window_blocks: source.policy.window_blocks,
  paid_claim_deadline: null,
  source_checkpoint: {
    commitment: source.checkpoint.commitment,
    height: source.checkpoint.height,
    hash: source.checkpoint.hash,
    program_id: deployment.source_image,
  },
};
const profile = {
  name: "testnet",
  deployment_id: deployment.domain,
  zcash: {
    genesis_hash: encoded.subarray(10, 42).toString("hex"),
    payment_profile_id: policy.payment_profile_id,
  },
  ethereum: {
    chain_id: "11155111",
    usdc: deployment.token,
    pool: deployment.pool,
    registry: deployment.registry,
  },
  policy,
};
const account = {
  withdraw: async (terms, options) => {
    const request = join(operation, "withdrawal-input.json");
    try {
      await writeFile(request, JSON.stringify(terms), { flag: "wx", mode: 0o600 });
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      if (JSON.stringify(JSON.parse(await readFile(request, "utf8"))) !== JSON.stringify(terms))
        throw new Error("Existing withdrawal has different authorized terms");
    }
    return new Promise((resolve_receipt, reject) => {
      const child = spawn(
        "python3",
        [
          resolve("scripts/settle_testnet.py"),
          "--config",
          config_path,
          "--operation",
          operation,
          "--withdraw",
          request,
        ],
        { stdio: ["ignore", "pipe", "pipe"] },
      );
      child.stderr.pipe(
        createWriteStream(join(operation, "withdrawal-worker.stderr"), { flags: "a", mode: 0o600 }),
      );
      let buffered = "";
      let receipt;
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk) => {
        buffered += chunk;
        if (buffered.length > 131_072) {
          reject(new Error("Native response exceeds its bound"));
          return;
        }
        for (;;) {
          const end = buffered.indexOf("\n");
          if (end < 0) break;
          const line = buffered.slice(0, end);
          buffered = buffered.slice(end + 1);
          try {
            const message = JSON.parse(line);
            if (message.type === "receipt") receipt = message.value;
            else if (message.type === "status") {
              try {
                options.on_status?.(message.value);
              } catch {}
            }
          } catch {
            reject(new Error("Invalid native withdrawal response"));
          }
        }
      });
      child.on("error", reject);
      child.on("close", (code) =>
        code === 0 && receipt
          ? resolve_receipt(receipt)
          : reject(new Error("Withdrawal remains pending; inspect its retained operation")),
      );
    });
  },
};
const zutti = create_zutti({ network: "testnet", wallet: { zutti: () => ({ profile, account }) } });
try {
  const receipt = await zutti
    .account()
    .withdraw({
      asset: "USDC",
      amount: values.amount,
      to: values.to,
      on_status: (status) => console.log(JSON.stringify({ status })),
    });
  console.log(JSON.stringify({ sdk_withdrawal_confirmed: true, receipt }));
} finally {
  await zutti.close();
}

import { parseArgs } from "node:util";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, join } from "node:path";
import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { create_native_core } from "../packages/bindings/dist/native.js";
import { EncryptedStore, create_native_execution } from "../packages/runtime/dist/native.js";
import {
  MessagingIdentity,
  RfqClient,
  RfqMaker,
  HttpRelay,
} from "../packages/runtime/dist/index.js";
import { create_relay_server } from "../server/dist/app.js";
import { create_zutti } from "../packages/sdk/dist/index.js";

const { values } = parseArgs({
  options: Object.fromEntries(
    [
      "deployment",
      "source-profile",
      "maker",
      "inventory",
      "taker",
      "notes",
      "anchor",
      "out",
      "wallet",
      "core",
      "amount",
      "rate",
      "owner-config",
    ].map((name) => [name, { type: "string" }]),
  ),
});
for (const name of [
  "deployment",
  "source-profile",
  "maker",
  "inventory",
  "taker",
  "notes",
  "anchor",
  "out",
  "wallet",
  "core",
  "amount",
  "rate",
  "owner-config",
])
  if (!values[name]) throw new Error(`Missing --${name}`);
const deployment = JSON.parse(await readFile(values.deployment, "utf8"));
const source = JSON.parse(await readFile(values["source-profile"], "utf8"));
const maker_account = JSON.parse(await readFile(join(values.maker, "public.json"), "utf8"));
if (deployment.network !== "testnet" || deployment.chain_id !== 11155111 || !source.checkpoint)
  throw new Error("Checkpoint Testnet deployment required");
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
const domain = { network: "testnet", deployment_id: deployment.domain, policy };
const out = resolve(values.out);
await mkdir(out, { mode: 0o700, recursive: true });
const owner_config = JSON.parse(await readFile(values["owner-config"], "utf8"));
for (const [argument, field] of [
  ["deployment", "deployment"],
  ["maker", "maker"],
  ["taker", "taker"],
])
  if (resolve(values[argument]) !== resolve(owner_config[field]))
    throw new Error("Quote inputs differ from the owner configuration");
const owner_execution = create_native_execution({
  directory: out,
  command: execFileSync("which", ["python3"], { encoding: "utf8" }).trim(),
  arguments: [
    resolve("scripts/settle_testnet.py"),
    "--config",
    resolve(values["owner-config"]),
    "--operation",
    out,
    "--notes",
    resolve(values.notes),
    "--anchor",
    resolve(values.anchor),
    "--ready",
    join(out, "readiness"),
    "--attach",
  ],
});
let retained_offer;
try {
  retained_offer = JSON.parse(await readFile(join(out, "selected-offer.json"), "utf8"));
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (retained_offer) {
  const receipt = await owner_execution.accept(retained_offer, {
    on_status: (value) => console.log(JSON.stringify({ status: value })),
  });
  console.log(JSON.stringify({ owner_operation_continued: true, receipt }));
  process.exit(0);
}
const readiness = JSON.parse(
  execFileSync(
    "python3",
    [
      resolve("scripts/ready_testnet.py"),
      "--config",
      resolve(values["owner-config"]),
      "--out",
      join(out, "readiness"),
    ],
    { encoding: "utf8", timeout: 180_000, maxBuffer: 65_536 },
  ),
);
if (readiness.verified.domain !== deployment.domain)
  throw new Error("Readiness belongs to another deployment");
const ready_at = performance.now();
const raw = randomBytes(32);
await writeFile(join(out, "messaging.key"), raw, { mode: 0o600, flag: "wx" });
const key = await crypto.subtle.importKey("raw", raw, { name: "AES-GCM" }, false, [
  "encrypt",
  "decrypt",
]);
raw.fill(0);
const store = (name) => new EncryptedStore(join(out, name), key, deployment.domain);
const core = create_native_core(resolve(values.core));
const server = create_relay_server(domain);
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const relay = new HttpRelay(`http://127.0.0.1:${server.address().port}`);
const identity = await MessagingIdentity.create();
const maker_store = store("maker");
await maker_store.create("identity", identity.export());
const rpc = async (method, params) => {
  const response = await fetch("https://ethereum-sepolia-rpc.publicnode.com", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error("Sepolia quote preflight unavailable");
  const value = await response.json();
  if (value.error) throw new Error("Sepolia quote preflight failed");
  return value.result;
};
if ((await rpc("eth_chainId", [])) !== "0xaa36a7") throw new Error("Sepolia RPC required");
const backing = await rpc("eth_call", [
  {
    to: deployment.pool,
    data: execFileSync("cast", ["sig", "backing()"], { encoding: "utf8" }).trim(),
  },
  "latest",
]);
const inventory = JSON.parse(await readFile(values.inventory, "utf8"));
const units = BigInt(readiness.verified.amount_units);
if (
  !inventory.notes.some(
    (note) =>
      note.position === readiness.verified.note_position && BigInt(note.amount_units) === units,
  )
)
  throw new Error("Quote inventory differs from verified maker ownership");
if (units === 0n || units > BigInt(backing)) throw new Error("Maker inventory is unavailable");
const format = (units, scale) =>
  `${units / scale}.${(units % scale).toString().padStart(scale.toString().length - 1, "0")}`;
const available = format(units, 1_000_000n);
const gas = format(
  BigInt(await rpc("eth_getBalance", [deployment.deployer, "latest"])),
  10n ** 18n,
);
const maker = new RfqMaker({
  domain,
  core,
  identity,
  store: maker_store,
  relay,
  preflight: async () => {
    if (BigInt(backing) === 0n || performance.now() - ready_at > 60_000)
      throw new Error("Refresh verified source and inventory evidence before quoting");
  },
  receiving: async () => ({
    zcash: maker_account.zcash_address,
    usdc: maker_account.usdc_receiver,
  }),
  price: async () => ({
    rate: values.rate,
    observed_at: Date.now(),
    valid_until: Date.now() + 15_000,
  }),
  capacity: async () => ({ ZEC: "0", USDC: available, ETH: gas }),
  fees: async () => [
    {
      kind: "source_network",
      asset: "ZEC",
      paid_by: "user",
      pay_from: "public_wallet",
      max_amount: "0.001",
      estimated_amount: null,
    },
    {
      kind: "ethereum_execution",
      asset: "ETH",
      paid_by: "maker",
      pay_from: "maker_inventory",
      max_amount: "0.005",
      estimated_amount: null,
    },
  ],
});
try {
  await maker.start({
    pairs: ["ZEC/USDC"],
    fee_bps: 0,
    quote_deadline: "15s",
    limits: {
      per_trade: { ZEC: { min: "0.001", max: "0.1" }, USDC: { min: "0.1", max: available } },
      max_committed: { ZEC: "0", USDC: available },
      max_fee_per_swap: { ZEC: "0.001", USDC: "0", ETH: "0.005" },
      max_active_swaps: 1,
    },
  });
  const client = new RfqClient({
    domain,
    core,
    store: store("taker"),
    relay,
    maker_ids: [identity.id],
    collection_ms: 5000,
  });
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
  const execution = owner_execution;
  const zutti = create_zutti({
    network: "testnet",
    wallet: { zutti: () => ({ profile, rfq: client, execution }) },
  });
  const intent = zutti.intent({ from: "ZEC", to: "USDC", amount: values.amount });
  const offers = await intent.offers();
  if (offers.items.length !== 1) throw new Error("No authenticated maker quote received");
  console.log(JSON.stringify({ authenticated_rfq: true, offer: offers.items[0] }));
  const swap = await intent.accept({
    offer_id: offers.items[0].id,
    on_status: (value) => console.log(JSON.stringify({ status: value })),
  });
  console.log(JSON.stringify({ sdk_swap_settled: true, swap: swap.id, receipt: swap.receipt }));
  await zutti.close();
} finally {
  await maker.stop();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}

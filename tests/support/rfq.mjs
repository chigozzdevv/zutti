import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { create_native_core } from "../../packages/bindings/dist/native.js";
import { EncryptedStore } from "../../packages/runtime/dist/native.js";
import {
  MessagingIdentity,
  RfqClient,
  RfqMaker,
  HttpRelay,
} from "../../packages/runtime/dist/index.js";
import { create_relay_server } from "../../server/dist/app.js";
import { create_zutti } from "../../packages/sdk/dist/index.js";
import { create_zutti_maker } from "../../packages/maker-sdk/dist/index.js";

import { domain, profile } from "./rfq-domain.mjs";
export { domain, profile, options } from "./rfq-domain.mjs";
export async function setup({ checkpoint } = {}) {
  const active_domain = checkpoint
    ? { ...domain, policy: { ...domain.policy, source_checkpoint: checkpoint } }
    : domain;
  const active_profile = { ...profile, policy: active_domain.policy };
  const root = await mkdtemp(join(tmpdir(), "zutti-rfq-integration-"));
  const server = create_relay_server(active_domain);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const relay = new HttpRelay(`http://127.0.0.1:${server.address().port}`);
  const core = create_native_core(process.env.ZUTTI_CORE_BINARY);
  const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
    "encrypt",
    "decrypt",
  ]);
  const store = (name) => new EncryptedStore(join(root, name), key, active_domain.deployment_id);
  const client_store = store("client");
  const maker_identity = await MessagingIdentity.create();
  const maker_store = store("maker");
  const issued = [];
  let price_calls = 0;
  const maker_config = {
    domain: active_domain,
    core,
    identity: maker_identity,
    store: maker_store,
    relay: {
      makers: (...args) => relay.makers(...args),
      announce: (...args) => relay.announce(...args),
      poll: (...args) => relay.poll(...args),
      send: async (message, signal) => {
        issued.push(message);
        await relay.send(message, signal);
      },
    },
    preflight: async () => {},
    price: async () => {
      price_calls++;
      const now = Date.now();
      return { rate: "40", observed_at: now, valid_until: now + 15_000 };
    },
    capacity: async () => ({ ZEC: "10", USDC: "5000", ETH: "1" }),
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
  };
  const maker_runtime = new RfqMaker(maker_config);
  const maker = create_zutti_maker({
    network: "testnet",
    wallet: { zutti_maker: () => ({ profile: active_profile, maker: maker_runtime }) },
  });
  const client_config = {
    domain: active_domain,
    core,
    store: client_store,
    relay,
    maker_ids: [maker_identity.id],
    collection_ms: 1500,
  };
  const make_client = (config = client_config) =>
    create_zutti({
      network: "testnet",
      wallet: { zutti: () => ({ profile: active_profile, rfq: new RfqClient(config) }) },
    });
  return {
    root,
    server,
    relay,
    core,
    key,
    store,
    client_store,
    maker_identity,
    maker_store,
    maker_config,
    maker_runtime,
    maker,
    client_config,
    make_client,
    issued,
    price_calls: () => price_calls,
    close: async () => {
      await maker.stop();
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
      await rm(root, { recursive: true, force: true });
    },
  };
}

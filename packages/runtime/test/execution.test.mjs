import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { create_native_execution } from "../dist/native.js";
import { RfqClient, random_id } from "../dist/index.js";
import { setup, options } from "../../../tests/support/rfq.mjs";

async function operation() {
  const fixture = await setup();
  await fixture.maker.start(options);
  const client = new RfqClient(fixture.client_config);
  const intent = random_id();
  const request = { from: "ZEC", to: "USDC", amount: "5" };
  const collection = await client.offers(intent, request);
  const offer = await client.select(intent, request, collection.items[0].id);
  const directory = join(fixture.root, "operation");
  await mkdir(directory, { mode: 0o700 });
  const reference = {
    chain: "ethereum",
    block_hash: "11".repeat(32),
    height: "12000000",
    transaction_id: "22".repeat(32),
    policy_id: offer.policy.source_policy_id,
  };
  const receipt = {
    outcome: "settled",
    swap_id: "33".repeat(32),
    received: offer.output,
    source_payment: { ...reference, chain: "zcash" },
    pool_settlement: reference,
    confirmed_at: Date.now(),
  };
  const worker = join(fixture.root, "worker.mjs");
  await writeFile(
    worker,
    `
    import { appendFile, readFile, rename, writeFile } from "node:fs/promises";
    import { join } from "node:path";
    const directory = process.argv[2];
    await appendFile(join(directory, "starts"), "started\\n", { mode: 0o600 });
    const offer = JSON.parse(await readFile(join(directory, "selected-offer.json"), "utf8"));
    await writeFile(join(directory, ".status"), JSON.stringify({
      operation_id: offer.intent_id, swap_id: null, sequence: "1", stage: "preparing",
      message: "Checking saved operation", blockers: [], next_action: "wait",
    }), { mode: 0o600 });
    await rename(join(directory, ".status"), join(directory, "status-0001.json"));
    await new Promise((resolve) => setTimeout(resolve, 150));
    await writeFile(join(directory, ".receipt"), ${JSON.stringify(JSON.stringify(receipt))}, { mode: 0o600 });
    await rename(join(directory, ".receipt"), join(directory, "settled-receipt.json"));
  `,
    { mode: 0o600 },
  );
  const config = {
    directory,
    command: process.execPath,
    arguments: [worker, directory],
    poll_ms: 10,
  };
  return { ...fixture, offer, receipt, directory, config };
}

test("concurrent acceptance shares one owner process and retains exact settlement terms", async () => {
  const fixture = await operation();
  try {
    const execution = create_native_execution(fixture.config);
    let seen = 0;
    const [first, second] = await Promise.all([
      execution.accept(fixture.offer, {
        on_status: () => {
          seen++;
          throw new Error("render");
        },
      }),
      execution.accept(fixture.offer),
    ]);
    assert.deepEqual(first, fixture.receipt);
    assert.deepEqual(second, fixture.receipt);
    assert.ok(Object.isFrozen(first));
    assert.equal(seen, 1);
    assert.equal(await readFile(join(fixture.directory, "starts"), "utf8"), "started\n");
  } finally {
    await fixture.close();
  }
});

test("reattaching a completed operation returns its receipt without another process", async () => {
  const fixture = await operation();
  try {
    await create_native_execution(fixture.config).accept(fixture.offer);
    const result = await create_native_execution(fixture.config).accept(fixture.offer);
    assert.deepEqual(result, fixture.receipt);
    assert.equal(await readFile(join(fixture.directory, "starts"), "utf8"), "started\n");
    await assert.rejects(
      create_native_execution(fixture.config).accept({ ...fixture.offer, id: "ff".repeat(32) }),
      { code: "persistence_failure" },
    );
  } finally {
    await fixture.close();
  }
});

test("interrupting a caller stops waiting while the same authorized operation continues", async () => {
  const fixture = await operation();
  try {
    const execution = create_native_execution(fixture.config);
    const controller = new AbortController();
    await assert.rejects(
      execution.accept(fixture.offer, {
        signal: controller.signal,
        on_status: () => controller.abort(),
      }),
      { code: "aborted" },
    );
    assert.deepEqual(await execution.accept(fixture.offer), fixture.receipt);
    assert.equal(await readFile(join(fixture.directory, "starts"), "utf8"), "started\n");
  } finally {
    await fixture.close();
  }
});

test("a retained receipt for another amount cannot complete an accepted operation", async () => {
  const fixture = await operation();
  try {
    await writeFile(
      join(fixture.directory, "settled-receipt.json"),
      JSON.stringify({
        ...fixture.receipt,
        received: { asset: "USDC", amount: "1" },
      }),
      { mode: 0o600 },
    );
    await assert.rejects(create_native_execution(fixture.config).accept(fixture.offer), {
      code: "invalid_message",
    });
    await assert.rejects(readFile(join(fixture.directory, "starts")), { code: "ENOENT" });
  } finally {
    await fixture.close();
  }
});

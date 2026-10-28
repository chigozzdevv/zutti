import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile, chmod, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { DepositService } from "../dist/features/wallet/deposit.service.js";

test("deposit preparation is durable, idempotent and never claims a proof or transaction", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "zutti-deposit-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const service = new DepositService(root);
  const witness = Buffer.from("ZUTTISPV\x02\x00bounded-test-fixture").toString("hex");
  const value = await service.prepare({ witness });
  assert.equal(value.state, "awaiting_proof");
  assert.equal(value.receipt, null);
  assert.deepEqual(await new DepositService(root).prepare({ witness }), value);
  assert.equal((await readFile(join(root, value.id, "witness.bin"))).toString("hex"), witness);
  await writeFile(join(root, value.id, "receipt.bin"), Buffer.from([1, 2]), { mode: 0o600 });
  const ready = await service.get(value.id);
  assert.equal(ready.receipt, "0102");
  assert.equal(ready.state, "proof_ready");
  await chmod(join(root, value.id, "receipt.bin"), 0o644);
  await assert.rejects(service.get(value.id), /Unsafe/);
});

test("malformed requests, unsafe files and other paths cannot enter the deposit queue", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "zutti-deposit-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const service = new DepositService(root);
  for (const value of [
    {},
    { witness: "00" },
    { witness: "ff".repeat(20000) },
    { witness: "aa", receipt: "bb" },
  ])
    await assert.rejects(service.prepare(value));
  await assert.rejects(service.get("../witness.bin"));
  const link = join(root, "ab".repeat(32));
  await symlink(root, link);
  await assert.rejects(service.get("ab".repeat(32)), /Unsafe/);
});

import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, chmod, symlink, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { EncryptedStore } from "../dist/native.js";

test("encrypted records survive restart, bind context and ID, and refuse replacement", async () => {
  const root = await mkdtemp(join(tmpdir(), "zutti-rfq-store-"));
  try {
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
      "encrypt",
      "decrypt",
    ]);
    const store = new EncryptedStore(join(root, "private"), key, "domain-one");
    assert.equal(await store.create("intent-abc", { amount: "5", secret: "do-not-publish" }), true);
    assert.equal(await store.create("intent-abc", { amount: "999" }), false);
    const reopened = new EncryptedStore(join(root, "private"), key, "domain-one");
    assert.deepEqual(await reopened.read("intent-abc"), { amount: "5", secret: "do-not-publish" });
    const path = join(root, "private", "intent-abc.json");
    const bytes = await readFile(path, "utf8");
    assert.ok(!bytes.includes("do-not-publish"));
    await assert.rejects(
      new EncryptedStore(join(root, "private"), key, "domain-two").read("intent-abc"),
    );
    await writeFile(join(root, "private", "intent-def.json"), bytes, { mode: 0o600 });
    await assert.rejects(reopened.read("intent-def"));
    const archive = JSON.parse(bytes);
    archive.ciphertext = (archive.ciphertext[0] === "0" ? "1" : "0") + archive.ciphertext.slice(1);
    await writeFile(path, JSON.stringify(archive));
    await assert.rejects(reopened.read("intent-abc"));
    await chmod(join(root, "private"), 0o755);
    await assert.rejects(reopened.read("intent-abc"));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
test("record paths cannot escape storage or follow symbolic links", async () => {
  const root = await mkdtemp(join(tmpdir(), "zutti-rfq-path-"));
  try {
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
      "encrypt",
      "decrypt",
    ]);
    const store = new EncryptedStore(join(root, "private"), key, "domain");
    await assert.rejects(store.create("../outside", { secret: true }));
    await store.create("valid", { secret: true });
    await symlink(join(root, "private", "valid.json"), join(root, "private", "linked.json"));
    await assert.rejects(store.read("linked"));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

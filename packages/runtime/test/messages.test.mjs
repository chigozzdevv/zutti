import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  MessagingIdentity,
  suite,
  open_message,
  seal_message,
  verify_announcement,
  announce,
  authorize_poll,
  verify_poll,
  unhex,
  hex,
} from "../dist/index.js";

export const domain = {
  network: "testnet",
  deployment_id: "44".repeat(32),
  policy: {
    payment_profile_id: "11".repeat(32),
    source_policy_id: "22".repeat(32),
    proof_profile_id: "33".repeat(32),
    ethereum_finality: "finalized",
    min_source_window_blocks: 10,
    max_source_window_blocks: 100,
    paid_claim_deadline: null,
  },
};
test("HPKE decrypts two sequential official RFC 9180 vector messages", async () => {
  const { vector } = JSON.parse(
    await readFile(new URL("../../../tests/fixtures/hpke-rfc9180.json", import.meta.url), "utf8"),
  );
  const key = await suite.kem.deserializePrivateKey(unhex(vector.skRm));
  const context = await suite.createRecipientContext({
    recipientKey: key,
    enc: unhex(vector.enc),
    info: unhex(vector.info),
  });
  for (const value of vector.encryptions)
    assert.equal(hex(await context.open(unhex(value.ct), unhex(value.aad))), value.pt);
});
test("only the selected recipient decrypts and every signed routing field is bound", async () => {
  const sender = await MessagingIdentity.create();
  const receiver = await MessagingIdentity.create();
  const other = await MessagingIdentity.create();
  const message = await seal_message(
    sender,
    {
      network: domain.network,
      deployment_id: domain.deployment_id,
      type: "request",
      request_id: "55".repeat(32),
      recipient_id: receiver.id,
      recipient_key: receiver.encryption_key,
      deadline: Date.now() + 10_000,
    },
    { amount: "5", private: "receiver-only" },
  );
  assert.deepEqual(await open_message(receiver, message, domain, Date.now()), {
    amount: "5",
    private: "receiver-only",
  });
  assert.ok(!JSON.stringify(message).includes("receiver-only"));
  await assert.rejects(open_message(other, message, domain, Date.now()));
  for (const field of [
    "sender_id",
    "recipient_id",
    "request_id",
    "nonce",
    "enc",
    "ciphertext",
    "signature",
  ]) {
    const changed = {
      ...message,
      [field]: (message[field][0] === "0" ? "1" : "0") + message[field].slice(1),
    };
    await assert.rejects(open_message(receiver, changed, domain, Date.now()));
  }
  await assert.rejects(open_message(receiver, { ...message, type: "offer" }, domain, Date.now()));
  await assert.rejects(
    open_message(receiver, message, { ...domain, network: "mainnet" }, Date.now()),
  );
  await assert.rejects(open_message(receiver, message, domain, message.deadline));
  await assert.rejects(
    open_message(receiver, { ...message, authentication: "verified" }, domain, Date.now()),
  );
});
test("announcements and mailbox access authenticate their actual owner", async () => {
  const owner = await MessagingIdentity.create();
  const announcement = await announce(owner, {
    network: domain.network,
    deployment_id: domain.deployment_id,
    key_version: 1,
    pairs: ["ZEC/USDC"],
    policy: domain.policy,
    deadline: Date.now() + 30_000,
  });
  await verify_announcement(announcement, domain, Date.now());
  await assert.rejects(
    verify_announcement({ ...announcement, encryption_key: "77".repeat(32) }, domain, Date.now()),
  );
  const auth = await authorize_poll(owner, domain, Date.now());
  await verify_poll(auth, domain, Date.now());
  await assert.rejects(verify_poll({ ...auth, recipient_id: "88".repeat(32) }, domain, Date.now()));
});

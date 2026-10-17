import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Buffer } from "node:buffer";
import { URL } from "node:url";

const fixture = JSON.parse(
  readFileSync(new URL("./fixtures/encoding.json", import.meta.url), "utf8"),
);
const u64 = (n) => {
  const bytes = Buffer.alloc(8);
  bytes.writeBigUInt64LE(BigInt(n));
  return bytes;
};
function transcript(tag, parts) {
  const state = createHash("sha256");
  state.update(Buffer.from("ZUTTI\0SHA256\x01"));
  state.update(u64(tag.length));
  state.update(tag);
  state.update(u64(parts.length));
  for (const part of parts) {
    state.update(u64(part.length));
    state.update(part);
  }
  return state.digest("hex");
}
test("Rust domain serialization matches the independent Node transcript", () => {
  const domain = Buffer.from(fixture.domain_bytes, "hex");
  assert.equal(domain.subarray(0, 8).toString(), "ZUTTIDOM");
  assert.equal(domain.readUInt16LE(8), 1);
  assert.equal(domain.readBigUInt64LE(42), 11155111n);
  assert.equal(transcript(Buffer.from("domain"), [domain]), fixture.domain_id);
});
test("monetary wire values remain exact integers", () => {
  const encoded = Buffer.from(fixture.amount.bytes, "hex");
  assert.equal(encoded.length, 17);
  assert.equal(encoded[0], fixture.amount.asset_id);
  const units = encoded.readBigUInt64LE(1) + (encoded.readBigUInt64LE(9) << 64n);
  assert.equal(units.toString(), fixture.amount.units);
});
test("length framing and field separation match across languages", () => {
  assert.equal(
    transcript(
      Buffer.from(fixture.framing.tag),
      fixture.framing.parts_hex.map((s) => Buffer.from(s, "hex")),
    ),
    fixture.framing.digest,
  );
  assert.notEqual(
    transcript(Buffer.from("a"), [Buffer.from("b"), Buffer.from("cd")]),
    transcript(Buffer.from("a"), [Buffer.from("bc"), Buffer.from("d")]),
  );
});

test("order, reservation and terminal nullifier bind the same canonical bytes", () => {
  const bytes = (s) => Buffer.from(s, "hex");
  const domain = bytes(fixture.domain_id);
  assert.equal(
    transcript(Buffer.from("order"), [
      domain,
      bytes(fixture.order.bytes),
      bytes(fixture.order.randomness),
    ]),
    fixture.order.commitment,
  );
  assert.equal(
    transcript(Buffer.from("order-use"), [domain, bytes(fixture.order.nonce)]),
    fixture.order.use_tag,
  );
  assert.equal(
    transcript(Buffer.from("reservation"), [
      domain,
      Buffer.concat([bytes(fixture.order.commitment), bytes(fixture.reservation.transaction_id)]),
      bytes(fixture.reservation.randomness),
    ]),
    fixture.reservation.commitment,
  );
  assert.equal(
    transcript(Buffer.from("escrow-spend"), [domain, bytes(fixture.escrow.secret)]),
    fixture.escrow.terminal_nullifier,
  );
});

test("real block fixtures match their pinned provenance hashes", () => {
  const provenance = JSON.parse(
    readFileSync(new URL("./fixtures/provenance.json", import.meta.url), "utf8"),
  );
  for (const record of provenance.fixtures) {
    const data = readFileSync(new URL(`./fixtures/${record.file}`, import.meta.url));
    assert.equal(createHash("sha256").update(data).digest("hex"), record.sha256);
  }
});

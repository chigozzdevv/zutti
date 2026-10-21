import { Aes128Gcm, CipherSuite, DhkemX25519HkdfSha256, HkdfSha256 } from "@hpke/core";
import { ZuttiError } from "@zutti/schema";
import { canonical, digest, hex, text, unhex } from "./encoding.js";

export const suite = new CipherSuite({
  kem: new DhkemX25519HkdfSha256(),
  kdf: new HkdfSha256(),
  aead: new Aes128Gcm(),
});
export interface StoredIdentity {
  signing: JsonWebKey;
  encryption: JsonWebKey;
}
export class MessagingIdentity {
  private constructor(
    readonly id: string,
    readonly signing_key: string,
    readonly encryption_key: string,
    private readonly signing: CryptoKey,
    readonly encryption: CryptoKey,
    private readonly stored: StoredIdentity,
  ) {}
  static async create(): Promise<MessagingIdentity> {
    const signing = (await crypto.subtle.generateKey("Ed25519", true, [
      "sign",
      "verify",
    ])) as CryptoKeyPair;
    const encryption = await suite.kem.generateKeyPair();
    return MessagingIdentity.restore({
      signing: await crypto.subtle.exportKey("jwk", signing.privateKey),
      encryption: await crypto.subtle.exportKey("jwk", encryption.privateKey),
    });
  }
  static async restore(stored: StoredIdentity): Promise<MessagingIdentity> {
    try {
      const signing = await crypto.subtle.importKey("jwk", stored.signing, "Ed25519", false, [
        "sign",
      ]);
      const encryption = await suite.kem.importKey("jwk", stored.encryption, false);
      if (
        stored.signing.kty !== "OKP" ||
        stored.signing.crv !== "Ed25519" ||
        stored.encryption.kty !== "OKP" ||
        stored.encryption.crv !== "X25519" ||
        !stored.signing.x ||
        !stored.encryption.x
      )
        throw new Error("key");
      const raw = (base64: string) =>
        hex(
          Uint8Array.from(atob(base64.replaceAll("-", "+").replaceAll("_", "/")), (c) =>
            c.charCodeAt(0),
          ),
        );
      const signing_key = raw(stored.signing.x);
      const encryption_key = raw(stored.encryption.x);
      const identity = new MessagingIdentity(
        await identity_id(signing_key),
        signing_key,
        encryption_key,
        signing,
        encryption,
        structuredClone(stored),
      );
      const proof = await identity.sign("key-check", {});
      await verify_signature("key-check", {}, signing_key, proof);
      return identity;
    } catch {
      throw new ZuttiError("wallet_unavailable", "Messaging keys could not be restored");
    }
  }
  export(): StoredIdentity {
    return structuredClone(this.stored);
  }
  async sign(purpose: string, value: unknown): Promise<string> {
    return hex(
      await crypto.subtle.sign(
        "Ed25519",
        this.signing,
        text(canonical({ protocol: "zutti-rfq", version: 1, purpose, value })),
      ),
    );
  }
}
export const identity_id = (signing_key: string): Promise<string> =>
  digest("identity", signing_key);
export async function verify_signature(
  purpose: string,
  value: unknown,
  signing_key: string,
  signature: string,
): Promise<void> {
  try {
    const key = await crypto.subtle.importKey("raw", unhex(signing_key), "Ed25519", false, [
      "verify",
    ]);
    if (
      !(await crypto.subtle.verify(
        "Ed25519",
        key,
        unhex(signature),
        text(canonical({ protocol: "zutti-rfq", version: 1, purpose, value })),
      ))
    )
      throw new Error("signature");
  } catch {
    throw new ZuttiError("invalid_signature", "Message authentication failed");
  }
}

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile, lstat, readdir } from "node:fs/promises";
import { join, isAbsolute } from "node:path";
import { ZuttiError } from "@zutti/schema";

export interface BrowserDeposit {
  id: string;
  witness: string;
  state: "awaiting_proof" | "proof_ready";
  receipt: string | null;
}

export class DepositService {
  constructor(private readonly directory: string) {
    if (!isAbsolute(directory)) throw new Error("Deposit directory must be absolute");
  }

  private async root(): Promise<void> {
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const stat = await lstat(this.directory);
    if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o077) !== 0)
      throw new Error("Unsafe browser deposit directory");
  }

  async prepare(input: unknown): Promise<BrowserDeposit> {
    if (
      typeof input !== "object" ||
      input === null ||
      Array.isArray(input) ||
      Object.keys(input).join(",") !== "witness"
    )
      throw new ZuttiError("invalid_input", "A prepared deposit witness is required.");
    const witness = (input as { witness?: unknown }).witness;
    if (
      typeof witness !== "string" ||
      !/^(?:[0-9a-f]{2})+$/.test(witness) ||
      witness.length > 32_768 ||
      !witness.startsWith(Buffer.from("ZUTTISPV").toString("hex"))
    )
      throw new ZuttiError("invalid_input", "Invalid deposit witness.");
    await this.root();
    const id = createHash("sha256").update(Buffer.from(witness, "hex")).digest("hex");
    const directory = join(this.directory, id);
    try {
      return await this.get(id);
    } catch (error) {
      if (!(error instanceof ZuttiError)) throw error;
    }
    if ((await readdir(this.directory)).length >= 64)
      throw new ZuttiError(
        "wallet_unavailable",
        "The deposit queue is full. Complete the pending deposits first.",
      );
    try {
      await mkdir(directory, { mode: 0o700 });
      await writeFile(join(directory, "witness.bin"), Buffer.from(witness, "hex"), {
        mode: 0o600,
        flag: "wx",
      });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      const existing = await this.get(id);
      if (existing.witness !== witness) throw new Error("Deposit identity mismatch");
      return existing;
    }
    return { id, witness, state: "awaiting_proof", receipt: null };
  }

  async get(id: string): Promise<BrowserDeposit> {
    if (!/^[0-9a-f]{64}$/.test(id)) throw new ZuttiError("invalid_input", "Unknown deposit.");
    await this.root();
    const directory = join(this.directory, id);
    let stat;
    try {
      stat = await lstat(directory);
    } catch {
      throw new ZuttiError("invalid_input", "Unknown deposit.");
    }
    if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o077) !== 0)
      throw new Error("Unsafe deposit directory");
    const read = async (name: string, limit: number): Promise<Buffer> => {
      const path = join(directory, name);
      const stat = await lstat(path);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size > limit || (stat.mode & 0o077) !== 0)
        throw new Error("Unsafe deposit artifact");
      return readFile(path);
    };
    let witness: Buffer;
    try {
      witness = await read("witness.bin", 16_384);
    } catch {
      throw new ZuttiError("invalid_input", "Unknown deposit.");
    }
    let receipt: Buffer | null = null;
    try {
      receipt = await read("receipt.bin", 16 * 1024 * 1024);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    return {
      id,
      witness: witness.toString("hex"),
      state: receipt ? "proof_ready" : "awaiting_proof",
      receipt: receipt?.toString("hex") ?? null,
    };
  }
}

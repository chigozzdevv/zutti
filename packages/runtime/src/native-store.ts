import { constants } from "node:fs";
import { link, lstat, mkdir, open, readdir, unlink } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { ZuttiError } from "@zutti/schema";
import { canonical, hex, random_id, text, unhex } from "./encoding.js";
import type { PrivateStore } from "./store.js";

const MAX_RECORD = 1_048_576;
export class EncryptedStore implements PrivateStore {
  private readonly directory: string;
  constructor(
    directory: string,
    private readonly key: CryptoKey,
    private readonly context: string,
  ) {
    this.directory = resolve(directory);
  }
  private async directory_ready(): Promise<void> {
    try {
      await mkdir(this.directory, { mode: 0o700 });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }
    const metadata = await lstat(this.directory);
    if (metadata.isSymbolicLink() || !metadata.isDirectory() || (metadata.mode & 0o077) !== 0)
      throw new ZuttiError("persistence_failure", "Private storage directory is unsafe");
    const parent = await open(dirname(this.directory), "r");
    try {
      await parent.sync();
    } finally {
      await parent.close();
    }
  }
  private path(id: string): string {
    if (!/^[a-z][a-z0-9-]{0,200}$/.test(id))
      throw new ZuttiError("persistence_failure", "Invalid private record identifier");
    return join(this.directory, `${id}.json`);
  }
  async list(prefix: string): Promise<string[]> {
    await this.directory_ready();
    this.path(prefix);
    const entries = await readdir(this.directory);
    if (entries.length > 10_000)
      throw new ZuttiError(
        "persistence_failure",
        "Private journal requires archival before accepting new work",
      );
    return entries
      .filter((name) => name.startsWith(prefix) && name.endsWith(".json"))
      .map((name) => name.slice(0, -5))
      .sort();
  }
  async read(id: string): Promise<unknown | undefined> {
    try {
      return await this.read_record(id);
    } catch {
      throw new ZuttiError(
        "persistence_failure",
        "Private record could not be read or authenticated",
      );
    }
  }
  private async read_record(id: string): Promise<unknown | undefined> {
    await this.directory_ready();
    let file;
    try {
      file = await open(this.path(id), constants.O_RDONLY | constants.O_NOFOLLOW);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw new ZuttiError("persistence_failure", "Private record could not be opened");
    }
    try {
      const metadata = await file.stat();
      if (
        !metadata.isFile() ||
        (metadata.mode & 0o077) !== 0 ||
        metadata.size > MAX_RECORD * 2 + 256
      )
        throw new Error("record");
      const archive: unknown = JSON.parse(await file.readFile("utf8"));
      if (
        !archive ||
        typeof archive !== "object" ||
        Object.keys(archive).sort().join(",") !== "ciphertext,iv,version" ||
        !("version" in archive) ||
        archive.version !== 1 ||
        !("iv" in archive) ||
        typeof archive.iv !== "string" ||
        archive.iv.length !== 24 ||
        !("ciphertext" in archive) ||
        typeof archive.ciphertext !== "string"
      )
        throw new Error("archive");
      const plaintext = new Uint8Array(
        await crypto.subtle.decrypt(
          {
            name: "AES-GCM",
            iv: unhex(archive.iv),
            additionalData: text(canonical({ version: 1, context: this.context, id })),
          },
          this.key,
          unhex(archive.ciphertext),
        ),
      );
      try {
        if (plaintext.length > MAX_RECORD) throw new Error("size");
        return JSON.parse(new TextDecoder().decode(plaintext)) as unknown;
      } finally {
        plaintext.fill(0);
      }
    } catch {
      throw new ZuttiError("persistence_failure", "Private record authentication failed");
    } finally {
      await file.close();
    }
  }
  async create(id: string, value: unknown): Promise<boolean> {
    try {
      return await this.create_record(id, value);
    } catch {
      throw new ZuttiError("persistence_failure", "Private record could not be saved");
    }
  }
  private async create_record(id: string, value: unknown): Promise<boolean> {
    await this.directory_ready();
    const plaintext = text(canonical(value));
    if (plaintext.length > MAX_RECORD)
      throw new ZuttiError("persistence_failure", "Private record exceeds its bound");
    const iv = crypto.getRandomValues(new Uint8Array(12));
    let ciphertext;
    try {
      ciphertext = await crypto.subtle.encrypt(
        {
          name: "AES-GCM",
          iv,
          additionalData: text(canonical({ version: 1, context: this.context, id })),
        },
        this.key,
        plaintext,
      );
    } finally {
      plaintext.fill(0);
    }
    const destination = this.path(id);
    const temporary = join(this.directory, `.pending-${random_id()}`);
    const file = await open(temporary, "wx", 0o600);
    try {
      await file.writeFile(
        JSON.stringify({ version: 1, iv: hex(iv), ciphertext: hex(ciphertext) }),
      );
      await file.sync();
    } finally {
      await file.close();
    }
    try {
      await link(temporary, destination);
      const directory = await open(this.directory, "r");
      try {
        await directory.sync();
      } finally {
        await directory.close();
      }
      return true;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EEXIST") return false;
      throw new ZuttiError("persistence_failure", "Private record could not be saved");
    } finally {
      await unlink(temporary);
    }
  }
}

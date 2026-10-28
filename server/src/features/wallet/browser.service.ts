import { randomBytes, createHash } from "node:crypto";
import { mkdir, readFile, writeFile, lstat, readdir, open } from "node:fs/promises";
import { constants } from "node:fs";
import { isAbsolute, join } from "node:path";
import { browser_connection, ZuttiError } from "@zutti/schema";
import type { BrowserConnection } from "@zutti/schema";

export interface BrowserSession {
  id: string;
  account: BrowserConnection;
  expires_at: number;
}
const methods = [
  "zutti_scanSource",
  "zutti_prepareOrder",
  "zutti_proveOrder",
  "zutti_signTransaction",
  "zutti_pool",
  "pool_transaction",
] as const;
export interface BrowserRequest {
  id: string;
  method: (typeof methods)[number];
  params: Record<string, unknown>;
}

export class BrowserService {
  constructor(
    private readonly directory: string,
    private readonly domain: string,
  ) {
    if (!isAbsolute(directory)) throw new Error("Browser wallet storage must be absolute");
  }
  async connect(input: unknown, token?: string): Promise<BrowserSession> {
    const account = browser_connection.parse(input);
    if (account.account.domain !== this.domain)
      throw new ZuttiError("deployment_unavailable", "Wallet deployment mismatch.");
    const existing = await this.get(token);
    if (existing && JSON.stringify(existing.account) === JSON.stringify(account)) return existing;
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const stat = await lstat(this.directory);
    if (!stat.isDirectory() || stat.isSymbolicLink() || stat.mode & 0o077)
      throw new Error("Unsafe browser wallet directory");
    if ((await readdir(this.directory)).length >= 128)
      throw new ZuttiError("service_unavailable", "Browser wallet connection limit reached.");
    const id = randomBytes(32).toString("hex");
    const session = { id, account, expires_at: Date.now() + 86400000 };
    await mkdir(join(this.directory, id), { mode: 0o700 });
    await mkdir(join(this.directory, id, "requests"), { mode: 0o700 });
    await mkdir(join(this.directory, id, "responses"), { mode: 0o700 });
    await writeFile(join(this.directory, id, "account.json"), JSON.stringify(session), {
      flag: "wx",
      mode: 0o600,
    });
    return session;
  }
  async get(token?: string): Promise<BrowserSession | null> {
    if (!token || !/^[0-9a-f]{64}$/.test(token)) return null;
    try {
      const path = join(this.directory, token, "account.json");
      const stat = await lstat(path);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.mode & 0o077 || stat.size > 8192)
        return null;
      const value = JSON.parse(await readFile(path, "utf8")) as BrowserSession;
      if (
        value.id !== token ||
        !Number.isSafeInteger(value.expires_at) ||
        value.expires_at < Date.now() ||
        value.expires_at > Date.now() + 86460000
      )
        return null;
      const account = browser_connection.parse(value.account);
      if (account.account.domain !== this.domain) return null;
      return { id: token, account, expires_at: value.expires_at };
    } catch {
      return null;
    }
  }
  operation_id(session: BrowserSession, id: string): string {
    if (!/^[0-9a-f]{64}$/.test(id))
      throw new ZuttiError("invalid_input", "Invalid swap identifier.");
    return createHash("sha256").update(`${session.id}:${id}`).digest("hex");
  }
  private async session(token?: string): Promise<BrowserSession> {
    const session = await this.get(token);
    if (!session) throw new ZuttiError("wallet_unavailable", "Reconnect your browser wallet.");
    return session;
  }
  private async record(path: string, limit: number): Promise<unknown> {
    const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const stat = await file.stat();
      if (!stat.isFile() || stat.mode & 0o077 || stat.size > limit)
        throw new Error("Unsafe browser operation record");
      return JSON.parse(await file.readFile("utf8")) as unknown;
    } finally {
      await file.close();
    }
  }
  async pending(token?: string): Promise<BrowserRequest | null> {
    const session = await this.session(token);
    const names = await readdir(join(this.directory, session.id, "requests"));
    if (names.length > 256)
      throw new ZuttiError("service_unavailable", "Browser operation limit reached.");
    for (const name of names.sort()) {
      if (!/^[0-9a-f]{64}\.json$/.test(name)) continue;
      try {
        await lstat(join(this.directory, session.id, "responses", name));
        continue;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      }
      const request = (await this.record(
        join(this.directory, session.id, "requests", name),
        48 * 1024 * 1024,
      )) as BrowserRequest;
      if (
        request.id !== name.slice(0, -5) ||
        !methods.includes(request.method) ||
        !request.params ||
        typeof request.params !== "object" ||
        Array.isArray(request.params)
      )
        throw new Error("Invalid browser operation request");
      return request;
    }
    return null;
  }
  async complete(token: string | undefined, id: string, result: unknown): Promise<void> {
    const session = await this.session(token);
    if (!/^[0-9a-f]{64}$/.test(id))
      throw new ZuttiError("invalid_input", "Invalid wallet request.");
    const request = (await this.record(
      join(this.directory, session.id, "requests", `${id}.json`),
      48 * 1024 * 1024,
    )) as BrowserRequest;
    if (request.id !== id || !methods.includes(request.method))
      throw new Error("Invalid browser operation request");
    const text = JSON.stringify(result);
    if (
      text.length > 131000 ||
      !result ||
      typeof result !== "object" ||
      Array.isArray(result) ||
      Object.keys(result).sort().join(",") !== "error,result" ||
      ((result as { error?: unknown }).error !== null &&
        typeof (result as { error?: unknown }).error !== "string")
    )
      throw new ZuttiError("invalid_input", "Invalid browser wallet response.");
    const path = join(this.directory, session.id, "responses", `${id}.json`);
    try {
      const file = await open(path, "wx", 0o600);
      try {
        await file.writeFile(text);
        await file.sync();
      } finally {
        await file.close();
      }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      if (JSON.stringify(await this.record(path, 131000)) !== text)
        throw new ZuttiError("invalid_input", "Wallet response is already fixed.");
    }
  }
}

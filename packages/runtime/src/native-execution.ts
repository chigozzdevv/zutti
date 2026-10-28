import { spawn } from "node:child_process";
import { constants } from "node:fs";
import { link, lstat, open, readdir, unlink } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { ZuttiError, settled_receipt, wire_offer } from "@zutti/schema";
import type {
  OperationOptions,
  SettledReceipt,
  StatusHandler,
  StatusUpdate,
  WireOffer,
} from "@zutti/schema";
import { canonical, immutable, random_id } from "./encoding.js";

export interface NativeExecutionConfig {
  readonly directory: string;
  readonly command: string;
  readonly arguments: readonly string[];
  readonly poll_ms?: number;
}

async function read_record(directory: string, name: string): Promise<unknown | undefined> {
  let file;
  try {
    file = await open(join(directory, name), constants.O_RDONLY | constants.O_NOFOLLOW);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw new ZuttiError("persistence_failure", "Operation record could not be opened");
  }
  try {
    const metadata = await file.stat();
    if (!metadata.isFile() || metadata.mode & 0o077 || metadata.size > 131_072)
      throw new Error("record");
    return JSON.parse(await file.readFile("utf8")) as unknown;
  } catch {
    throw new ZuttiError("persistence_failure", "Operation record is invalid");
  } finally {
    await file.close();
  }
}

function matching_receipt(value: unknown, offer: WireOffer): Readonly<SettledReceipt> {
  const receipt = settled_receipt.parse(value);
  if (
    receipt.received.asset !== offer.output.asset ||
    receipt.received.amount !== offer.output.amount ||
    receipt.source_payment.chain !== "zcash" ||
    receipt.pool_settlement.chain !== "ethereum" ||
    receipt.source_payment.policy_id !== offer.policy.source_policy_id ||
    receipt.pool_settlement.policy_id !== offer.policy.source_policy_id ||
    receipt.confirmed_at > Date.now() + 300_000
  )
    throw new ZuttiError(
      "invalid_message",
      "Retained settlement does not match the accepted offer",
    );
  return immutable(receipt);
}

async function persist_offer(directory: string, offer: WireOffer): Promise<void> {
  const existing = await read_record(directory, "selected-offer.json");
  if (existing !== undefined) {
    if (canonical(wire_offer.parse(existing)) !== canonical(offer))
      throw new ZuttiError("persistence_failure", "Operation already belongs to another offer");
    return;
  }
  const temporary = join(directory, `.offer-${random_id()}`);
  const file = await open(temporary, "wx", 0o600);
  try {
    await file.writeFile(JSON.stringify(offer));
    await file.sync();
  } finally {
    await file.close();
  }
  try {
    try {
      await link(temporary, join(directory, "selected-offer.json"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
      const retained = wire_offer.parse(await read_record(directory, "selected-offer.json"));
      if (canonical(retained) !== canonical(offer))
        throw new ZuttiError("persistence_failure", "Operation already belongs to another offer");
    }
    const parent = await open(directory, "r");
    try {
      await parent.sync();
    } finally {
      await parent.close();
    }
  } finally {
    await unlink(temporary);
  }
}

export function create_native_execution(config: NativeExecutionConfig) {
  if (!isAbsolute(config.directory) || !isAbsolute(config.command))
    throw new ZuttiError("invalid_input", "Native execution requires absolute paths");
  const poll = config.poll_ms ?? 500;
  if (!Number.isInteger(poll) || poll < 10 || poll > 5_000)
    throw new ZuttiError("invalid_input", "Invalid operation polling interval");
  const handlers = new Set<StatusHandler>();
  let running: { offer: string; result: Promise<Readonly<SettledReceipt>> } | undefined;

  async function continue_operation(offer: WireOffer): Promise<Readonly<SettledReceipt>> {
    const metadata = await lstat(config.directory);
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || metadata.mode & 0o077)
      throw new ZuttiError("persistence_failure", "Private operation directory is unsafe");
    await persist_offer(config.directory, offer);
    const completed = await read_record(config.directory, "settled-receipt.json");
    if (completed !== undefined) return matching_receipt(completed, offer);
    const log = await open(
      join(config.directory, "worker.stderr"),
      constants.O_WRONLY | constants.O_CREAT | constants.O_APPEND | constants.O_NOFOLLOW,
      0o600,
    );
    let child;
    let ended = false;
    let failure: Error | undefined;
    try {
      const metadata = await log.stat();
      if (!metadata.isFile() || metadata.mode & 0o077)
        throw new ZuttiError("persistence_failure", "Private worker log is unsafe");
      child = spawn(config.command, [...config.arguments], {
        detached: true,
        stdio: ["ignore", "ignore", log.fd],
      });
      child.once("error", (error) => {
        failure = error;
        ended = true;
      });
      child.once("exit", () => {
        ended = true;
      });
      child.unref();
    } finally {
      await log.close();
    }
    let sequence = 0n;
    const deadline = performance.now() + 86_400_000;
    for (;;) {
      const names = await readdir(config.directory);
      if (names.length > 10_000)
        throw new ZuttiError("persistence_failure", "Operation journal exceeds its bound");
      for (const name of names.filter((name) => /^status-[0-9]+\.json$/.test(name)).sort()) {
        const raw = await read_record(config.directory, name);
        if (
          !raw ||
          typeof raw !== "object" ||
          !("sequence" in raw) ||
          typeof raw.sequence !== "string" ||
          !/^[1-9][0-9]*$/.test(raw.sequence)
        )
          throw new ZuttiError("invalid_message", "Invalid retained operation status");
        const status = raw as StatusUpdate;
        if (status.operation_id !== offer.intent_id || BigInt(status.sequence) <= sequence)
          continue;
        sequence = BigInt(status.sequence);
        for (const handler of handlers) {
          try {
            handler(immutable(status));
          } catch {
            continue;
          }
        }
      }
      const receipt = await read_record(config.directory, "settled-receipt.json");
      if (receipt !== undefined) return matching_receipt(receipt, offer);
      if (ended || failure || performance.now() >= deadline)
        throw new ZuttiError(
          "service_unavailable",
          "Settlement paused; its saved operation can continue",
        );
      await new Promise((resolve) => setTimeout(resolve, poll));
    }
  }

  return Object.freeze({
    async accept(
      value: Readonly<WireOffer>,
      options: OperationOptions = {},
    ): Promise<Readonly<SettledReceipt>> {
      const offer = wire_offer.parse(value);
      if (options.signal?.aborted)
        throw new ZuttiError("aborted", "Operation waiting was interrupted");
      if (
        options.wait_ms !== undefined &&
        (!Number.isSafeInteger(options.wait_ms) || options.wait_ms <= 0)
      )
        throw new ZuttiError("invalid_input", "Invalid operation wait budget");
      if (running && running.offer !== offer.id)
        throw new ZuttiError("service_unavailable", "Another offer already owns this operation");
      const callback = options.on_status;
      const handler: StatusHandler | undefined = callback ? (value) => callback(value) : undefined;
      if (handler) handlers.add(handler);
      if (!running) {
        const result = continue_operation(offer);
        running = { offer: offer.id, result };
        void result.catch(() => {
          running = undefined;
        });
      }
      const result = running.result;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let aborted: (() => void) | undefined;
      const interrupted = new Promise<never>((_resolve, reject) => {
        aborted = () =>
          reject(
            new ZuttiError("aborted", "Stopped waiting; the authorized operation is retained"),
          );
        options.signal?.addEventListener("abort", aborted, { once: true });
        if (options.signal?.aborted) aborted();
        if (options.wait_ms !== undefined)
          timer = setTimeout(
            () => reject(new ZuttiError("service_unavailable", "Settlement remains pending")),
            options.wait_ms,
          );
      });
      try {
        return await Promise.race([result, interrupted]);
      } finally {
        if (timer) clearTimeout(timer);
        if (aborted) options.signal?.removeEventListener("abort", aborted);
        if (handler) handlers.delete(handler);
      }
    },
  });
}

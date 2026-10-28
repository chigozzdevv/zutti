import { pool_events, ZuttiError } from "@zutti/schema";
import type { NetworkProfile, PoolEvents } from "@zutti/schema";

const inserted = "0xedce0671667295f0006264205260e23ff16f8cc41f6f5ddd70f702a630ed51ee";
const spent = "0x2d8b76eb247945151bd870d531c84c5420f53e3cb9c0ab3b350f46bb09362096";
const quantity = (value: unknown): number => {
  if (
    typeof value !== "string" ||
    !/^0x[0-9a-fA-F]+$/.test(value) ||
    BigInt(value) > BigInt(Number.MAX_SAFE_INTEGER)
  )
    throw new Error("Invalid chain quantity");
  return Number(BigInt(value));
};
export function decode_output(log: Record<string, unknown>) {
  const topics = log.topics as string[];
  if (
    log.removed ||
    !Array.isArray(topics) ||
    topics.length !== 3 ||
    topics[0] !== inserted ||
    typeof log.data !== "string" ||
    !/^0x(?:[0-9a-f]{2})+$/.test(log.data)
  )
    throw new Error("Invalid pool event");
  const bytes = Buffer.from(log.data.slice(2), "hex");
  const word = (offset: number) =>
    BigInt(`0x${bytes.subarray(offset, offset + 32).toString("hex")}`);
  if (bytes.length < 64 || word(0) !== 32n || word(32) > 4096n || word(32) === 0n)
    throw new Error("Invalid pool ciphertext");
  const length = Number(word(32));
  if (
    bytes.length !== 64 + Math.ceil(length / 32) * 32 ||
    bytes.subarray(64 + length).some((byte) => byte !== 0)
  )
    throw new Error("Invalid ciphertext padding");
  return {
    position: quantity(topics[1]),
    commitment: topics[2]!,
    ciphertext: `0x${bytes.subarray(64, 64 + length).toString("hex")}`,
  };
}

export class PoolService {
  private cached: PoolEvents | undefined;
  private pending: Promise<PoolEvents> | undefined;
  constructor(
    private readonly profile: NetworkProfile,
    private readonly first: number,
    private readonly endpoint = "https://ethereum-sepolia-rpc.publicnode.com",
  ) {}
  private async rpc(method: string, params: unknown[]): Promise<unknown> {
    const response = await fetch(this.endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
      signal: AbortSignal.timeout(12000),
    });
    if (!response.ok) throw new Error("Pool RPC unavailable");
    const raw = await response.text();
    if (raw.length > 8 * 1024 * 1024) throw new Error("Pool RPC response exceeds its bound");
    const value = JSON.parse(raw) as { error?: unknown; result?: unknown };
    if (value.error || value.result === undefined || value.result === null)
      throw new Error("Pool RPC rejected the request");
    return value.result;
  }
  async snapshot(): Promise<PoolEvents> {
    if (this.cached && Date.now() - this.cached.checked_at < 30000) return this.cached;
    this.pending ??= this.collect()
      .then((value) => {
        this.cached = value;
        return value;
      })
      .finally(() => {
        this.pending = undefined;
      });
    try {
      return await this.pending;
    } catch {
      throw new ZuttiError("service_unavailable", "Inventory is refreshing. Try again shortly.");
    }
  }
  private async collect(): Promise<PoolEvents> {
    if (this.profile.name !== "testnet" || (await this.rpc("eth_chainId", [])) !== "0xaa36a7")
      throw new Error("Sepolia required");
    const block = (await this.rpc("eth_getBlockByNumber", ["finalized", false])) as {
      number: string;
      hash: string;
    };
    const last = quantity(block.number);
    if (
      !Number.isSafeInteger(this.first) ||
      this.first <= 0 ||
      last < this.first ||
      last - this.first > 64000
    )
      throw new Error("Pool history exceeds its bound");
    const pool = this.profile.ethereum.pool;
    const pinned = { blockHash: block.hash, requireCanonical: true };
    const [root, size] = await Promise.all([
      this.rpc("eth_call", [{ to: pool, data: "0xebf0c717" }, pinned]),
      this.rpc("eth_call", [{ to: pool, data: "0x66c7cb11" }, pinned]),
    ]);
    const logs: Record<string, unknown>[] = [];
    for (let first = this.first; first <= last; first += 1000) {
      const batch = await this.rpc("eth_getLogs", [
        {
          address: pool,
          fromBlock: `0x${first.toString(16)}`,
          toBlock: `0x${Math.min(first + 999, last).toString(16)}`,
          topics: [[inserted, spent]],
        },
      ]);
      if (!Array.isArray(batch) || batch.length + logs.length > 131072)
        throw new Error("Invalid pool logs");
      for (const log of batch as Record<string, unknown>[]) {
        if (
          typeof log.address !== "string" ||
          log.address.toLowerCase() !== pool.toLowerCase() ||
          log.removed ||
          quantity(log.blockNumber) < first ||
          quantity(log.blockNumber) > Math.min(first + 999, last)
        )
          throw new Error("Pool log origin mismatch");
        logs.push(log);
      }
    }
    logs.sort(
      (a, b) =>
        quantity(a.blockNumber) - quantity(b.blockNumber) ||
        quantity(a.transactionIndex) - quantity(b.transactionIndex) ||
        quantity(a.logIndex) - quantity(b.logIndex),
    );
    const outputs = logs
      .filter((log) => (log.topics as string[])[0] === inserted)
      .map(decode_output);
    if (outputs.length !== quantity(size) || outputs.some((out, i) => out.position !== i))
      throw new Error("Incomplete pool history");
    const transactions: PoolEvents["transactions"] = [];
    const hashes = [...new Set(logs.map((log) => String(log.transactionHash)))];
    for (const id of hashes) {
      const [transaction, receipt] = (await Promise.all([
        this.rpc("eth_getTransactionByHash", [id]),
        this.rpc("eth_getTransactionReceipt", [id]),
      ])) as [
        { from: string; to: string; input: string; blockHash: string },
        { status: string; blockHash: string },
      ];
      if (
        transaction.to.toLowerCase() !== pool.toLowerCase() ||
        quantity(receipt.status) !== 1 ||
        receipt.blockHash !== transaction.blockHash
      )
        throw new Error("Pool transaction mismatch");
      const data = Buffer.from(transaction.input.slice(2), "hex");
      if (data.length < 68 || data.subarray(0, 4).toString("hex") !== "1d0a9b2e")
        throw new Error("Unsupported pool operation");
      const offset = Number(BigInt(`0x${data.subarray(36, 68).toString("hex")}`)) + 4;
      if (!Number.isSafeInteger(offset) || offset < 68 || offset + 352 > data.length)
        throw new Error("Invalid operation offset");
      const kind = Number(BigInt(`0x${data.subarray(offset, offset + 32).toString("hex")}`));
      const kinds = ["", "deposit", "transfer", "withdrawal", "reservation", "settlement"] as const;
      if (!kinds[kind]) throw new Error("Unsupported operation kind");
      const units = BigInt(`0x${data.subarray(offset + 224, offset + 256).toString("hex")}`);
      const amount =
        kind === 1 || kind === 3
          ? `${units / 1000000n}.${(units % 1000000n).toString().padStart(6, "0")}`.replace(
              /\.?0+$/,
              "",
            )
          : null;
      const header = (await this.rpc("eth_getBlockByHash", [receipt.blockHash, false])) as {
        timestamp: string;
      };
      transactions.push({
        id,
        from: transaction.from,
        kind: kinds[kind] as PoolEvents["transactions"][number]["kind"],
        amount,
        at: quantity(header.timestamp) * 1000,
      });
    }
    const canonical = (await this.rpc("eth_getBlockByNumber", [block.number, false])) as {
      hash: string;
    };
    if (canonical.hash !== block.hash) throw new Error("Inventory branch changed");
    return pool_events.parse({
      events: {
        schema: 1,
        chain_id: 11155111,
        pool,
        domain: this.profile.deployment_id,
        root,
        next_leaf: outputs.length,
        execution_hash: block.hash,
        execution_height: last,
        outputs,
      },
      transactions,
      checked_at: Date.now(),
    });
  }
}

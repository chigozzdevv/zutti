import type { NetworkProfile, PoolEvents } from "@zutti/schema";
import type { EthereumWallet } from "./ethereum-wallet.js";
import type { ZcashWallet } from "./zcash-wallet.js";
import { usdc_units } from "./funding.js";

export interface InventoryState {
  available: string | null;
  reserved: string | null;
  active_trades: number | null;
  live_quotes: number | null;
  activities: PoolEvents["transactions"];
  loading: boolean;
  error: string | null;
}
export const empty_inventory: InventoryState = {
  available: null,
  reserved: null,
  active_trades: null,
  live_quotes: null,
  activities: [],
  loading: false,
  error: null,
};
export class BrowserInventory {
  private current: InventoryState = empty_inventory;
  private readonly listeners = new Set<() => void>();
  private active = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private scan:
    | {
        root: string;
        result: {
          inventory: string;
          notes: { position: number; amount: string; request: { nullifier: string } }[];
        };
      }
    | undefined;
  private readonly ethereum: Pick<EthereumWallet, "snapshot" | "extension">;
  private readonly zcash: Pick<ZcashWallet, "snapshot" | "execute">;
  private readonly profile: NetworkProfile;
  private readonly read: () => Promise<PoolEvents>;
  private readonly owner: string;
  private readonly receiver: string;
  constructor(
    ethereum: Pick<EthereumWallet, "snapshot" | "extension">,
    zcash: Pick<ZcashWallet, "snapshot" | "execute">,
    profile: NetworkProfile,
    read: () => Promise<PoolEvents>,
  ) {
    this.ethereum = ethereum;
    this.zcash = zcash;
    this.profile = profile;
    this.read = read;
    this.owner = ethereum.snapshot().account?.toLowerCase() ?? "";
    this.receiver = zcash.snapshot().account?.usdc_receiver.owner ?? "";
  }
  snapshot = () => this.current;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private update(value: Partial<InventoryState>) {
    if (!this.active) return;
    this.current = Object.freeze({ ...this.current, ...value });
    for (const listener of this.listeners) listener();
  }
  start() {
    if (this.active) return;
    this.active = true;
    void this.refresh();
  }
  stop() {
    this.active = false;
    clearTimeout(this.timer);
    this.scan = undefined;
  }
  private async refresh() {
    this.update({ loading: this.current.available === null });
    try {
      const provider = this.ethereum.extension();
      if (
        !provider ||
        this.ethereum.snapshot().account?.toLowerCase() !== this.owner ||
        this.zcash.snapshot().account?.usdc_receiver.owner !== this.receiver ||
        this.zcash.snapshot().account?.domain !== this.profile.deployment_id ||
        (await provider.request({ method: "eth_chainId" })) !== "0xaa36a7"
      )
        throw new Error("Reconnect your Testnet wallets.");
      const data = await this.read();
      if (!this.active) return;
      if (
        data.events.domain !== this.profile.deployment_id ||
        data.events.pool.toLowerCase() !== this.profile.ethereum.pool.toLowerCase()
      )
        throw new Error("Inventory deployment mismatch.");
      if (this.scan?.root !== data.events.root) {
        const result = (await this.zcash.execute("zutti_pool", {
          request: JSON.stringify({ action: "scan", events: data.events }),
        })) as NonNullable<BrowserInventory["scan"]>["result"];
        if (
          !result ||
          !Array.isArray(result.notes) ||
          typeof result.inventory !== "string" ||
          result.notes.length > 64
        )
          throw new Error("Invalid wallet inventory.");
        this.scan = { root: data.events.root, result };
      }
      let units = 0n;
      const seen = new Set<number>();
      for (const note of this.scan.result.notes) {
        if (
          !Number.isSafeInteger(note.position) ||
          note.position < 0 ||
          note.position >= data.events.next_leaf ||
          seen.has(note.position) ||
          !/^0x[0-9a-f]{64}$/.test(note.request.nullifier)
        )
          throw new Error("Invalid owned note.");
        seen.add(note.position);
        const value = await provider.request({
          method: "eth_call",
          params: [
            {
              to: this.profile.ethereum.pool,
              data: `0xae20bed3${note.request.nullifier.slice(2)}`,
            },
            "latest",
          ],
        });
        if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(value) || BigInt(value) > 1n)
          throw new Error("Note status unavailable.");
        if (BigInt(value) === 0n) units += usdc_units(note.amount);
      }
      if (
        !this.active ||
        this.ethereum.snapshot().account?.toLowerCase() !== this.owner ||
        this.zcash.snapshot().account?.usdc_receiver.owner !== this.receiver
      )
        return;
      const activities = data.transactions
        .filter((item) => item.from.toLowerCase() === this.owner)
        .sort((a, b) => b.at - a.at);
      const has_reservations = activities.some((item) => item.kind === "reservation");
      this.update({
        available: `${units / 1000000n}.${(units % 1000000n).toString().padStart(6, "0")}`.replace(
          /\.?0+$/,
          "",
        ),
        reserved: has_reservations ? null : "0",
        active_trades: has_reservations ? null : 0,
        live_quotes: 0,
        activities,
        loading: false,
        error: null,
      });
    } catch (error) {
      this.update({
        ...empty_inventory,
        error: error instanceof Error ? error.message : "Inventory is refreshing.",
      });
    } finally {
      if (this.active)
        this.timer = setTimeout(() => {
          void this.refresh();
        }, 15000);
    }
  }
}

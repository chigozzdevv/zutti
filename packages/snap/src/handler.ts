import type { OnRpcRequestHandler, SnapsProvider } from "@metamask/snaps-sdk";
import { Box, Heading, Text } from "@metamask/snaps-sdk/jsx";
import { allowed_origins } from "./policy.ts";

export interface Authorization {
  review(): string;
  sign(wallet: Wallet, now: number): Uint8Array;
  free(): void;
}

export interface Wallet {
  account(): string;
  authorize(payment: string, now: number): Authorization;
  prepare_deposit(amount: string, owner: string): string;
  verify_deposit(witness: string, receipt: string): string;
  free(): void;
}

export interface WalletRuntime {
  wallet(entropy: Uint8Array): Promise<Wallet>;
}

function bytes(hex: unknown): Uint8Array {
  if (typeof hex !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(hex)) {
    throw new Error("MetaMask returned invalid wallet entropy");
  }
  return Uint8Array.from(hex.slice(2).match(/../g)!, (pair) => parseInt(pair, 16));
}

function encode(value: Uint8Array): string {
  return Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function fee(units: string): string {
  if (!/^\d+$/.test(units)) throw new Error("Invalid verified fee");
  const padded = units.padStart(9, "0");
  return `${padded.slice(0, -8)}.${padded.slice(-8)}`;
}

function content(title: string, ...lines: string[]) {
  const children = [
    { ...Heading({ children: title }), props: { children: title } },
    ...lines.map((line) => ({ ...Text({ children: line }), props: { children: line } })),
  ];
  return { ...Box({ children }), props: { children } };
}

export function create_handler(
  runtime: WalletRuntime,
  provider: SnapsProvider,
  now = () => Math.floor(Date.now() / 1000),
): OnRpcRequestHandler {
  let busy = false;
  return async ({ origin, originMetadata, request }) => {
    if (
      !allowed_origins.includes(origin) ||
      (originMetadata !== null && originMetadata !== undefined)
    ) {
      throw new Error("This origin cannot use the Zutti Testnet wallet");
    }
    if (busy) throw new Error("Complete the pending wallet request first");
    if (
      ![
        "zutti_getAccount",
        "zutti_signPayment",
        "zutti_prepareDeposit",
        "zutti_verifyDeposit",
      ].includes(request.method)
    ) {
      throw new Error("Unsupported Zutti wallet method");
    }
    const params = request.params;
    if (
      params !== undefined &&
      (params === null || typeof params !== "object" || Array.isArray(params))
    ) {
      throw new Error("Invalid wallet parameters");
    }
    const fields = Object.keys(params ?? {});
    let payment: string | undefined;
    if (request.method === "zutti_getAccount") {
      if (fields.length !== 0) throw new Error("Account access takes no parameters");
    } else if (request.method === "zutti_signPayment") {
      if (fields.length !== 1 || fields[0] !== "payment" || typeof params?.payment !== "string") {
        throw new Error("A prepared payment and verification evidence are required");
      }
      payment = params.payment;
      if (payment.length === 0 || payment.length > 48 * 1024 * 1024) {
        throw new Error("Payment request exceeds its bound");
      }
    } else if (request.method === "zutti_prepareDeposit") {
      if (
        fields.sort().join(",") !== "amount,owner" ||
        typeof params?.amount !== "string" ||
        !/^(0|[1-9][0-9]{0,7})(\.[0-9]{1,6})?$/.test(params.amount) ||
        !/[1-9]/.test(params.amount) ||
        typeof params.owner !== "string" ||
        !/^0x[0-9a-fA-F]{40}$/.test(params.owner) ||
        /^0x0{40}$/.test(params.owner)
      ) {
        throw new Error("A positive USDC amount and Sepolia funding address are required");
      }
    } else {
      if (
        fields.sort().join(",") !== "receipt,witness" ||
        typeof params?.witness !== "string" ||
        typeof params.receipt !== "string" ||
        !/^(?:[0-9a-f]{2})+$/.test(params.witness) ||
        !/^(?:[0-9a-f]{2})+$/.test(params.receipt) ||
        params.witness.length > 524288 ||
        params.receipt.length > 32 * 1024 * 1024
      ) {
        throw new Error("A bounded deposit witness and genuine receipt are required");
      }
    }
    busy = true;
    let wallet: Wallet | undefined;
    let authorization: Authorization | undefined;
    try {
      if (request.method === "zutti_getAccount") {
        const approved = await provider.request({
          method: "snap_dialog",
          params: {
            type: "confirmation",
            content: content(
              "Connect Zutti Testnet",
              `Share your Testnet receiving addresses with ${origin}?`,
              "Use test funds only. This development wallet is recovered with the same MetaMask recovery phrase and Snap ID. Changing the Snap installation URL changes the wallet.",
            ),
          },
        });
        if (approved !== true) throw new Error("Wallet connection declined");
      }
      const entropy = bytes(
        await provider.request({
          method: "snap_getEntropy",
          params: { version: 1, salt: "zutti-testnet-wallet-v1" },
        }),
      );
      try {
        wallet = await runtime.wallet(entropy);
      } finally {
        entropy.fill(0);
      }
      if (request.method === "zutti_getAccount") return JSON.parse(wallet.account());
      if (request.method === "zutti_prepareDeposit") {
        const prepared = JSON.parse(
          wallet.prepare_deposit(params!.amount as string, params!.owner as string),
        );
        const approved = await provider.request({
          method: "snap_dialog",
          params: {
            type: "confirmation",
            content: content(
              "Prepare Testnet deposit",
              `Deposit ${prepared.amount} USDC into your private Zutti inventory on Sepolia.`,
              `Funding wallet: ${prepared.owner}`,
              "This demo publishes the deposit witness to a remote prover. It reveals the amount and note opening. Signing and viewing secret keys stay in Flask.",
              "MetaMask will request token approval and the deposit transaction after the proof verifies.",
            ),
          },
        });
        if (approved !== true) throw new Error("Deposit preparation declined");
        return prepared;
      }
      if (request.method === "zutti_verifyDeposit") {
        return JSON.parse(
          wallet.verify_deposit(params!.witness as string, params!.receipt as string),
        );
      }
      if (payment === undefined) throw new Error("Prepared payment required");
      authorization = wallet.authorize(payment, now());
      const review = JSON.parse(authorization.review()) as {
        swap_id: string;
        txid: string;
        zec: string;
        usdc: string;
        fee_zatoshi: string;
        recipient: string;
        expiry_height: number;
      };
      const approved = await provider.request({
        method: "snap_dialog",
        params: {
          type: "confirmation",
          content: content(
            "Approve Testnet swap",
            `Send ${review.zec} ZEC`,
            `Receive ${review.usdc} private USDC on Sepolia`,
            `Zcash fee: ${fee(review.fee_zatoshi)} ZEC`,
            `To: ${review.recipient}`,
            `Payment expires at Zcash block ${review.expiry_height}`,
            `Swap: ${review.swap_id}`,
          ),
        },
      });
      if (approved !== true) throw new Error("Payment declined");
      const capability = authorization;
      authorization = undefined;
      const signed = capability.sign(wallet, now());
      try {
        return { swap_id: review.swap_id, txid: review.txid, pczt: encode(signed) };
      } finally {
        signed.fill(0);
      }
    } finally {
      authorization?.free();
      wallet?.free();
      busy = false;
    }
  };
}

import { useState } from "react";
import { ArrowRight, Check, Copy, LoaderCircle, LogOut, Wallet } from "lucide-react";
import type { WalletSession } from "@zutti/schema";
import { sepolia_chain } from "../shared/ethereum-wallet.js";
import type { EthereumState } from "../shared/ethereum-wallet.js";
import { short_id } from "../shared/format.js";
import { Dialog } from "./dialog.js";

interface WalletProps {
  ethereum: EthereumState;
  connect_ethereum: () => void;
  disconnect_ethereum: () => void;
  switch_network: () => void;
  session: WalletSession;
  native_available: boolean;
  connect_native: () => void;
  disconnect_native: () => void;
  busy: boolean;
  pending_swap: boolean;
  error: string | null;
  received: string | null;
  close: () => void;
}
export function WalletDialog(props: WalletProps) {
  const { ethereum, session } = props;
  const [copied, set_copied] = useState<string | null>(null);
  const [copy_error, set_copy_error] = useState(false);
  const connected = !!ethereum.account || session.connected;
  return (
    <Dialog title={connected ? "Your wallets" : "Connect wallet"} close={props.close}>
      <div className="wallet-section-title">Ethereum</div>
      <div className="wallet-option">
        <span className="wallet-option-icon">
          <Wallet size={22} />
        </span>
        <div>
          <strong>MetaMask</strong>
          <span>
            {ethereum.account
              ? ethereum.chain_id === sepolia_chain
                ? "Sepolia"
                : "Switch to Sepolia"
              : "Ethereum Sepolia"}
          </span>
        </div>
        {ethereum.account ? (
          <button
            className="text-button"
            disabled={ethereum.pending}
            onClick={props.disconnect_ethereum}
            aria-label="Disconnect MetaMask"
          >
            <LogOut size={15} />
          </button>
        ) : ethereum.available ? (
          <button
            className="text-button"
            disabled={ethereum.pending}
            onClick={props.connect_ethereum}
          >
            {ethereum.pending ? (
              <>
                <LoaderCircle size={15} className="spin" /> Connecting…
              </>
            ) : (
              <>
                Connect <ArrowRight size={14} />
              </>
            )}
          </button>
        ) : (
          <a
            className="text-button"
            href="https://metamask.io/download/"
            target="_blank"
            rel="noreferrer"
          >
            Install <ArrowRight size={14} />
          </a>
        )}
      </div>
      {ethereum.pending && (
        <p className="dialog-caption" role="status">
          Confirm the request in MetaMask.
        </p>
      )}
      {ethereum.account && (
        <div className="wallet-address">
          <span title={ethereum.account}>{short_id(ethereum.account)}</span>
          <button
            className="text-button"
            aria-label="Copy Ethereum address"
            onClick={() => {
              void navigator.clipboard
                .writeText(ethereum.account!)
                .then(() => {
                  set_copied(ethereum.account);
                  set_copy_error(false);
                })
                .catch(() => set_copy_error(true));
            }}
          >
            {copied === ethereum.account ? <Check size={14} /> : <Copy size={14} />}
          </button>
        </div>
      )}
      {copy_error && (
        <p className="form-error" role="alert">
          Couldn’t copy the address.
        </p>
      )}
      {ethereum.account && ethereum.chain_id !== sepolia_chain && (
        <button
          className="secondary-button"
          disabled={ethereum.pending}
          onClick={props.switch_network}
        >
          {ethereum.pending && <LoaderCircle size={15} className="spin" />} Switch to Sepolia
        </button>
      )}
      {ethereum.error && (
        <p className="form-error" role="alert">
          {ethereum.error}
        </p>
      )}
      <div className="wallet-section-title wallet-zcash-title">Zcash</div>
      {session.connected || props.native_available ? (
        <>
          <div className="wallet-option">
            <span className="wallet-option-icon">
              <Wallet size={22} />
            </span>
            <div>
              <strong>{session.connected ? session.label : "Local wallet"}</strong>
              <span>Zcash Testnet · Private USDC</span>
            </div>
            {session.connected && <span className="connected-dot" />}
          </div>
          {props.received && (
            <p className="dialog-caption">Last swap received {props.received} USDC.</p>
          )}
          <button
            className="secondary-button"
            disabled={props.busy || (session.connected && props.pending_swap)}
            onClick={session.connected ? props.disconnect_native : props.connect_native}
          >
            {props.busy && <LoaderCircle size={15} className="spin" />}
            {session.connected ? "Disconnect local wallet" : "Connect local wallet"}
          </button>
          {props.pending_swap && (
            <p className="dialog-caption">A swap is running in this wallet.</p>
          )}
        </>
      ) : (
        <p className="dialog-caption">A compatible Zcash Testnet signer is required for swaps.</p>
      )}
      {props.error && (
        <p className="form-error" role="alert">
          {props.error}
        </p>
      )}
    </Dialog>
  );
}

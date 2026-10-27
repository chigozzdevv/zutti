use crate::{ConsensusConfig, Error, FinalityEvidence, Result, verify_finalized};
use alloy::{
    consensus::{Header, ReceiptEnvelope, TxEnvelope, proofs},
    eips::eip2718::Encodable2718,
    primitives::{Address, B256, Log, U256, keccak256},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub chain_id: u64,
    pub token: Address,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub transaction_id: B256,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferEvidence {
    pub consensus: FinalityEvidence,
    pub transactions: Vec<TxEnvelope>,
    pub receipts: Vec<ReceiptEnvelope<Log>>,
}

#[derive(Debug, Serialize)]
pub struct VerifiedTransfer {
    pub transaction_id: B256,
    pub execution_hash: B256,
    pub execution_height: u64,
    pub finalized_slot: u64,
    pub recipient: Address,
    pub value: U256,
}

pub fn verify(
    config: &ConsensusConfig,
    request: &TransferRequest,
    evidence: &TransferEvidence,
    now: u64,
) -> Result<VerifiedTransfer> {
    if request.chain_id != config.chain_id
        || request.token.is_zero()
        || request.from.is_zero()
        || request.to.is_zero()
        || request.value.is_zero()
        || request.transaction_id.is_zero()
    {
        return Err(Error::Configuration);
    }
    let execution = verify_finalized(config, &evidence.consensus, now)?;
    verify_body(
        request,
        &execution.header,
        &evidence.transactions,
        &evidence.receipts,
    )?;
    Ok(VerifiedTransfer {
        transaction_id: request.transaction_id,
        execution_hash: execution.header.hash_slow(),
        execution_height: execution.header.number,
        finalized_slot: execution.finalized_slot,
        recipient: request.to,
        value: request.value,
    })
}

fn verify_body(
    request: &TransferRequest,
    header: &Header,
    transactions: &[TxEnvelope],
    receipts: &[ReceiptEnvelope<Log>],
) -> Result<()> {
    if transactions.is_empty() || transactions.len() > 4096 || transactions.len() != receipts.len()
    {
        return Err(Error::Bounds);
    }
    if proofs::calculate_transaction_root(transactions) != header.transactions_root
        || proofs::calculate_receipt_root(receipts) != header.receipts_root
    {
        return Err(Error::Execution);
    }
    let position = transactions
        .iter()
        .position(|tx| keccak256(tx.encoded_2718()) == request.transaction_id)
        .ok_or(Error::Execution)?;
    let receipt = &receipts[position];
    let transfer = keccak256(b"Transfer(address,address,uint256)");
    let from = B256::from(request.from.into_word());
    let to = B256::from(request.to.into_word());
    if !receipt.status()
        || !receipt.logs().iter().any(|log| {
            log.address == request.token
                && log.data.topics() == [transfer, from, to]
                && log.data.data.as_ref() == request.value.to_be_bytes::<32>()
        })
    {
        return Err(Error::Storage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{Receipt, SignableTransaction, TxLegacy},
        primitives::{LogData, Signature},
    };

    fn fixture() -> (
        TransferRequest,
        Header,
        Vec<TxEnvelope>,
        Vec<ReceiptEnvelope<Log>>,
    ) {
        let transaction: TxEnvelope = TxLegacy::default()
            .into_signed(Signature::new(U256::from(1), U256::from(2), false))
            .into();
        let request = TransferRequest {
            chain_id: 11155111,
            token: Address::repeat_byte(1),
            from: Address::repeat_byte(2),
            to: Address::repeat_byte(3),
            value: U256::from(1_000_000),
            transaction_id: keccak256(transaction.encoded_2718()),
        };
        let receipt = ReceiptEnvelope::Legacy(
            Receipt {
                status: true.into(),
                cumulative_gas_used: 21_000,
                logs: vec![Log {
                    address: request.token,
                    data: LogData::new_unchecked(
                        vec![
                            keccak256(b"Transfer(address,address,uint256)"),
                            request.from.into_word(),
                            request.to.into_word(),
                        ],
                        request.value.to_be_bytes::<32>().to_vec().into(),
                    ),
                }],
            }
            .with_bloom(),
        );
        let transactions = vec![transaction];
        let receipts = vec![receipt];
        let header = Header {
            transactions_root: proofs::calculate_transaction_root(&transactions),
            receipts_root: proofs::calculate_receipt_root(&receipts),
            ..Default::default()
        };
        (request, header, transactions, receipts)
    }

    #[test]
    fn authenticated_transfer_rejects_wrong_recipient_amount_and_transaction() {
        let (mut request, header, transactions, receipts) = fixture();
        assert!(verify_body(&request, &header, &transactions, &receipts).is_ok());
        request.to = Address::repeat_byte(4);
        assert!(verify_body(&request, &header, &transactions, &receipts).is_err());
        request.to = Address::repeat_byte(3);
        request.value += U256::from(1);
        assert!(verify_body(&request, &header, &transactions, &receipts).is_err());
        request.value -= U256::from(1);
        request.transaction_id = B256::repeat_byte(9);
        assert!(verify_body(&request, &header, &transactions, &receipts).is_err());
    }

    #[test]
    fn rpc_metadata_cannot_replace_committed_transaction_or_receipt_roots() {
        let (request, mut header, transactions, receipts) = fixture();
        header.receipts_root = B256::ZERO;
        assert!(verify_body(&request, &header, &transactions, &receipts).is_err());
        header.receipts_root = proofs::calculate_receipt_root(&receipts);
        header.transactions_root = B256::ZERO;
        assert!(verify_body(&request, &header, &transactions, &receipts).is_err());
        assert!(verify_body(&request, &header, &transactions, &[]).is_err());
    }
}

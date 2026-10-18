use crate::{block::ParsedBlock, transaction::BlockValue};
use zcash_protocol::consensus::NetworkType;
use zebra_chain::{
    amount::{Amount, NonNegative},
    block::Height,
    parameters::{Network, subsidy},
    serialization::ZcashDeserialize,
};
use zutti_protocol::{
    Error as CodecError,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("unsupported subsidy network")]
    Network,
    #[error("invalid subsidy or funding stream parameters")]
    Parameters,
    #[error("required founders reward, funding stream or disbursement output is absent")]
    Recipient,
    #[error("coinbase value does not match permitted subsidy, fees and deferred output")]
    Value,
    #[error("deferred pool balance is invalid")]
    Deferred,
    #[error("NSM reserve does not match its activation or monetary bounds")]
    Nsm,
}
type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct MonetaryState {
    network: Network,
    deferred: u64,
    issued: u64,
    nsm: Option<u64>,
}
impl MonetaryState {
    pub fn new(network: NetworkType) -> Result<Self> {
        let network = match network {
            NetworkType::Main => Network::Mainnet,
            NetworkType::Test => Network::new_default_testnet(),
            NetworkType::Regtest => return Err(Error::Network),
        };
        Ok(Self {
            network,
            deferred: 0,
            issued: 0,
            nsm: None,
        })
    }
    pub const fn deferred(&self) -> u64 {
        self.deferred
    }
    pub const fn issued(&self) -> u64 {
        self.issued
    }
    pub const fn nsm(&self) -> Option<u64> {
        self.nsm
    }
    pub(crate) fn validate_height(&self, network: NetworkType, height: u32) -> Result<()> {
        let expected = if matches!(self.network, Network::Mainnet) {
            NetworkType::Main
        } else {
            NetworkType::Test
        };
        if expected != network {
            return Err(Error::Network);
        }
        if subsidy::nsm_value_balance_is_tracked(Height(height), &self.network)
            != self.nsm.is_some()
        {
            return Err(Error::Nsm);
        }
        Ok(())
    }
    pub fn transition(&self, block: &ParsedBlock, value: BlockValue) -> Result<Self> {
        let height = Height(block.claimed_height());
        let network = if matches!(self.network, Network::Mainnet) {
            NetworkType::Main
        } else {
            NetworkType::Test
        };
        self.validate_height(network, height.0.checked_sub(1).ok_or(Error::Parameters)?)?;
        let parent =
            Amount::<NonNegative>::try_from(self.nsm.unwrap_or(0)).map_err(|_| Error::Nsm)?;
        let expected =
            subsidy::block_subsidy_with_parent_nsm_value_balance(height, &self.network, parent)
                .map_err(|_| Error::Parameters)?;
        let bytes = block.encoded().map_err(|_| Error::Parameters)?;
        let zebra = zebra_chain::block::Block::zcash_deserialize(bytes.as_slice())
            .map_err(|_| Error::Parameters)?;
        let coinbase = zebra.transactions.first().ok_or(Error::Value)?;
        let transparent = coinbase
            .outputs()
            .iter()
            .try_fold(0_u64, |sum, output| {
                sum.checked_add(u64::from(output.value()))
            })
            .ok_or(Error::Value)?;
        let actual_value = i128::from(transparent)
            - i128::from(i64::from(coinbase.sapling_value_balance().sapling_amount()))
            - i128::from(i64::from(coinbase.orchard_value_balance().orchard_amount()))
            - i128::from(i64::from(
                coinbase.ironwood_value_balance().ironwood_amount(),
            ));
        if actual_value != i128::from(value.coinbase_value) {
            return Err(Error::Value);
        }
        let deferred = subsidy::subsidy_is_valid(&zebra, &self.network, expected)
            .map_err(|_| Error::Recipient)?;
        let fees = Amount::<NonNegative>::try_from(value.fees).map_err(|_| Error::Value)?;
        subsidy::miner_fees_are_valid(coinbase, height, fees, expected, deferred, &self.network)
            .map_err(|_| Error::Value)?;
        let deferred_delta = i128::from(i64::from(deferred.value()));
        self.apply_value(height, value, deferred_delta)
    }

    fn apply_value(&self, height: Height, value: BlockValue, deferred_delta: i128) -> Result<Self> {
        let parent =
            Amount::<NonNegative>::try_from(self.nsm.unwrap_or(0)).map_err(|_| Error::Nsm)?;
        let fees = Amount::<NonNegative>::try_from(value.fees).map_err(|_| Error::Value)?;
        let mut next = self.clone();
        next.deferred = u64::try_from(i128::from(self.deferred) + deferred_delta)
            .ok()
            .filter(|v| *v <= crate::transaction::MAX_MONEY)
            .ok_or(Error::Deferred)?;
        next.issued = u64::try_from(
            i128::from(self.issued) + i128::from(value.coinbase_value) + deferred_delta
                - i128::from(value.fees),
        )
        .ok()
        .filter(|v| *v <= crate::transaction::MAX_MONEY)
        .ok_or(Error::Value)?;
        if subsidy::nsm_value_balance_is_tracked(height, &self.network) {
            let before = subsidy::nsm_value_balance_before(height, &self.network, parent);
            let reissued = subsidy::additional_block_subsidy(height, &self.network, before);
            let contributed = subsidy::nsm_fee_contribution(height, &self.network, fees);
            let reserve = (before - reissued)
                .and_then(|balance| balance + contributed)
                .map_err(|_| Error::Nsm)?;
            next.nsm = Some(u64::from(reserve));
        } else if self.nsm.is_some() {
            return Err(Error::Nsm);
        }
        if u128::from(next.issued) + u128::from(next.nsm.unwrap_or(0))
            > u128::from(crate::transaction::MAX_MONEY)
        {
            return Err(Error::Nsm);
        }
        Ok(next)
    }
    pub(crate) fn encode_state(
        &self,
        writer: &mut Writer,
        nsm: bool,
    ) -> zutti_protocol::Result<()> {
        if nsm != self.nsm.is_some() {
            return Err(CodecError::Invalid("monetary state version"));
        }
        writer.u8(if matches!(self.network, Network::Mainnet) {
            1
        } else {
            2
        });
        writer.u64(self.deferred);
        writer.u64(self.issued);
        if let Some(value) = self.nsm {
            writer.u64(value);
        }
        Ok(())
    }
    pub(crate) fn decode_state(reader: &mut Reader<'_>, nsm: bool) -> zutti_protocol::Result<Self> {
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(CodecError::Invalid("network")),
        };
        let mut state = Self::new(network).map_err(|_| CodecError::Invalid("monetary network"))?;
        state.deferred = reader.u64()?;
        state.issued = reader.u64()?;
        if nsm {
            state.nsm = Some(reader.u64()?);
        }
        if state.deferred > state.issued
            || state.deferred > crate::transaction::MAX_MONEY
            || u128::from(state.issued) + u128::from(state.nsm.unwrap_or(0))
                > u128::from(crate::transaction::MAX_MONEY)
        {
            return Err(CodecError::Invalid("monetary state"));
        }
        Ok(state)
    }
}
impl Canonical for MonetaryState {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIMON");
        writer.u16(1);
        writer.u8(u8::from(self.nsm.is_some()));
        self.encode_state(writer, self.nsm.is_some())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIMON" || reader.u16()? != 1 {
            return Err(CodecError::Version);
        }
        let nsm = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(CodecError::Invalid("monetary state version")),
        };
        Self::decode_state(reader, nsm)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use zebra_chain::{parameters::NetworkUpgrade, value_balance::ValueBalance};

    fn coinbase_block(
        state: &MonetaryState,
        height: u32,
        fees: u64,
        excess: u64,
    ) -> (ParsedBlock, BlockValue) {
        use transparent::{
            address::Script,
            bundle::{Authorized, Bundle, OutPoint, TxIn, TxOut},
        };
        use zcash_primitives::{
            block::{BlockHash, BlockHeaderData},
            transaction::{TransactionData, TxVersion},
        };
        use zcash_protocol::{
            consensus::{BlockHeight, BranchId, TEST_NETWORK},
            value::Zatoshis,
        };
        use zcash_script::{Opcode, pattern::push_num, script::Code};
        let expected = subsidy::block_subsidy_with_parent_nsm_value_balance(
            Height(height),
            &state.network,
            Amount::try_from(state.nsm.unwrap_or(0)).unwrap(),
        )
        .unwrap();
        let mut outputs = Vec::new();
        let mut reserved = 0_u64;
        for (recipient, value) in
            subsidy::funding_stream_values(Height(height), &state.network, expected).unwrap()
        {
            reserved += u64::from(value);
            if recipient != subsidy::FundingStreamReceiver::Deferred {
                let address =
                    subsidy::funding_stream_address(Height(height), &state.network, recipient)
                        .unwrap();
                outputs.push(TxOut::new(
                    Zatoshis::from_u64(u64::from(value)).unwrap(),
                    Script(Code(address.script().as_raw_bytes().to_vec())),
                ));
            }
        }
        let contribution = u64::from(subsidy::nsm_fee_contribution(
            Height(height),
            &state.network,
            Amount::try_from(fees).unwrap(),
        ));
        outputs.push(TxOut::new(
            Zatoshis::from_u64(u64::from(expected) + fees - contribution - reserved + excess)
                .unwrap(),
            Script(Code(vec![0x51])),
        ));
        let coinbase_value = outputs.iter().map(|o| o.value().into_u64()).sum();
        let transaction = TransactionData::<zcash_primitives::transaction::Authorized>::from_parts(
            TxVersion::V5,
            BranchId::for_height(&TEST_NETWORK, BlockHeight::from_u32(height)),
            0,
            BlockHeight::from_u32(height),
            Some(Bundle {
                vin: vec![TxIn::from_parts(
                    OutPoint::new([0; 32], u32::MAX),
                    Script(Code(Code::serialize(&[Opcode::PushValue(push_num(
                        i64::from(height),
                    ))]))),
                    u32::MAX,
                )],
                vout: outputs,
                authorization: Authorized,
            }),
            None,
            None,
            None,
        )
        .freeze()
        .unwrap();
        let header = BlockHeaderData {
            version: 4,
            prev_block: BlockHash([1; 32]),
            merkle_root: *transaction.txid().as_ref(),
            final_sapling_root: [0; 32],
            time: 1_700_000_000,
            bits: 0x2007ffff,
            nonce: [0; 32],
            solution: vec![0; 1344],
        }
        .freeze()
        .unwrap();
        let mut bytes = Vec::new();
        header.write(&mut bytes).unwrap();
        zcash_encoding::CompactSize::write(&mut bytes, 1).unwrap();
        transaction.write(&mut bytes).unwrap();
        (
            ParsedBlock::parse(&bytes, &TEST_NETWORK).unwrap(),
            BlockValue {
                fees,
                coinbase_value,
                sigops: 0,
            },
        )
    }

    #[test]
    fn coinbase_fee_split_and_parent_reserve_are_checked_in_complete_block_bodies() {
        let mut state = MonetaryState::new(NetworkType::Test).unwrap();
        state.issued = 1_000_000_000_000_000;
        let activation = NetworkUpgrade::Nu7
            .activation_height(&state.network)
            .unwrap()
            .0;
        for height in [activation - 1, activation] {
            let (block, value) = coinbase_block(&state, height, 100_003, 0);
            state.transition(&block, value).unwrap();
            let (inflated, value) = coinbase_block(&state, height, 100_003, 1);
            assert_eq!(
                state.transition(&inflated, value).unwrap_err(),
                Error::Value
            );
        }
        let (activation_block, value) = coinbase_block(&state, activation, 100_003, 0);
        let next = state.transition(&activation_block, value).unwrap();
        assert!(next.nsm().unwrap() > 0);
        let (block, value) = coinbase_block(&next, activation + 1, 100_003, 0);
        next.transition(&block, value).unwrap();
        assert_eq!(state.transition(&block, value).unwrap_err(), Error::Nsm);
        let deployment = next.network.nsm_reissuance_height().unwrap().0;
        let (block, value) = coinbase_block(&next, deployment.max(activation + 1), 100_003, 0);
        next.transition(&block, value).unwrap();
        let mut changed_parent = next.clone();
        changed_parent.nsm = Some(0);
        assert!(changed_parent.transition(&block, value).is_err());
    }

    #[test]
    fn nsm_activation_fee_rounding_and_reissuance_match_reference_accounting() {
        let mut state = MonetaryState::new(NetworkType::Test).unwrap();
        state.issued = 1_000_000_000_000_000;
        let activation = NetworkUpgrade::Nu7
            .activation_height(&state.network)
            .unwrap();
        let deployment = state.network.nsm_reissuance_height().unwrap();
        assert!(deployment >= activation);
        for height in [
            activation,
            Height(activation.0 + 1),
            deployment,
            Height(deployment.0 + 1),
        ] {
            for fees in [0_u64, 1, 2, 9, 10, 11, 100_003] {
                let mut previous = state.clone();
                if height != activation {
                    previous.nsm = Some(55_000_000_000);
                }
                let parent = Amount::<NonNegative>::try_from(previous.nsm.unwrap_or(0)).unwrap();
                let fees_amount = Amount::<NonNegative>::try_from(fees).unwrap();
                let mut pools = ValueBalance::zero();
                pools.set_nsm_amount(parent);
                let change =
                    subsidy::nsm_value_balance_change(height, &state.network, pools, fees_amount)
                        .unwrap();
                let expected_reserve =
                    u64::try_from(i128::from(u64::from(parent)) + i128::from(i64::from(change)))
                        .unwrap();
                let subsidy =
                    subsidy::block_subsidy_with_parent_pools(height, &state.network, pools)
                        .unwrap();
                let contribution = u64::from(subsidy::nsm_fee_contribution(
                    height,
                    &state.network,
                    fees_amount,
                ));
                assert_eq!(contribution, fees * 6 / 10);
                let value = BlockValue {
                    fees,
                    coinbase_value: u64::from(subsidy) + fees - contribution,
                    sigops: 0,
                };
                let next = previous.apply_value(height, value, 0).unwrap();
                assert_eq!(next.nsm(), Some(expected_reserve));
                assert_eq!(
                    next.issued(),
                    previous.issued() + value.coinbase_value - fees
                );
                next.validate_height(NetworkType::Test, height.0).unwrap();
                let decoded = MonetaryState::from_bytes(&next.to_bytes().unwrap()).unwrap();
                assert_eq!(decoded.nsm(), next.nsm());
                assert_eq!(decoded.issued(), next.issued());
            }
        }
    }

    #[test]
    fn monetary_presence_and_bounds_reject_invalid_snapshots() {
        let state = MonetaryState::new(NetworkType::Test).unwrap();
        let activation = NetworkUpgrade::Nu7
            .activation_height(&state.network)
            .unwrap()
            .0;
        assert!(
            state
                .validate_height(NetworkType::Test, activation - 1)
                .is_ok()
        );
        assert_eq!(
            state.validate_height(NetworkType::Test, activation),
            Err(Error::Nsm)
        );
        assert_eq!(
            state.validate_height(NetworkType::Main, 0),
            Err(Error::Network)
        );
        let mut active = state.clone();
        active.nsm = Some(0);
        assert_eq!(
            active.validate_height(NetworkType::Test, activation - 1),
            Err(Error::Nsm)
        );
        assert!(
            active
                .validate_height(NetworkType::Test, activation)
                .is_ok()
        );
        active.nsm = Some(1);
        active.issued = crate::transaction::MAX_MONEY;
        assert!(MonetaryState::from_bytes(&active.to_bytes().unwrap()).is_err());
        let mut deferred = state;
        deferred.deferred = 1;
        assert!(MonetaryState::from_bytes(&deferred.to_bytes().unwrap()).is_err());
    }
}

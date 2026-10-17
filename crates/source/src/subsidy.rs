use crate::{block::ParsedBlock, transaction::BlockValue};
use std::collections::BTreeMap;
use zcash_protocol::consensus::NetworkType;
use zebra_chain::{
    block::Height,
    parameters::{
        Network, NetworkUpgrade,
        subsidy::{self, FundingStreamReceiver, ParameterSubsidy},
    },
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
}
type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct MonetaryState {
    network: Network,
    deferred: u64,
    issued: u64,
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
        })
    }
    pub const fn deferred(&self) -> u64 {
        self.deferred
    }
    pub const fn issued(&self) -> u64 {
        self.issued
    }
    pub fn transition(&self, block: &ParsedBlock, value: BlockValue) -> Result<Self> {
        let height = Height(block.claimed_height());
        let subsidy =
            subsidy::block_subsidy(height, &self.network).map_err(|_| Error::Parameters)?;
        let upgrade = NetworkUpgrade::current(&self.network, height);
        let coinbase = block.transactions().first().ok_or(Error::Value)?;
        let mut outputs = BTreeMap::new();
        if let Some(bundle) = coinbase.transparent_bundle() {
            for output in &bundle.vout {
                *outputs
                    .entry((
                        output.value().into_u64(),
                        output.script_pubkey().0.0.clone(),
                    ))
                    .or_insert(0_usize) += 1;
            }
        }
        let mut require =
            |address: &zebra_chain::transparent::Address, amount: u64| -> Result<()> {
                let count = outputs
                    .get_mut(&(amount, address.script().as_raw_bytes().to_vec()))
                    .filter(|count| **count != 0)
                    .ok_or(Error::Recipient)?;
                *count -= 1;
                Ok(())
            };
        let mut deferred = 0_i128;
        if u64::from(subsidy) != 0 {
            if upgrade < NetworkUpgrade::Canopy {
                if height.0 != 0 && height < self.network.height_for_first_halving() {
                    let address = subsidy::founders_reward_address(&self.network, height)
                        .ok_or(Error::Parameters)?;
                    require(
                        &address,
                        u64::from(subsidy::founders_reward(&self.network, height)),
                    )?;
                }
            } else {
                for (receiver, amount) in
                    subsidy::funding_stream_values(height, &self.network, subsidy)
                        .map_err(|_| Error::Parameters)?
                {
                    if receiver == FundingStreamReceiver::Deferred {
                        deferred += i128::from(u64::from(amount));
                        continue;
                    }
                    let streams = self
                        .network
                        .funding_streams(height)
                        .ok_or(Error::Parameters)?;
                    let index = subsidy::funding_stream_address_period(height, &self.network)
                        .checked_sub(subsidy::funding_stream_address_period(
                            streams.height_range().start,
                            &self.network,
                        ))
                        .ok_or(Error::Parameters)?;
                    let address = streams
                        .recipient(receiver)
                        .and_then(|recipient| recipient.addresses().get(index as usize))
                        .ok_or(Error::Parameters)?;
                    require(address, u64::from(amount))?;
                }
                if NetworkUpgrade::Nu6_1.activation_height(&self.network) == Some(height) {
                    let disbursements = self.network.lockbox_disbursements(height);
                    if disbursements.is_empty() {
                        return Err(Error::Parameters);
                    }
                    for (address, amount) in disbursements {
                        require(&address, u64::from(amount))?;
                        deferred -= i128::from(u64::from(amount));
                    }
                }
            }
        }
        let output = i128::from(value.coinbase_value) + deferred;
        let input = i128::from(u64::from(subsidy)) + i128::from(value.fees);
        if output < 0
            || if upgrade < NetworkUpgrade::Nu6 {
                output > input
            } else {
                output != input
            }
        {
            return Err(Error::Value);
        }
        let mut next = self.clone();
        next.deferred = u64::try_from(i128::from(self.deferred) + deferred)
            .ok()
            .filter(|value| *value <= crate::transaction::MAX_MONEY)
            .ok_or(Error::Deferred)?;
        next.issued = u64::try_from(i128::from(self.issued) + output - i128::from(value.fees))
            .ok()
            .filter(|value| *value <= crate::transaction::MAX_MONEY)
            .ok_or(Error::Value)?;
        Ok(next)
    }
}

impl Canonical for MonetaryState {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.u8(if matches!(self.network, Network::Mainnet) {
            1
        } else {
            2
        });
        writer.u64(self.deferred);
        writer.u64(self.issued);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(CodecError::Invalid("network")),
        };
        let mut state = Self::new(network).map_err(|_| CodecError::Invalid("monetary network"))?;
        state.deferred = reader.u64()?;
        state.issued = reader.u64()?;
        if state.deferred > crate::transaction::MAX_MONEY
            || state.issued > crate::transaction::MAX_MONEY
        {
            return Err(CodecError::Invalid("monetary state"));
        }
        Ok(state)
    }
}

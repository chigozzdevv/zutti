use crate::{
    Error, Result,
    block::ParsedBlock,
    bootstrap::GenesisBlock,
    work::{Target, verify_header_work},
};
use num_bigint::BigUint;
use std::collections::VecDeque;
use zcash_primitives::block::BlockHeader;
use zcash_protocol::consensus::{
    BlockHeight, MAIN_NETWORK, NetworkType, NetworkUpgrade, Parameters, TEST_NETWORK,
};
use zutti_protocol::{
    Digest, Error as CodecError,
    encoding::{Canonical, Reader, Writer},
};

const AVERAGING_WINDOW: usize = 17;
const NU7_AVERAGING_WINDOW: usize = 102;
const MEDIAN_WINDOW: usize = 11;
const CONTEXT_WINDOW: usize = NU7_AVERAGING_WINDOW + MEDIAN_WINDOW;

#[derive(Debug, Clone)]
struct Header {
    height: u32,
    hash: Digest,
    time: u32,
    bits: u32,
}

#[derive(Debug, Clone)]
pub struct HeaderCandidate {
    network: NetworkType,
    headers: VecDeque<Header>,
    cumulative_work: BigUint,
}

impl HeaderCandidate {
    pub fn from_checkpoint_headers(
        network: NetworkType,
        height: u32,
        context: &[BlockHeader],
        cumulative_work: [u8; 32],
    ) -> Result<Self> {
        if network != NetworkType::Test
            || context.len() != (u64::from(height) + 1).min(CONTEXT_WINDOW as u64) as usize
            || height == 0
            || height >= 500_000_000
        {
            return Err(Error::Height);
        }
        let mut bytes = Writer::default();
        bytes.u8(2);
        bytes.u8(context.len() as u8);
        let first = height + 1 - context.len() as u32;
        let mut minimum_work = BigUint::from(0_u8);
        for (index, header) in context.iter().enumerate() {
            verify_header_work(header, network)?;
            if index > 0 && header.prev_block.0 != context[index - 1].hash().0 {
                return Err(Error::Height);
            }
            bytes.u32(first + index as u32);
            bytes.fixed(&header.hash().0);
            bytes.u32(header.time);
            bytes.u32(header.bits);
            minimum_work += work(header.bits)?;
        }
        if BigUint::from_bytes_be(&cumulative_work) < minimum_work {
            return Err(Error::Work);
        }
        bytes.fixed(&cumulative_work);
        Ok(Self::from_bytes(&bytes.finish())?)
    }
    pub fn from_genesis(genesis: &GenesisBlock) -> Result<Self> {
        let block = genesis.block();
        let header = Header {
            height: 0,
            hash: block.hash(),
            time: block.header().time,
            bits: block.header().bits,
        };
        let cumulative_work = work(header.bits)?;
        Ok(Self {
            network: genesis.network(),
            headers: VecDeque::from([header]),
            cumulative_work,
        })
    }
    pub fn from_genesis_header(header: &BlockHeader, network: NetworkType) -> Result<Self> {
        if header.prev_block.0 != [0; 32]
            || Digest(header.hash().0) != crate::bootstrap::genesis_hash(network)?
        {
            return Err(Error::Genesis);
        }
        Ok(Self {
            network,
            headers: VecDeque::from([Header {
                height: 0,
                hash: Digest(header.hash().0),
                time: header.time,
                bits: header.bits,
            }]),
            cumulative_work: work(header.bits)?,
        })
    }
    pub fn append(&mut self, block: &ParsedBlock) -> Result<()> {
        if block.claimed_height() != self.height()?.checked_add(1).ok_or(Error::Height)? {
            return Err(Error::Height);
        }
        self.append_header(block.header())
    }
    pub fn append_header(&mut self, header: &BlockHeader) -> Result<()> {
        let tip = self.headers.back().ok_or(Error::Genesis)?;
        let height = tip.height.checked_add(1).ok_or(Error::Height)?;
        if Digest(header.prev_block.0) != tip.hash {
            return Err(Error::Parent);
        }
        let time = header.time;
        let context: Vec<_> = self.headers.iter().rev().cloned().collect();
        let median = median(context.iter().take(MEDIAN_WINDOW).map(|h| h.time))?;
        let maximum_time_active = match self.network {
            NetworkType::Main => height >= 2,
            NetworkType::Test => height >= 653_606,
            NetworkType::Regtest => return Err(Error::Network),
        };
        if time <= median || (maximum_time_active && u64::from(time) > u64::from(median) + 5_400) {
            return Err(Error::Work);
        }
        if header.bits != expected_bits(self.network, height, time, &context)? {
            return Err(Error::Work);
        }
        verify_header_work(header, self.network)?;
        let cumulative_work = &self.cumulative_work + work(header.bits)?;
        if cumulative_work.bits() > 256 {
            return Err(Error::Work);
        }
        if self.headers.len() == CONTEXT_WINDOW {
            self.headers.pop_front();
        }
        self.headers.push_back(Header {
            height,
            hash: Digest(header.hash().0),
            time,
            bits: header.bits,
        });
        self.cumulative_work = cumulative_work;
        Ok(())
    }
    pub fn expected_bits(&self, time: u32) -> Result<u32> {
        let tip = self.headers.back().ok_or(Error::Genesis)?;
        let height = tip.height.checked_add(1).ok_or(Error::Height)?;
        let context: Vec<_> = self.headers.iter().rev().cloned().collect();
        expected_bits(self.network, height, time, &context)
    }
    pub fn cumulative_work(&self) -> Result<[u8; 32]> {
        let bytes = self.cumulative_work.to_bytes_be();
        if bytes.len() > 32 {
            return Err(Error::Work);
        }
        let mut encoded = [0; 32];
        encoded[32 - bytes.len()..].copy_from_slice(&bytes);
        Ok(encoded)
    }
    pub fn timestamp(&self) -> Result<u32> {
        Ok(self.headers.back().ok_or(Error::Genesis)?.time)
    }
    pub fn height(&self) -> Result<u32> {
        Ok(self.headers.back().ok_or(Error::Genesis)?.height)
    }
    pub fn tip(&self) -> Result<Digest> {
        Ok(self.headers.back().ok_or(Error::Genesis)?.hash)
    }
}

impl HeaderCandidate {
    pub const fn network(&self) -> NetworkType {
        self.network
    }
}
impl Canonical for HeaderCandidate {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.u8(match self.network {
            NetworkType::Main => 1,
            NetworkType::Test => 2,
            _ => return Err(CodecError::Invalid("network")),
        });
        writer.u8(self.headers.len() as u8);
        for header in &self.headers {
            writer.u32(header.height);
            writer.fixed(&header.hash.0);
            writer.u32(header.time);
            writer.u32(header.bits);
        }
        writer.fixed(
            &self
                .cumulative_work()
                .map_err(|_| CodecError::Invalid("chain work"))?,
        );
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(CodecError::Invalid("network")),
        };
        let count = reader.u8()? as usize;
        if count == 0 || count > CONTEXT_WINDOW {
            return Err(CodecError::Length);
        }
        let mut headers: VecDeque<Header> = VecDeque::new();
        for _ in 0..count {
            let header = Header {
                height: reader.u32()?,
                hash: Digest::nonzero(reader.fixed()?)?,
                time: reader.u32()?,
                bits: reader.u32()?,
            };
            if let Some(previous) = headers.back()
                && previous.height.checked_add(1) != Some(header.height)
            {
                return Err(CodecError::Invalid("header context"));
            }
            Target::from_compact(header.bits).map_err(|_| CodecError::Invalid("header target"))?;
            headers.push_back(header);
        }
        let tip = headers.back().ok_or(CodecError::Length)?;
        if count as u64 != (u64::from(tip.height) + 1).min(CONTEXT_WINDOW as u64) {
            return Err(CodecError::Length);
        }
        let cumulative_work = BigUint::from_bytes_be(&reader.fixed::<32>()?);
        if cumulative_work == BigUint::from(0_u8) {
            return Err(CodecError::Invalid("chain work"));
        }
        Ok(Self {
            network,
            headers,
            cumulative_work,
        })
    }
}

fn pow_limit(network: NetworkType) -> Result<BigUint> {
    let bits = match network {
        NetworkType::Main => 243,
        NetworkType::Test => 251,
        NetworkType::Regtest => return Err(Error::Network),
    };
    Ok((BigUint::from(1_u8) << bits) - BigUint::from(1_u8))
}

fn expand(bits: u32) -> Result<BigUint> {
    Ok(BigUint::from_bytes_le(
        &Target::from_compact(bits)?.little_endian(),
    ))
}

fn compact(target: &BigUint) -> Result<u32> {
    if target == &BigUint::from(0_u8) || target.bits() > 256 {
        return Err(Error::Work);
    }
    let mut size = u32::try_from(target.bits().div_ceil(8)).map_err(|_| Error::Work)?;
    let shifted = if size <= 3 {
        target << (8 * (3 - size))
    } else {
        target >> (8 * (size - 3))
    };
    let mut value = shifted.to_u32_digits().first().copied().unwrap_or(0);
    if value & 0x0080_0000 != 0 {
        value >>= 8;
        size += 1;
    }
    Ok((size << 24) | value)
}

fn work(bits: u32) -> Result<BigUint> {
    let target = expand(bits)?;
    let maximum = (BigUint::from(1_u8) << 256_u32) - BigUint::from(1_u8);
    Ok((&maximum - &target) / (&target + BigUint::from(1_u8)) + BigUint::from(1_u8))
}

fn median(times: impl Iterator<Item = u32>) -> Result<u32> {
    let mut times: Vec<_> = times.collect();
    if times.is_empty() || times.len() > MEDIAN_WINDOW {
        return Err(Error::Work);
    }
    times.sort_unstable();
    Ok(times[times.len() / 2])
}

fn expected_bits(network: NetworkType, height: u32, time: u32, context: &[Header]) -> Result<u32> {
    let previous = context.first().ok_or(Error::Genesis)?;
    if height != previous.height.checked_add(1).ok_or(Error::Height)?
        || context.len()
            != usize::try_from(height)
                .map_err(|_| Error::Height)?
                .min(CONTEXT_WINDOW)
        || context
            .windows(2)
            .any(|pair| pair[0].height.checked_sub(1) != Some(pair[1].height))
    {
        return Err(Error::Height);
    }
    let limit = pow_limit(network)?;
    let blossom = match network {
        NetworkType::Main => MAIN_NETWORK.activation_height(NetworkUpgrade::Blossom),
        NetworkType::Test => TEST_NETWORK.activation_height(NetworkUpgrade::Blossom),
        NetworkType::Regtest => return Err(Error::Network),
    }
    .ok_or(Error::Network)?;
    let nu7 = match network {
        NetworkType::Main => MAIN_NETWORK.activation_height(NetworkUpgrade::Nu7),
        NetworkType::Test => TEST_NETWORK.activation_height(NetworkUpgrade::Nu7),
        NetworkType::Regtest => return Err(Error::Network),
    };
    let (spacing, window) =
        if nu7.is_some_and(|activation| BlockHeight::from_u32(height) >= activation) {
            (25_i64, NU7_AVERAGING_WINDOW)
        } else if BlockHeight::from_u32(height) >= blossom {
            (75_i64, AVERAGING_WINDOW)
        } else {
            (150_i64, AVERAGING_WINDOW)
        };
    if network == NetworkType::Test
        && previous.height >= 299_187
        && u64::from(time) > u64::from(previous.time) + 6 * spacing as u64
    {
        return compact(&limit);
    }
    if context.len() <= window {
        return compact(&limit);
    }
    let mut sum = BigUint::from(0_u8);
    for header in context.iter().take(window) {
        let target = expand(header.bits)?;
        if target > limit {
            return Err(Error::Work);
        }
        sum += target;
    }
    let recent = i64::from(median(context.iter().take(MEDIAN_WINDOW).map(|h| h.time))?);
    let earlier = i64::from(median(
        context
            .iter()
            .skip(window)
            .take(MEDIAN_WINDOW)
            .map(|h| h.time),
    )?);
    let average = sum / BigUint::from(window);
    retarget(average, recent, earlier, spacing, window, limit)
}

fn retarget(
    average: BigUint,
    recent: i64,
    earlier: i64,
    spacing: i64,
    window: usize,
    limit: BigUint,
) -> Result<u32> {
    let span = window as i64 * spacing;
    let adjusted = (span + (recent - earlier - span) / 4).clamp(span * 84 / 100, span * 132 / 100);
    let target = average / BigUint::from(span as u64) * BigUint::from(adjusted as u64);
    compact(&target.min(limit))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn context(height: u32, spacing: u32, bits: u32) -> Vec<Header> {
        (0..height.min(CONTEXT_WINDOW as u32))
            .map(|offset| Header {
                height: height - 1 - offset,
                hash: Digest([1; 32]),
                time: 1_500_000_000 + (height - 1 - offset) * spacing,
                bits,
            })
            .collect()
    }

    #[test]
    fn retarget_matches_zcashd_consensus_vectors() {
        for (bits, recent, earlier, spacing, expected) in [
            (0x1d00ffff, 1_000_003_570, 1_000_000_000, 150, 0x1d011998),
            (0x1f07ffff, 1_233_061_996, 1_231_006_505, 150, 0x1f07ffff),
            (0x1c05a3f4, 100_000_917, 1_000_000_000, 150, 0x1c04bceb),
            (0x1c05a3f4, 1_000_000_458, 1_000_000_000, 75, 0x1c04bceb),
            (0x1c387f6f, 1_000_005_815, 1_000_000_000, 150, 0x1c4a93bb),
            (0x1c387f6f, 1_000_002_908, 1_000_000_000, 75, 0x1c4a93bb),
        ] {
            assert_eq!(
                retarget(
                    expand(bits).unwrap(),
                    recent,
                    earlier,
                    spacing,
                    AVERAGING_WINDOW,
                    pow_limit(NetworkType::Main).unwrap()
                )
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn compact_and_work_match_zebra() {
        use zebra_chain::work::difficulty::CompactDifficulty;
        for bits in [
            0x1f07ffff_u32,
            0x2007ffff,
            0x1d00ffff,
            0x1c03ffff,
            0x1b012345,
        ] {
            let reference =
                CompactDifficulty::from_bytes_in_display_order(&bits.to_be_bytes()).unwrap();
            assert_eq!(
                work(bits).unwrap(),
                BigUint::from(reference.to_work().unwrap().as_u128())
            );
            assert_eq!(
                compact(&expand(bits).unwrap()).unwrap().to_be_bytes(),
                reference
                    .to_expanded()
                    .unwrap()
                    .to_compact()
                    .bytes_in_display_order()
            );
        }
    }

    #[test]
    fn early_history_and_exact_testnet_exception() {
        let main = context(17, 150, 0x1f07ffff);
        assert_eq!(
            expected_bits(NetworkType::Main, 17, main[0].time + 150, &main).unwrap(),
            0x1f07ffff
        );
        let test = context(299_188, 150, 0x1d00ffff);
        assert_ne!(
            expected_bits(NetworkType::Test, 299_188, test[0].time + 900, &test).unwrap(),
            0x2007ffff
        );
        assert_eq!(
            expected_bits(NetworkType::Test, 299_188, test[0].time + 901, &test).unwrap(),
            0x2007ffff
        );
        let before = context(299_187, 150, 0x1d00ffff);
        assert_ne!(
            expected_bits(NetworkType::Test, 299_187, before[0].time + 901, &before).unwrap(),
            0x2007ffff
        );
    }

    #[test]
    fn retarget_uses_candidate_upgrade_spacing_and_complete_context() {
        let before = context(653_599, 150, 0x1d00ffff);
        let activation = context(653_600, 150, 0x1d00ffff);
        let first =
            expected_bits(NetworkType::Main, 653_599, before[0].time + 150, &before).unwrap();
        let second = expected_bits(
            NetworkType::Main,
            653_600,
            activation[0].time + 75,
            &activation,
        )
        .unwrap();
        assert_ne!(first, second);
        assert!(
            expected_bits(
                NetworkType::Main,
                653_600,
                activation[0].time + 75,
                &activation[..27]
            )
            .is_err()
        );
        assert_eq!(
            expected_bits(
                NetworkType::Regtest,
                653_600,
                activation[0].time + 75,
                &activation
            ),
            Err(Error::Network)
        );
    }

    #[test]
    fn nu7_retarget_uses_102_blocks_and_the_new_testnet_exception() {
        use zebra_chain::{
            block::Height,
            parameters::{Network, NetworkUpgrade as Upgrade},
        };
        let network = Network::new_default_testnet();
        let activation = Upgrade::Nu7.activation_height(&network).unwrap().0;
        for (height, spacing, window) in [
            (activation - 1, 75, AVERAGING_WINDOW),
            (activation, 25, NU7_AVERAGING_WINDOW),
            (activation + 1, 25, NU7_AVERAGING_WINDOW),
        ] {
            assert_eq!(
                Upgrade::target_spacing_for_height(&network, Height(height)).num_seconds(),
                spacing
            );
            assert_eq!(
                Upgrade::averaging_window_for_height(&network, Height(height)),
                window
            );
            let headers = context(height, spacing as u32, 0x1d00ffff);
            let reference = retarget(
                expand(0x1d00ffff).unwrap(),
                i64::from(median(headers.iter().take(11).map(|h| h.time)).unwrap()),
                i64::from(median(headers.iter().skip(window).take(11).map(|h| h.time)).unwrap()),
                spacing,
                window,
                pow_limit(NetworkType::Test).unwrap(),
            )
            .unwrap();
            assert_eq!(
                expected_bits(
                    NetworkType::Test,
                    height,
                    headers[0].time + spacing as u32,
                    &headers
                )
                .unwrap(),
                reference
            );
            assert_ne!(
                expected_bits(
                    NetworkType::Test,
                    height,
                    headers[0].time + 6 * spacing as u32,
                    &headers
                )
                .unwrap(),
                0x2007ffff
            );
            assert_eq!(
                expected_bits(
                    NetworkType::Test,
                    height,
                    headers[0].time + 6 * spacing as u32 + 1,
                    &headers
                )
                .unwrap(),
                0x2007ffff
            );
        }
        let mut headers = context(activation, 75, 0x1d00ffff);
        let expected = expected_bits(
            NetworkType::Test,
            activation,
            headers[0].time + 25,
            &headers,
        )
        .unwrap();
        headers[100].bits = 0x1d007fff;
        assert_ne!(
            expected_bits(
                NetworkType::Test,
                activation,
                headers[0].time + 25,
                &headers
            )
            .unwrap(),
            expected
        );
        assert!(
            expected_bits(
                NetworkType::Test,
                activation,
                headers[0].time + 25,
                &headers[..28]
            )
            .is_err()
        );
    }
}

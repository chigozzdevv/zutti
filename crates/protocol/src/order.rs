use crate::{
    Digest, Error, Result, Secret32,
    amount::{Amount, Asset, FeeBps},
    commitment::{commit, hash},
    domain::Domain,
    encoding::{Canonical, Reader, Writer},
    note::FixedOutput,
    policy::PaymentWindow,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    ZecToUsdc = 1,
    UsdcToZec = 2,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OrderTerms {
    domain: Domain,
    direction: Direction,
    offer: Digest,
    zec: Amount,
    usdc: Amount,
    receiver: [u8; 43],
    payout: FixedOutput,
    refund: FixedOutput,
    gross_output: Amount,
    fee_bps: FeeBps,
    fee_caps: [Amount; 3],
    window: PaymentWindow,
    nonce: Digest,
}

pub struct OrderInput {
    pub domain: Domain,
    pub direction: Direction,
    pub offer: Digest,
    pub zec: Amount,
    pub usdc: Amount,
    pub receiver: [u8; 43],
    pub payout: FixedOutput,
    pub refund: FixedOutput,
    pub gross_output: Amount,
    pub fee_bps: FeeBps,
    pub fee_caps: [Amount; 3],
    pub window: PaymentWindow,
    pub nonce: Digest,
}

impl core::fmt::Debug for OrderTerms {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OrderTerms([PRIVATE TERMS])")
    }
}
impl core::fmt::Debug for OrderInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OrderInput([PRIVATE TERMS])")
    }
}

impl OrderTerms {
    pub fn new(input: OrderInput) -> Result<Self> {
        let OrderInput {
            domain,
            direction,
            offer,
            zec,
            usdc,
            receiver,
            payout,
            refund,
            gross_output,
            fee_bps,
            fee_caps,
            window,
            nonce,
        } = input;
        Digest::nonzero(offer.0)?;
        Digest::nonzero(nonce.0)?;
        if zec.positive()?.asset() != Asset::Zec || usdc.positive()?.asset() != Asset::Usdc {
            return Err(Error::Asset);
        }
        if receiver == [0; 43] {
            return Err(Error::Invalid("shielded receiver"));
        }
        if payout.amount() != usdc
            || refund.amount() != usdc
            || payout.commitment() == refund.commitment()
        {
            return Err(Error::Invalid("fixed outputs"));
        }
        let expected = match direction {
            Direction::ZecToUsdc => usdc,
            Direction::UsdcToZec => zec,
        };
        if gross_output.net_after_fee(fee_bps) != expected {
            return Err(Error::Invalid("gross/net maker fee"));
        }
        for (cap, asset) in fee_caps.iter().zip([Asset::Zec, Asset::Eth, Asset::Usdc]) {
            if cap.asset() != asset {
                return Err(Error::Asset);
            }
        }
        Ok(Self {
            domain,
            direction,
            offer,
            zec,
            usdc,
            receiver,
            payout,
            refund,
            gross_output,
            fee_bps,
            fee_caps,
            window,
            nonce,
        })
    }
    pub fn commitment(&self, randomness: &Secret32) -> Result<Digest> {
        Ok(commit(
            b"order",
            self.domain.id()?,
            &self.to_bytes()?,
            randomness,
        ))
    }
    pub fn use_tag(&self) -> Result<Digest> {
        Ok(hash(b"order-use", &[&self.domain.id()?.0, &self.nonce.0]))
    }
    pub fn memo(&self, commitment: Digest) -> Result<[u8; 512]> {
        let mut memo = [0; 512];
        // ZIP 302 arbitrary-data memo: random commitments are not UTF-8 text.
        memo[0] = 0xff;
        memo[1..9].copy_from_slice(b"ZUTTIMEM");
        memo[9..11].copy_from_slice(&1_u16.to_le_bytes());
        memo[11..43].copy_from_slice(&self.domain.id()?.0);
        memo[43..75].copy_from_slice(&commitment.0);
        memo[75..107].copy_from_slice(&self.window.anchor_hash().0);
        Ok(memo)
    }
    pub fn domain(&self) -> &Domain {
        &self.domain
    }
    pub const fn direction(&self) -> Direction {
        self.direction
    }
    pub const fn zec(&self) -> Amount {
        self.zec
    }
    pub const fn usdc(&self) -> Amount {
        self.usdc
    }
    pub const fn receiver(&self) -> &[u8; 43] {
        &self.receiver
    }
    pub const fn payout(&self) -> FixedOutput {
        self.payout
    }
    pub const fn refund(&self) -> FixedOutput {
        self.refund
    }
    pub const fn window(&self) -> PaymentWindow {
        self.window
    }
    pub const fn source_fee_cap(&self) -> Amount {
        self.fee_caps[0]
    }
}
impl Canonical for OrderTerms {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTIORD");
        writer.u16(1);
        self.domain.encode(writer)?;
        writer.u8(self.direction as u8);
        writer.fixed(&self.offer.0);
        self.zec.encode(writer)?;
        self.usdc.encode(writer)?;
        writer.fixed(&self.receiver);
        self.payout.encode(writer)?;
        self.refund.encode(writer)?;
        self.gross_output.encode(writer)?;
        writer.u16(self.fee_bps.value());
        for cap in self.fee_caps {
            cap.encode(writer)?;
        }
        self.window.encode(writer)?;
        writer.fixed(&self.nonce.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIORD" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let domain = Domain::decode(reader)?;
        let direction = match reader.u8()? {
            1 => Direction::ZecToUsdc,
            2 => Direction::UsdcToZec,
            _ => return Err(Error::Invalid("direction")),
        };
        Self::new(OrderInput {
            domain,
            direction,
            offer: Digest(reader.fixed()?),
            zec: Amount::decode(reader)?,
            usdc: Amount::decode(reader)?,
            receiver: reader.fixed()?,
            payout: FixedOutput::decode(reader)?,
            refund: FixedOutput::decode(reader)?,
            gross_output: Amount::decode(reader)?,
            fee_bps: FeeBps::new(reader.u16()?)?,
            fee_caps: [
                Amount::decode(reader)?,
                Amount::decode(reader)?,
                Amount::decode(reader)?,
            ],
            window: PaymentWindow::decode(reader)?,
            nonce: Digest(reader.fixed()?),
        })
    }
}

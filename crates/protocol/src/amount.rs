use crate::{
    Error, Result,
    encoding::{Canonical, Reader, Writer},
};
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Asset {
    Zec = 1,
    Usdc = 2,
    Eth = 3,
}

impl Asset {
    pub const fn decimals(self) -> u32 {
        match self {
            Self::Zec => 8,
            Self::Usdc => 6,
            Self::Eth => 18,
        }
    }
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            1 => Ok(Self::Zec),
            2 => Ok(Self::Usdc),
            3 => Ok(Self::Eth),
            _ => Err(Error::Invalid("asset")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Amount {
    asset: Asset,
    units: u128,
}

impl Amount {
    pub fn from_units(asset: Asset, units: u128) -> Result<Self> {
        if asset == Asset::Zec && units > 2_100_000_000_000_000 {
            return Err(Error::Invalid("ZEC range"));
        }
        Ok(Self { asset, units })
    }
    pub const fn asset(self) -> Asset {
        self.asset
    }
    pub const fn units(self) -> u128 {
        self.units
    }
    pub fn positive(self) -> Result<Self> {
        if self.units == 0 {
            Err(Error::Invalid("zero principal"))
        } else {
            Ok(self)
        }
    }
    pub fn parse(asset: Asset, text: &str) -> Result<Self> {
        if text.is_empty() || text.len() > 80 || !text.is_ascii() {
            return Err(Error::Invalid("decimal amount"));
        }
        let mut parts = text.split('.');
        let whole = parts.next().ok_or(Error::Invalid("decimal amount"))?;
        let fraction = parts.next();
        if parts.next().is_some() || whole.is_empty() || (whole.len() > 1 && whole.starts_with('0'))
        {
            return Err(Error::Invalid("decimal amount"));
        }
        if !whole.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Invalid("decimal amount"));
        }
        let fraction = match fraction {
            Some("") => return Err(Error::Invalid("decimal amount")),
            Some(value) => value,
            None => "",
        };
        if fraction.len() > asset.decimals() as usize
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::Invalid("decimal precision"));
        }
        let digits = |s: &str| -> Result<u128> {
            s.bytes().try_fold(0_u128, |n, b| {
                n.checked_mul(10)
                    .and_then(|v| v.checked_add(u128::from(b - b'0')))
                    .ok_or(Error::Overflow)
            })
        };
        let scale = 10_u128.pow(asset.decimals());
        let fraction_scale = 10_u128.pow(asset.decimals() - fraction.len() as u32);
        let units = digits(whole)?
            .checked_mul(scale)
            .and_then(|v| {
                digits(fraction)
                    .ok()
                    .and_then(|f| f.checked_mul(fraction_scale))
                    .and_then(|f| v.checked_add(f))
            })
            .ok_or(Error::Overflow)?;
        Self::from_units(asset, units)
    }
    pub fn checked_add(self, other: Self) -> Result<Self> {
        if self.asset != other.asset {
            return Err(Error::Asset);
        }
        Self::from_units(
            self.asset,
            self.units.checked_add(other.units).ok_or(Error::Overflow)?,
        )
    }
    pub fn checked_sub(self, other: Self) -> Result<Self> {
        if self.asset != other.asset {
            return Err(Error::Asset);
        }
        Self::from_units(
            self.asset,
            self.units.checked_sub(other.units).ok_or(Error::Overflow)?,
        )
    }
    pub fn net_after_fee(self, fee: FeeBps) -> Self {
        let multiplier = u128::from(10_000 - fee.0);
        // Decompose before multiplying: even u128::MAX cannot overflow.
        let units =
            (self.units / 10_000) * multiplier + ((self.units % 10_000) * multiplier) / 10_000;
        Self {
            asset: self.asset,
            units,
        }
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = 10_u128.pow(self.asset.decimals());
        let whole = self.units / scale;
        let fraction = self.units % scale;
        if fraction == 0 {
            return write!(f, "{whole}");
        }
        let fraction = format!("{fraction:0width$}", width = self.asset.decimals() as usize);
        write!(f, "{whole}.{}", fraction.trim_end_matches('0'))
    }
}

impl Canonical for Amount {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(self.asset as u8);
        writer.u128(self.units);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::from_units(Asset::from_id(reader.u8()?)?, reader.u128()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeBps(u16);
impl FeeBps {
    pub fn new(value: u16) -> Result<Self> {
        if value >= 10_000 {
            Err(Error::Invalid("maker fee"))
        } else {
            Ok(Self(value))
        }
    }
    pub const fn value(self) -> u16 {
        self.0
    }
}

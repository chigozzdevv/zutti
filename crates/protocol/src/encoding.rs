use crate::{Error, Result};

#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn fixed(&mut self, value: &[u8]) {
        self.0.extend_from_slice(value);
    }
    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub fn u16(&mut self, value: u16) {
        self.fixed(&value.to_le_bytes());
    }
    pub fn u32(&mut self, value: u32) {
        self.fixed(&value.to_le_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.fixed(&value.to_le_bytes());
    }
    pub fn u128(&mut self, value: u128) {
        self.fixed(&value.to_le_bytes());
    }
    pub fn bytes(&mut self, value: &[u8]) -> Result<()> {
        self.u32(u32::try_from(value.len()).map_err(|_| Error::Length)?);
        self.fixed(value);
        Ok(())
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self.offset.checked_add(N).ok_or(Error::Length)?;
        let slice = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        let mut out = [0; N];
        out.copy_from_slice(slice);
        self.offset = end;
        Ok(out)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.fixed()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }
    pub fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_le_bytes(self.fixed()?))
    }
    pub fn bytes(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = usize::try_from(self.u32()?).map_err(|_| Error::Length)?;
        if len > max {
            return Err(Error::Length);
        }
        let end = self.offset.checked_add(len).ok_or(Error::Length)?;
        let out = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        self.offset = end;
        Ok(out)
    }
    pub fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::Trailing)
        }
    }
}

pub trait Canonical: Sized {
    fn encode(&self, writer: &mut Writer) -> Result<()>;
    fn decode(reader: &mut Reader<'_>) -> Result<Self>;
    fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut writer = Writer::default();
        self.encode(&mut writer)?;
        Ok(writer.finish())
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let value = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(value)
    }
}

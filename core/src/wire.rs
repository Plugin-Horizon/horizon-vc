//! Big-endian buffers as the voice protocol writes them (Minecraft's conventions:
//! VarInt lengths, UTF-8 strings, UUIDs as two big-endian longs).

#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(v as u8)
    }
    pub fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn varint(&mut self, v: i32) -> &mut Self {
        let mut n = v as u32;
        loop {
            let b = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                self.0.push(b);
                return self;
            }
            self.0.push(b | 0x80);
        }
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.varint(v.len() as i32);
        self.0.extend_from_slice(v);
        self
    }
    pub fn string(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }
    pub fn uuid(&mut self, v: u128) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn opt_uuid(&mut self, v: Option<u128>) -> &mut Self {
        self.bool(v.is_some());
        if let Some(v) = v {
            self.uuid(v);
        }
        self
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

pub type Result<T> = std::result::Result<T, &'static str>;

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or("length overflow")?;
        let out = self.data.get(self.at..end).ok_or("packet too short")?;
        self.at = end;
        Ok(out)
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.at..]
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn varint(&mut self) -> Result<i32> {
        let mut n = 0u32;
        for i in 0..5 {
            let b = self.u8()?;
            n |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(n as i32);
            }
        }
        Err("varint too long")
    }
    pub fn bytes(&mut self, max: usize) -> Result<&'a [u8]> {
        let n = usize::try_from(self.varint()?).map_err(|_| "negative length")?;
        if n > max {
            return Err("byte array too long");
        }
        self.take(n)
    }
    pub fn string(&mut self, max: usize) -> Result<String> {
        let b = self.bytes(max * 4)?;
        let s = std::str::from_utf8(b).map_err(|_| "string is not UTF-8")?;
        if s.chars().count() > max {
            return Err("string too long");
        }
        Ok(s.to_owned())
    }
    pub fn uuid(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.take(16)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for v in [0, 1, 127, 128, 300, 25_565, i32::MAX, -1] {
            let mut w = Writer::default();
            w.varint(v);
            assert_eq!(Reader::new(&w.0).varint(), Ok(v));
        }
    }

    #[test]
    fn short_input_is_an_error() {
        assert!(Reader::new(&[0, 0, 0]).i32().is_err());
        assert!(Reader::new(&[5, 1, 2]).bytes(10).is_err());
        assert!(Reader::new(&[5, 1, 2, 3, 4, 5]).bytes(4).is_err());
    }
}

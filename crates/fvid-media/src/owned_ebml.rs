//! Owned bounded EBML framing and scalar values; shared with native Matroska.
use std::io::{Read, Seek, SeekFrom};
#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error(message.into())
}
include!("owned_ebml_impl.rs");
impl Element {
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn data_offset(&self) -> u64 {
        self.data
    }
    pub fn end_offset(&self) -> Option<u64> {
        self.end
    }
    pub fn read_bytes(self, source: &mut (impl Read + Seek), limit: usize) -> Result<Vec<u8>> {
        bytes(source, self, limit)
    }
    pub fn read_uint(self, source: &mut (impl Read + Seek)) -> Result<u64> {
        uint(source, self)
    }
    pub fn read_sint(self, source: &mut (impl Read + Seek)) -> Result<i64> {
        sint(source, self)
    }
    pub fn read_float(self, source: &mut (impl Read + Seek)) -> Result<f64> {
        float(source, self)
    }
}
/// Shared element admission count. A framing error ends this parser instance.
pub struct Budget {
    count: usize,
    limit: usize,
    failed: bool,
}
impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            count: 0,
            limit,
            failed: false,
        }
    }
    pub fn next(&mut self, source: &mut (impl Read + Seek), parent_end: u64) -> Result<Element> {
        if self.failed || self.count >= self.limit {
            return Err(invalid("WebM element limit exceeded"));
        }
        let result = element(source, parent_end, &mut self.count, self.limit);
        self.failed = result.is_err();
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn bounded_values_preserve_offsets_and_stop_before_next_header() {
        let mut source = Cursor::new([0x81, 0x82, 1, 2]);
        let mut budget = Budget::new(1);
        let value = budget.next(&mut source, 4).unwrap();
        assert_eq!(
            (value.id(), value.data_offset(), value.end_offset()),
            (0x81, 2, Some(4))
        );
        assert_eq!(value.read_uint(&mut source).unwrap(), 258);
        assert!(value.read_bytes(&mut source, 1).is_err());
        let before = source.position();
        assert!(budget.next(&mut source, 4).is_err());
        assert_eq!(source.position(), before);
    }
    #[test]
    fn malformed_headers_unknown_values_and_parent_overflow_are_distinct() {
        for data in [vec![0, 0], vec![0x81], vec![0x81, 0x85]] {
            let length = data.len() as u64;
            let mut source = Cursor::new(data);
            assert!(Budget::new(10).next(&mut source, length).is_err());
        }
        let mut source = Cursor::new([0x81, 0xff]);
        let value = Budget::new(10).next(&mut source, 2).unwrap();
        assert_eq!(value.end_offset(), None);
        assert!(
            value
                .read_bytes(&mut source, 8)
                .unwrap_err()
                .to_string()
                .contains("unknown size")
        );
    }
}

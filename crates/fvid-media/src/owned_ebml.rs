//! Owned bounded EBML framing and scalar values; shared with native Matroska.
use crate::owned_file_tags::FileTags;
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
include!("owned_matroska_metadata_read_impl.rs");

fn read_tags<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut FileTags,
) {
    read_tags_collect(r, e, count, max, out, &mut |_, _, _| {});
}
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
    /// Read file-wide tags using the same admission counter as framing.
    /// Malformed optional metadata preserves the native reader's lenient policy.
    pub fn read_file_tags(&mut self, source: &mut (impl Read + Seek), value: Element) -> FileTags {
        let mut tags = FileTags::default();
        if !self.failed && self.count < self.limit {
            read_tags(source, value, &mut self.count, self.limit, &mut tags);
        }
        tags
    }
    /// Read flat chapter atoms; times remain in the reader's original units.
    pub fn read_chapter_atoms(
        &mut self,
        source: &mut (impl Read + Seek),
        value: Element,
    ) -> Vec<(u64, Option<u64>, String)> {
        let mut chapters = Vec::new();
        if !self.failed && self.count < self.limit {
            read_chapters(source, value, &mut self.count, self.limit, &mut chapters);
        }
        chapters
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
    fn library_reads_synthetic_file_tags_and_chapter_atoms() {
        fn node(id: &[u8], payload: &[u8]) -> Vec<u8> {
            assert!(payload.len() < 127);
            [id, &[0x80 | payload.len() as u8], payload].concat()
        }
        let name = node(&[0x45, 0xa3], b"TITLE");
        let value = node(&[0x44, 0x87], b"Synthetic title");
        let simple = node(&[0x67, 0xc8], &[name, value].concat());
        let tag = node(&[0x73, 0x73], &simple);
        let tags = node(&[0x12, 0x54, 0xc3, 0x67], &tag);
        let mut source = Cursor::new(tags);
        let length = source.get_ref().len() as u64;
        let mut budget = Budget::new(32);
        let master = budget.next(&mut source, length).unwrap();
        assert_eq!(
            budget.read_file_tags(&mut source, master).title,
            "Synthetic title"
        );
        let start = node(&[0x91], &[5]);
        let end = node(&[0x92], &[9]);
        let display = node(&[0x80], &node(&[0x85], b"Chapter"));
        let atom = node(&[0xb6], &[start, end, display].concat());
        let edition = node(&[0x45, 0xb9], &atom);
        let chapters = node(&[0x10, 0x43, 0xa7, 0x70], &edition);
        let mut source = Cursor::new(chapters);
        let length = source.get_ref().len() as u64;
        let master = budget.next(&mut source, length).unwrap();
        assert_eq!(
            budget.read_chapter_atoms(&mut source, master),
            vec![(5, Some(9), "Chapter".into())]
        );
    }
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

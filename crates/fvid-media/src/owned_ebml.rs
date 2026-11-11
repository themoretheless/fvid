//! Owned bounded EBML framing and scalar values; shared with native Matroska.
use crate::owned_file_tags::FileTags;
pub use fvid_control::error::{Error, Result};
pub(crate) use fvid_control::error::invalid;
use std::io::{Read, Seek, SeekFrom};
#[derive(Clone, Copy)]
pub struct Element {
    pub(crate) id: u32,
    pub(crate) data: u64,
    pub(crate) end: Option<u64>,
}
pub(crate) fn vint(r: &mut impl Read, id: bool) -> Result<(u64, bool)> {
    let mut first = [0];
    r.read_exact(&mut first)?;
    if first[0] == 0 {
        return Err(invalid("zero EBML variable integer"));
    }
    let len = first[0].leading_zeros() as usize + 1;
    if len > if id { 4 } else { 8 } {
        return Err(invalid("oversized EBML variable integer"));
    }
    let mut value = u64::from(if id {
        first[0]
    } else {
        first[0] & ((1u8 << (8 - len)) - 1)
    });
    for _ in 1..len {
        let mut b = [0];
        r.read_exact(&mut b)?;
        value = (value << 8) | u64::from(b[0]);
    }
    Ok((value, !id && value == (1u64 << (7 * len)) - 1))
}
/// Stand at `pos` on the way past it rather than by starting again. A walk moves
/// from one element to the next, and the bytes in between are the payload it has
/// just measured: reading them costs their own length, while a seek costs the
/// buffer behind the reader its whole capacity and the next read fetches the
/// same bytes back. Only a gap longer than any block the reader steps over, or a
/// move backwards, is a jump worth the seek.
const SKIP: u64 = 8 << 20;
pub(crate) fn goto<R: Read + Seek>(r: &mut R, pos: u64) -> Result<()> {
    let here = r.stream_position()?;
    if pos == here {
        return Ok(());
    }
    if pos > here && pos - here <= SKIP {
        let mut buf = [0u8; 1 << 13];
        let mut left = pos - here;
        while left > 0 {
            let want = (left as usize).min(buf.len());
            // A short read is the source answering less than asked, not an end:
            // no bytes at all means the item stopped before the element its own
            // size promised.
            let taken = match r.read(&mut buf[..want]) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if taken == 0 {
                return Err(invalid("truncated EBML element"));
            }
            left -= taken as u64;
        }
        return Ok(());
    }
    r.seek(SeekFrom::Start(pos))?;
    Ok(())
}
pub(crate) fn element<R: Read + Seek>(
    r: &mut R,
    limit: u64,
    count: &mut usize,
    max: usize,
) -> Result<Element> {
    *count += 1;
    if *count > max {
        return Err(invalid("WebM element limit exceeded"));
    }
    let (id, _) = vint(r, true)?;
    let (size, unknown) = vint(r, false)?;
    let data = r.stream_position()?;
    if data > limit {
        return Err(invalid("truncated EBML element header"));
    }
    let end = if unknown {
        None
    } else {
        Some(
            data.checked_add(size)
                .filter(|&n| n <= limit)
                .ok_or_else(|| invalid("EBML element exceeds parent"))?,
        )
    };
    Ok(Element {
        id: id as u32,
        data,
        end,
    })
}
pub(crate) fn bytes<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<Vec<u8>> {
    let end = e
        .end
        .ok_or_else(|| invalid("unknown size for EBML value"))?;
    let len = usize::try_from(end - e.data)
        .ok()
        .filter(|&n| n <= max)
        .ok_or_else(|| invalid("EBML value exceeds limit"))?;
    r.seek(SeekFrom::Start(e.data))?;
    let mut out = vec![0; len];
    r.read_exact(&mut out)?;
    Ok(out)
}
/// RFC 8794 section 13: the first NUL ends the value; later bytes are padding.
pub(crate) fn terminated_text_bytes(mut value: Vec<u8>) -> Vec<u8> {
    if let Some(end) = value.iter().position(|&byte| byte == 0) {
        value.truncate(end);
    }
    value
}
/// Validate UTF-8 only within the semantic string, before its first NUL.
pub(crate) fn text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Result<String> {
    String::from_utf8(terminated_text_bytes(bytes(r, e, max)?))
        .map_err(|_| invalid("invalid WebM text"))
}
pub(crate) fn uint<R: Read + Seek>(r: &mut R, e: Element) -> Result<u64> {
    Ok(bytes(r, e, 8)?
        .into_iter()
        .fold(0, |v, b| (v << 8) | u64::from(b)))
}
pub(crate) fn sint<R: Read + Seek>(r: &mut R, e: Element) -> Result<i64> {
    let value = bytes(r, e, 8)?;
    let mut signed = if value.first().is_some_and(|byte| byte & 0x80 != 0) { -1i64 } else { 0 };
    for byte in value { signed = (signed << 8) | i64::from(byte); }
    Ok(signed)
}
/// Timestamps and rates are IEEE-754, written as either four or eight bytes.
pub(crate) fn float<R: Read + Seek>(r: &mut R, e: Element) -> Result<f64> {
    let value = bytes(r, e, 8)?;
    Ok(match value.as_slice() {
        [a, b, c, d] => f64::from(f32::from_be_bytes([*a, *b, *c, *d])),
        [a, b, c, d, e0, e1, e2, e3] => f64::from_be_bytes([*a, *b, *c, *d, *e0, *e1, *e2, *e3]),
        _ => return Err(invalid("invalid WebM float size")),
    })
}
pub(crate) fn end(e: Element) -> Result<u64> {
    e.end
        .ok_or_else(|| invalid("unsupported unknown-sized EBML element"))
}
pub(crate) fn fields<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Result<Vec<Element>> {
    let limit = end(e)?;
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        goto(r, at)?;
        let child = element(r, limit, count, max)?;
        at = end(child)?;
        out.push(child);
    }
    Ok(out)
}
/// The words a text element holds, with an encoding this reader cannot spell
/// left as the characters it happens to decode to rather than as a file it
/// refuses: a name is worth showing however it was written, and a file with a
/// badly coded one still plays.
pub(crate) fn lenient_text<R: Read + Seek>(r: &mut R, e: Element, max: usize) -> Option<String> {
    Some(
        String::from_utf8_lossy(&terminated_text_bytes(bytes(r, e, max).ok()?))
            .into_owned(),
    )
}
/// The children of a master up to the first one this reader cannot read, which
/// `fields` would take as the whole list being bad. A `Tags` master is where
/// this is needed: some muxers rewrite it in place and leave a byte of padding
/// behind the last tag, and the tags standing before it are still the file's.
pub(crate) fn fields_to_first_gap<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
) -> Vec<Element> {
    let Ok(limit) = end(e) else {
        return Vec::new();
    };
    let mut at = e.data;
    let mut out = Vec::new();
    while at < limit {
        if r.seek(SeekFrom::Start(at)).is_err() {
            break;
        }
        let Ok(child) = element(r, limit, count, max) else {
            break;
        };
        let Ok(next) = end(child) else {
            break;
        };
        out.push(child);
        at = next;
    }
    out
}
/// What the whole file says about itself among its `Tags`: the `SimpleTag`
/// values of a `Tag` whose `Targets` single out no track, edition, chapter or
/// attachment. A list this reader cannot walk, or one that names nothing of the
/// kind, leaves the file stating nothing instead of failing it — as does a tag
/// of a name this player has no line for.
pub(crate) fn read_tags_collect<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut FileTags,
    collect: &mut impl FnMut(Option<u64>, &str, &str),
) -> bool {
    let mut complete = true;
    for tag in fields_to_first_gap(r, e, count, max) {
        if tag.id != 0x7373 {
            continue;
        }
        let Ok(parts) = fields(r, tag, count, max) else {
            complete = false;
            continue;
        };
        // Either order the two parts come in is settled before the tag is kept,
        // so a `SimpleTag` written before its `Targets` still has them.
        let mut names_a_part = false;
        let mut track_uid = None;
        let mut unsupported_scope = false;
        let mut stated: Vec<(String, String)> = Vec::new();
        for part in parts {
            match part.id {
                0x63c0 => {
                    let entries = match fields(r, part, count, max) {
                        Ok(entries) => entries,
                        Err(_) => {
                            complete = false;
                            continue;
                        }
                    };
                    for entry in entries {
                        match entry.id {
                            0x63c5 => {
                                names_a_part = true;
                                match uint(r, entry) {
                                    Ok(uid) if track_uid.is_none() => track_uid = Some(uid),
                                    _ => unsupported_scope = true,
                                }
                            }
                            0x63c4 | 0x63c6 | 0x63c9 => {
                                names_a_part = true;
                                unsupported_scope = true;
                            }
                            _ => {}
                        }
                    }
                }
                0x67c8 => {
                    let Ok(fields_of_tag) = fields(r, part, count, max) else {
                        complete = false;
                        continue;
                    };
                    let (mut name, mut value) = (String::new(), String::new());
                    for field in fields_of_tag {
                        match field.id {
                            0x45a3 => match lenient_text(r, field, 128) {
                                Some(text) => name = text,
                                None => complete = false,
                            },
                            0x4487 => match lenient_text(r, field, 1024) {
                                Some(text) => value = text,
                                None => complete = false,
                            },
                            0x67c8 => complete = false,
                            _ => {}
                        }
                    }
                    if !name.is_empty() && !value.is_empty() {
                        stated.push((name, value));
                    }
                }
                _ => {}
            }
        }
        if unsupported_scope {
            complete = false;
        }
        if !names_a_part || (track_uid.is_some() && !unsupported_scope) {
            for (name, value) in stated {
                if !names_a_part {
                    out.insert(&name, &value);
                }
                collect(track_uid, &name, &value);
            }
        }
    }
    complete
}

/// The chapter atoms of one `Chapters` master, kept in the units the file's
/// `TimestampScale` states so the scale — which the `Info` may still be to come
/// by — can be applied later. A list this reader cannot walk leaves no
/// chapters at all rather than failing the file, which plays fine without them.
pub(crate) fn read_chapters<R: Read + Seek>(
    r: &mut R,
    e: Element,
    count: &mut usize,
    max: usize,
    out: &mut Vec<(u64, Option<u64>, String)>,
) {
    // A master may carry no chapters at all rather than a bad one, so an
    // unknown-sized parent or a truncated list simply yields nothing.
    let Ok(editions) = fields(r, e, count, max) else {
        return;
    };
    for edition in editions {
        if edition.id != 0x45b9 {
            continue;
        }
        let Ok(atoms) = fields(r, edition, count, max) else {
            return;
        };
        for atom in atoms {
            if atom.id != 0xb6 {
                continue;
            }
            if out.len() >= 1_024 {
                return;
            }
            let Ok(entries) = fields(r, atom, count, max) else {
                return;
            };
            let mut start = None;
            let mut end = None;
            let mut title = String::new();
            for field in entries {
                match field.id {
                    0x91 => start = uint(r, field).ok(),
                    0x92 => end = uint(r, field).ok(),
                    // A chapter may be displayed in several languages; what the
                    // file leads with is what a player has to show.
                    0x80 if title.is_empty() => {
                        if let Ok(texts) = fields(r, field, count, max) {
                            for text in texts {
                                if text.id != 0x85 {
                                    continue;
                                }
                                if let Ok(bytes) = bytes(r, text, 1024) {
                                    title = String::from_utf8_lossy(&terminated_text_bytes(bytes))
                                        .into_owned();
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(start) = start {
                out.push((start, end, title));
            }
        }
    }
}

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
    pub fn read_text(self, source: &mut (impl Read + Seek), limit: usize) -> Result<String> {
        text(source, self, limit)
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

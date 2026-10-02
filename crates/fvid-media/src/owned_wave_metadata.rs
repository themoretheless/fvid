//! Owned RIFF INFO metadata edits. Untouched entries retain their original bytes.
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, String>;
/// Common semantic keys, or an explicit printable four-byte RIFF INFO tag.
pub fn tag_for_key(key: &str) -> Result<[u8; 4]> {
    let tag = match key.to_ascii_lowercase().as_str() {
        "title" => *b"INAM",
        "artist" => *b"IART",
        "comment" => *b"ICMT",
        "date" => *b"ICRD",
        "album" => *b"IPRD",
        "genre" => *b"IGNR",
        "copyright" => *b"ICOP",
        "encoder" => *b"ISFT",
        "track" => *b"ITRK",
        _ if key.len() == 4 && key.bytes().all(|byte| (32..=126).contains(&byte)) => {
            let bytes: [u8; 4] = key.as_bytes().try_into().unwrap();
            bytes.map(|byte| byte.to_ascii_uppercase())
        }
        _ => return Err(format!("unsupported WAVE INFO metadata key: {key}")),
    };
    Ok(tag)
}
/// Deletions run first; the last assignment to a key wins.
pub fn edit_info_chunks(
    chunks: &[u8],
    deletes: &[String],
    sets: &[(String, String)],
) -> Result<Vec<u8>> {
    if deletes.len() + sets.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    if deletes.is_empty() && sets.is_empty() {
        return Ok(chunks.to_vec());
    }
    let mut changed = BTreeSet::new();
    let mut assigned = BTreeMap::new();
    for key in deletes {
        changed.insert(tag_for_key(key)?);
    }
    for (key, value) in sets {
        if value.contains('\0') {
            return Err("embedded NUL in metadata value".into());
        }
        let tag = tag_for_key(key)?;
        changed.insert(tag);
        assigned.insert(tag, value);
    }
    let mut entries = Vec::new();
    let mut at = 0usize;
    while at < chunks.len() {
        let header = chunks
            .get(at..at.checked_add(12).ok_or("WAVE INFO offset overflow")?)
            .ok_or("truncated WAVE INFO list")?;
        if &header[..4] != b"LIST" || &header[8..12] != b"INFO" {
            return Err("WAVE metadata is not an INFO list".into());
        }
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if size < 4 {
            return Err("short WAVE INFO list".into());
        }
        let end = at
            .checked_add(8)
            .and_then(|n| n.checked_add(size))
            .ok_or("WAVE INFO size overflow")?;
        let next = end.checked_add(size & 1).ok_or("WAVE INFO size overflow")?;
        if next > chunks.len() {
            return Err("WAVE INFO list exceeds chunk buffer".into());
        }
        let mut cursor = at + 12;
        while cursor < end {
            let header = chunks
                .get(cursor..cursor.checked_add(8).ok_or("WAVE INFO offset overflow")?)
                .filter(|_| end - cursor >= 8)
                .ok_or("truncated WAVE INFO entry")?;
            let tag: [u8; 4] = header[..4].try_into().unwrap();
            let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            let entry_end = cursor
                .checked_add(8)
                .and_then(|n| n.checked_add(size))
                .and_then(|n| n.checked_add(size & 1))
                .ok_or("WAVE INFO entry size overflow")?;
            if entry_end > end {
                return Err("WAVE INFO entry exceeds list".into());
            }
            if !changed.contains(&tag.map(|byte| byte.to_ascii_uppercase())) {
                entries.extend_from_slice(&chunks[cursor..entry_end]);
            }
            cursor = entry_end;
        }
        at = next;
    }
    for (tag, value) in assigned {
        let size = u32::try_from(value.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("WAVE INFO value is too large")?;
        entries.extend_from_slice(&tag);
        entries.extend_from_slice(&size.to_le_bytes());
        entries.extend_from_slice(value.as_bytes());
        entries.push(0);
        if size & 1 != 0 {
            entries.push(0);
        }
    }
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let size = u32::try_from(entries.len())
        .ok()
        .and_then(|n| n.checked_add(4))
        .ok_or("WAVE INFO list is too large")?;
    let mut result = Vec::new();
    result.extend_from_slice(b"LIST");
    result.extend_from_slice(&size.to_le_bytes());
    result.extend_from_slice(b"INFO");
    result.extend(entries);
    if size & 1 != 0 {
        result.push(0);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_preserve_opaque_entries_and_assignment_wins_after_deletion() {
        let mut source = b"LIST".to_vec();
        source.extend_from_slice(&30u32.to_le_bytes());
        source.extend_from_slice(b"INFOINAM");
        source.extend_from_slice(&6u32.to_le_bytes());
        source.extend_from_slice(b"title\0");
        source.extend_from_slice(b"QWER");
        source.extend_from_slice(&3u32.to_le_bytes());
        source.extend_from_slice(b"abc\xee");
        assert_eq!(edit_info_chunks(&source, &[], &[]).unwrap(), source);
        let edited = edit_info_chunks(
            &source,
            &["TITLE".into()],
            &[
                ("title".into(), "first".into()),
                ("title".into(), "last".into()),
            ],
        )
        .unwrap();
        let opaque = b"QWER\x03\x00\x00\x00abc\xee";
        assert!(edited.windows(opaque.len()).any(|bytes| bytes == opaque));
        assert!(edited.windows(5).any(|bytes| bytes == b"last\0"));
        assert!(!edited.windows(6).any(|bytes| bytes == b"title\0"));
        assert!(!edited.windows(6).any(|bytes| bytes == b"first\0"));
        assert!(
            edit_info_chunks(&source, &["title".into(), "QWER".into()], &[])
                .unwrap()
                .is_empty()
        );
        assert!(edit_info_chunks(&source[..source.len() - 1], &["title".into()], &[]).is_err());
        assert!(edit_info_chunks(&[], &[], &[("title".into(), "a\0b".into())]).is_err());
        assert!(tag_for_key("unrepresentable-key").is_err());
        assert!(edit_info_chunks(&[], &vec!["title".into(); 65], &[]).is_err());
    }
}

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
/// Assignments run first; deletions run after assignments and remove any matching key.
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
    for (key, value) in sets {
        if value.contains('\0') {
            return Err("embedded NUL in metadata value".into());
        }
        let tag = tag_for_key(key)?;
        changed.insert(tag);
        assigned.insert(tag, value);
    }
    for key in deletes {
        let tag = tag_for_key(key)?;
        changed.insert(tag);
        assigned.remove(&tag);
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

/// Build RIFF `LIST`/`INFO` chunks from container `FileTags`, track title/language,
/// optional writing application (`encoder`), and extra container/track tags.
pub fn info_for_file_tags(
    tags: &crate::owned_file_tags::FileTags,
    track_name: &str,
    track_language: &str,
    encoder: Option<&str>,
    extra: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    if let Some(enc) = encoder.filter(|s| !s.is_empty() && !s.contains('\0')) {
        entries.push(("encoder".into(), enc.to_owned()));
    }
    for (key, value) in extra {
        if !value.is_empty() && !value.contains('\0') && tag_for_key(key).is_ok() {
            entries.push((key.clone(), value.clone()));
        }
    }
    for (key, value) in [
        ("title", &tags.title),
        ("artist", &tags.artist),
        ("comment", &tags.comment),
        ("date", &tags.date),
        ("album", &tags.album),
        ("genre", &tags.genre),
        ("copyright", &tags.copyright),
        ("track", &tags.track),
    ] {
        if !value.is_empty() && !value.contains('\0') {
            entries.push((key.into(), value.clone()));
        }
    }
    if tags.title.is_empty() && !track_name.is_empty() && !track_name.contains('\0') {
        entries.push(("title".into(), track_name.to_owned()));
    }
    if !track_language.is_empty()
        && !track_language.eq_ignore_ascii_case("und")
        && !track_language.contains('\0')
    {
        entries.push(("ILNG".into(), track_language.to_owned()));
    }
    edit_info_chunks(&[], &[], &entries)
}

pub(crate) fn mp4_source_info(
    source: &std::path::Path,
    options: &fvid_control::CopyOptions,
) -> Result<Vec<u8>> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let metadata_bytes = crate::owned_mp4::Limits::default()
        .metadata_bytes
        .min(options.max_controlled_bytes.unwrap_or(usize::MAX));
    let reader = crate::owned_mp4::Mp4Reader::open_indexed(
        std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?),
        crate::owned_mp4::Limits {
            metadata_bytes,
            ..Default::default()
        },
    )
    .map_err(|error| {
        if metadata_bytes < crate::owned_mp4::Limits::default().metadata_bytes
            && error.to_string().contains("MP4 metadata exceeds budget")
        {
            "controlled memory budget exceeded: MP4 metadata exceeds budget".to_owned()
        } else {
            error.to_string()
        }
    })?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index =
        crate::owned_mp4_audio::mp4_audio_index(&reader, selected).map_err(|e| e.to_string())?;
    let track = &reader.tracks()[index];
    let info = info_for_file_tags(
        reader.tags(),
        &track.name,
        &track.language,
        Some(reader.encoder()),
        &BTreeMap::new(),
    )?;
    if let Some(max) = options.max_controlled_bytes {
        let index_bytes = reader
            .estimated_index_payload_bytes()
            .map_err(|e| e.to_string())?;
        if index_bytes
            .checked_add(info.len())
            .ok_or("WAVE metadata size overflow")?
            > max
        {
            return Err("controlled memory budget exceeded: WAVE metadata".into());
        }
    }
    Ok(info)
}

pub(crate) fn matroska_source_info(
    source: &std::path::Path,
    options: &fvid_control::CopyOptions,
) -> Result<Vec<u8>> {
    matroska_source_info_with_interval(source, None, options)
}

pub(crate) fn matroska_source_info_with_interval(
    source: &std::path::Path,
    interval: Option<(std::time::Duration, std::time::Duration)>,
    options: &fvid_control::CopyOptions,
) -> Result<Vec<u8>> {
    if options.cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
        return Err("media operation cancelled".into());
    }
    if options.max_packet_bytes == 0 {
        return Err("packet byte limit must be positive".into());
    }
    if options.max_packets == Some(0) {
        return Err("packet work limit requires at least one packet".into());
    }
    if options.max_controlled_bytes == Some(0) {
        return Err("controlled memory limit must be positive".into());
    }
    crate::owned_budget::check_rss_budget(options)?;
    let mut reader = crate::owned_matroska_audio::open_audio_reader(
        std::io::BufReader::new(std::fs::File::open(source).map_err(|e| e.to_string())?),
        options,
        options.max_packet_bytes,
    )
    .map_err(|e| e.to_string())?;
    let selected = match options.streams.as_slice() {
        [] => None,
        [index] => Some(*index),
        _ => return Err("select exactly one audio stream".into()),
    };
    let index = crate::owned_matroska_audio::matroska_audio_index(&reader, selected)
        .map_err(|e| e.to_string())?;
    if options.max_controlled_bytes.is_some() {
        crate::owned_matroska_audio::admit_audio_reader_with_interval(&mut reader, index, interval, options)
            .map_err(|e| e.to_string())?;
    } else {
        crate::owned_matroska_audio::validate_audio_packet_budget_with_interval(
            &mut reader, index, interval, options,
        )
        .map_err(|e| e.to_string())?;
    }
    let track = &reader.tracks[index];
    let mut extra = reader.metadata.clone();
    if let Some(global_track_tags) = reader.track_metadata.get(&0) {
        extra.extend(global_track_tags.clone());
    }
    if let Some(uid) = reader.track_uids.get(&track.number)
        && let Some(scoped) = reader.track_metadata.get(uid)
    {
        extra.extend(scoped.clone());
    }
    let language = reader
        .track_languages
        .get(&track.number)
        .map(String::as_str)
        .unwrap_or(&track.language);
    let info = info_for_file_tags(
        &reader.tags,
        &track.name,
        language,
        Some(&reader.writing_app),
        &extra,
    )?;
    if let Some(max) = options.max_controlled_bytes {
        let index_bytes = reader
            .estimated_index_payload_bytes()
            .map_err(|e| e.to_string())?;
        if index_bytes
            .checked_add(info.len())
            .ok_or("WAVE metadata size overflow")?
            > max
        {
            return Err("controlled memory budget exceeded: WAVE metadata".into());
        }
    }
    Ok(info)
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
            &[],
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
        let set_then_deleted = edit_info_chunks(
            &source,
            &["TITLE".into()],
            &[("title".into(), "transient".into())],
        )
        .unwrap();
        assert!(set_then_deleted.windows(opaque.len()).any(|bytes| bytes == opaque));
        assert!(!set_then_deleted.windows(6).any(|bytes| bytes == b"title\0"));
        assert!(!set_then_deleted.windows(10).any(|bytes| bytes == b"transient\0"));
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

use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const HEADER: &[u8]=b"[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
pub use media_info::NativeSubtitleStats as SubtitleStats;
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn timestamp(value: &str) -> Result<u64> {
    let trimmed = value.trim();
    let (clock, fraction) = trimmed
        .split_once(',')
        .or_else(|| trimmed.split_once('.'))
        .ok_or_else(|| invalid("invalid SRT timestamp"))?;
    let fields: Vec<_> = clock.split(':').collect();
    if fields.len() != 3
        || fields
            .iter()
            .any(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
        || fields[1].parse::<u32>().map_or(true, |v| v >= 60)
        || fields[2].parse::<u32>().map_or(true, |v| v >= 60)
        || fraction.len() != 3
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("invalid SRT timestamp"));
    }
    let hours = fields[0]
        .parse::<u64>()
        .map_err(|_| invalid("SRT timestamp overflow"))?;
    let minutes = fields[1]
        .parse::<u64>()
        .map_err(|_| invalid("invalid SRT timestamp"))?;
    let seconds = fields[2]
        .parse::<u64>()
        .map_err(|_| invalid("invalid SRT timestamp"))?;
    let millis = fraction
        .parse::<u64>()
        .map_err(|_| invalid("invalid SRT timestamp"))?;
    hours
        .checked_mul(3600)
        .and_then(|n| n.checked_add(minutes * 60 + seconds))
        .and_then(|n| n.checked_mul(1000))
        .and_then(|n| n.checked_add(millis))
        .and_then(|n| n.checked_mul(1_000_000))
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or_else(|| invalid("SRT timestamp overflow"))
}

#[derive(Clone, Default)]
struct Font {
    color: Option<String>,
    face: Option<String>,
    size: Option<u32>,
}
fn font_attributes(tag: &str, inherited: &Font) -> Option<Font> {
    let mut font = inherited.clone();
    let mut rest = tag.get(5..tag.len() - 1)?.trim();
    if rest.is_empty() {
        return None;
    }
    let mut seen = Vec::new();
    while !rest.is_empty() {
        let equal = rest.find('=')?;
        let key = rest[..equal].trim().to_ascii_lowercase();
        if seen.contains(&key) {
            return None;
        }
        seen.push(key.clone());
        rest = rest[equal + 1..].trim_start();
        let value;
        if rest.starts_with(char::from(39)) || rest.starts_with('"') {
            let quote = rest.chars().next()?;
            rest = &rest[1..];
            let end = rest.find(quote)?;
            value = &rest[..end];
            rest = &rest[end + 1..];
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            value = &rest[..end];
            rest = &rest[end..];
        }
        match key.as_str() {
            "color" => {
                let named = match value.to_ascii_lowercase().as_str() {
                    "black" => "000000",
                    "silver" => "c0c0c0",
                    "gray" => "808080",
                    "white" => "ffffff",
                    "maroon" => "800000",
                    "red" => "ff0000",
                    "purple" => "800080",
                    "fuchsia" => "ff00ff",
                    "green" => "008000",
                    "lime" => "00ff00",
                    "olive" => "808000",
                    "yellow" => "ffff00",
                    "navy" => "000080",
                    "blue" => "0000ff",
                    "teal" => "008080",
                    "aqua" => "00ffff",
                    _ => "",
                };
                let c = value.strip_prefix('#').unwrap_or(named);
                if c.len() != 6 || !c.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                font.color = Some(format!("&H{}{}{}&", &c[4..6], &c[2..4], &c[..2]));
            }
            "size" => {
                let size = value.parse::<u32>().ok()?;
                if !(1..=4096).contains(&size) {
                    return None;
                }
                font.size = Some(size);
            }
            "face" => {
                if value.is_empty()
                    || value.contains(['{', '}', '\\', '&'])
                    || value.chars().any(char::is_control)
                {
                    return None;
                }
                font.face = Some(value.into());
            }
            _ => return None,
        }
        rest = rest.trim_start();
    }
    Some(font)
}
fn font_changes(old: &Font, new: &Font, output: &mut String) {
    if old.color != new.color {
        output.push_str(&format!("{{\\c{}}}", new.color.as_deref().unwrap_or("")));
    }
    if old.face != new.face {
        output.push_str(&format!("{{\\fn{}}}", new.face.as_deref().unwrap_or("")));
    }
    if old.size != new.size {
        output.push_str(&format!(
            "{{\\fs{}}}",
            new.size.map_or(String::new(), |n| n.to_string())
        ));
    }
}
fn ass_text(text: &str) -> Option<String> {
    if text.contains(['{', '}', '\\', '\0']) {
        return None;
    }
    let mut output = String::new();
    let mut fonts: Vec<Font> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('&') {
            let Some(end) = rest.find(';').filter(|end| *end <= 16) else {
                output.push('&');
                rest = &rest[1..];
                continue;
            };
            let entity = &rest[1..end];
            let ch = if entity.eq_ignore_ascii_case("amp") {
                '&'
            } else if entity.eq_ignore_ascii_case("lt") {
                '<'
            } else if entity.eq_ignore_ascii_case("gt") {
                '>'
            } else if entity.eq_ignore_ascii_case("quot") {
                '"'
            } else if entity.eq_ignore_ascii_case("apos") {
                '\''
            } else if entity.eq_ignore_ascii_case("nbsp") {
                '\u{a0}'
            } else {
                let number = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    entity.strip_prefix('#')?.parse::<u32>().ok()?
                };
                char::from_u32(number)?
            };
            if matches!(ch, '{' | '}' | '\\') || ch.is_control() {
                return None;
            }
            output.push(ch);
            rest = &rest[end + 1..];
        } else if rest.starts_with('<') {
            let end = rest.find('>')?;
            let tag = rest[..=end].to_ascii_lowercase();
            if tag.starts_with("<font ") {
                let old = fonts.last().cloned().unwrap_or_default();
                let new = font_attributes(&rest[..=end], &old)?;
                font_changes(&old, &new, &mut output);
                fonts.push(new);
                rest = &rest[end + 1..];
                continue;
            }
            if tag == "</font>" {
                let old = fonts.pop()?;
                let new = fonts.last().cloned().unwrap_or_default();
                font_changes(&old, &new, &mut output);
                rest = &rest[end + 1..];
                continue;
            }
            output.push_str(match tag.as_str() {
                "<b>" => "{\\b1}",
                "</b>" => "{\\b0}",
                "<i>" => "{\\i1}",
                "</i>" => "{\\i0}",
                "<u>" => "{\\u1}",
                "</u>" => "{\\u0}",
                _ => return None,
            });
            rest = &rest[end + 1..];
        } else {
            let ch = rest.chars().next()?;
            if ch == '\n' {
                output.push_str("\\N");
            } else {
                output.push(ch);
            }
            rest = &rest[ch.len_utf8()..];
        }
    }
    Some(output)
}

pub fn try_convert(
    source: &Path,
    destination: &Path,
    streams: &[usize],
) -> Result<Option<SubtitleStats>> {
    try_convert_controlled(
        source,
        destination,
        streams,
        usize::MAX,
        None,
        None,
        None,
        None,
    )
}

pub fn try_convert_controlled(
    source: &Path,
    destination: &Path,
    streams: &[usize],
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    max_rss_bytes: Option<u64>,
    cancel: Option<&media_control::CancelFlag>,
    progress: Option<&media_control::ProgressHook>,
) -> Result<Option<SubtitleStats>> {
    let mut controls = media_control::CopyOptions::default();
    controls.streams = streams.to_vec();
    controls.max_packet_bytes = max_packet_bytes;
    controls.max_packets = max_packets;
    controls.max_rss_bytes = max_rss_bytes;
    controls.cancel = cancel.cloned();
    controls.progress = progress.cloned();
    try_convert_with_controls(
        source,
        destination,
        &media_info::SubtitleConvertOptions {
            streams: streams.to_vec(),
            codec: media_info::SubtitleCodec::Ass,
        },
        &controls,
    )
}

pub fn try_plan_with_controls(
    source: &Path,
    destination: Option<&Path>,
    options: &media_info::SubtitleConvertOptions,
    controls: &media_control::CopyOptions,
) -> Result<Option<media_info::MediaPlan>> {
    let streams = if !options.streams.is_empty() {
        options.streams.as_slice()
    } else {
        controls.streams.as_slice()
    };
    let max_packet_bytes = controls.max_packet_bytes;
    let max_packets = controls.max_packets;
    let max_rss_bytes = controls.max_rss_bytes;
    let cancel = controls.cancel.as_ref();
    let check_controls = || -> Result<()> {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        if max_packet_bytes == 0 {
            return Err(invalid("packet byte limit must be positive"));
        }
        if max_packets == Some(0) {
            return Err(invalid("packet work limit requires at least one packet"));
        }
        if controls.max_controlled_bytes == Some(0) {
            return Err(invalid("controlled memory limit must be positive"));
        }
        if let Some(limit) = max_rss_bytes {
            let mut opts = media_control::CopyOptions::default();
            opts.max_rss_bytes = Some(limit);
            owned_budget::check_rss_budget(&opts).map_err(|e| invalid(&e))?;
        }
        Ok(())
    };
    check_controls()?;
    let planned_dest = destination.unwrap_or_else(|| Path::new("planned-subtitle.mkv"));
    let (stream_index, source_codec, cues) = if !is_matroska_source(source)? {
        if source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("ass"))
        {
            let Some(stats) = try_convert_ass_inner(
                source,
                planned_dest,
                streams,
                max_packet_bytes,
                max_packets,
                Some(controls),
                &check_controls,
                true,
            )?
            else {
                return Ok(None);
            };
            (0usize, "ass", stats.cues)
        } else {
            let Some(stats) = try_convert_srt_inner(
                source,
                planned_dest,
                streams,
                max_packet_bytes,
                max_packets,
                Some(controls),
                &check_controls,
                true,
            )?
            else {
                return Ok(None);
            };
            (0usize, "subrip", stats.cues)
        }
    } else {
        let effective_packet_bytes =
            max_packet_bytes.min(controls.max_controlled_bytes.unwrap_or(usize::MAX));
        let memory_packets = controls
            .max_controlled_bytes
            .map(|limit| limit / std::mem::size_of::<webm::Packet>());
        let mut limits = webm::Limits::default();
        limits.packet_bytes = limits.packet_bytes.min(effective_packet_bytes);
        if let Some(maximum) = memory_packets {
            limits.packets = limits.packets.min(maximum);
        }
        let map_packet_limit = |error: Error| {
            if memory_packets.is_some_and(|max| max < webm::Limits::default().packets)
                && error
                    .to_string()
                    .contains("WebM packet count exceeds limit")
            {
                invalid("controlled memory budget exceeded: Matroska packet index limit")
            } else {
                error
            }
        };
        let mut reader = webm::WebmReader::open(
            std::io::BufReader::new(File::open(source)?),
            limits,
        )
        .map_err(|e| map_packet_limit(e.into()))?;
        reader.restrict_packet_bytes(effective_packet_bytes);
        reader.scan_all().map_err(|e| map_packet_limit(e.into()))?;
        if let Some(limit) = controls.max_controlled_bytes {
            let index_bytes = reader
                .estimated_index_payload_bytes()
                .map_err(|e| invalid(&e.to_string()))?
                .saturating_add(reader.packets.len().saturating_mul(64))
                .saturating_add(16384);
            if index_bytes > limit {
                return Err(invalid("controlled memory budget exceeded"));
            }
        }
        let idx = match streams {
            [] => reader
                .tracks
                .iter()
                .position(|track| track.kind == 17)
                .unwrap_or(0),
            [idx] => *idx,
            _ => 0,
        };
        let codec_name = match reader.tracks.get(idx).map(|t| t.codec.as_str()) {
            Some("S_TEXT/ASS") => "ass",
            _ => "subrip",
        };
        let Some(stats) = try_convert_matroska_inner(
            source,
            planned_dest,
            streams,
            max_packet_bytes,
            max_packets,
            Some(controls),
            &check_controls,
            true,
        )?
        else {
            return Ok(None);
        };
        (idx, codec_name, stats.cues)
    };
    if max_packets.is_some_and(|limit| cues > limit) {
        return Err(invalid("packet work limit exceeded"));
    }
    check_controls()?;
    let mut steps = vec![media_info::PlanStep {
        action: if source_codec == "ass" {
            "copy".into()
        } else {
            "decode".into()
        },
        detail: format!("validate and convert {cues} {source_codec} subtitle cues to Matroska S_TEXT/ASS"),
    }];
    let has_meta = !controls.metadata_set.is_empty()
        || !controls.metadata_delete.is_empty()
        || !controls.stream_metadata_set.is_empty()
        || !controls.stream_metadata_delete.is_empty();
    if has_meta {
        steps.push(media_info::PlanStep {
            action: "metadata".into(),
            detail: "apply Matroska container and stream 0 metadata mutations".into(),
        });
    }
    steps.push(media_info::PlanStep {
        action: "publish".into(),
        detail: "write Matroska S_TEXT/ASS output atomically via temporary file".into(),
    });
    let mut notes = vec!["subtitle conversion preserves source cues and Matroska metadata".into()];
    if max_packet_bytes != media_control::CopyOptions::default().max_packet_bytes
        || max_packets.is_some()
        || controls.max_controlled_bytes.is_some()
        || max_rss_bytes.is_some()
    {
        notes.push(format!(
            "encoded packet limit: {} bytes; packet work limit: {:?}; controlled memory limit: {:?}; RSS limit: {:?}",
            max_packet_bytes, max_packets, controls.max_controlled_bytes, max_rss_bytes
        ));
    }
    Ok(Some(media_info::MediaPlan {
        command: "convert-subtitles".into(),
        input: source.into(),
        inputs: vec![source.into()],
        streams: vec![media_info::PlanStream {
            index: stream_index,
            media_type: "subtitle".into(),
            codec: source_codec.into(),
            disposition: "convert".into(),
        }],
        steps,
        graph: None,
        notes,
    }))
}

pub fn try_convert_with_controls(
    source: &Path,
    destination: &Path,
    options: &media_info::SubtitleConvertOptions,
    controls: &media_control::CopyOptions,
) -> Result<Option<SubtitleStats>> {
    let streams = if !options.streams.is_empty() {
        options.streams.as_slice()
    } else {
        controls.streams.as_slice()
    };
    let max_packet_bytes = controls.max_packet_bytes;
    let max_packets = controls.max_packets;
    let max_rss_bytes = controls.max_rss_bytes;
    let cancel = controls.cancel.as_ref();
    let progress = controls.progress.as_ref();
    let check_controls = || -> Result<()> {
        if cancel.is_some_and(|c| c.is_cancelled()) {
            return Err(invalid("media operation cancelled"));
        }
        if max_packet_bytes == 0 {
            return Err(invalid("packet byte limit must be positive"));
        }
        if max_packets == Some(0) {
            return Err(invalid("packet work limit requires at least one packet"));
        }
        if controls.max_controlled_bytes == Some(0) {
            return Err(invalid("controlled memory limit must be positive"));
        }
        if let Some(limit) = max_rss_bytes {
            let mut opts = media_control::CopyOptions::default();
            opts.max_rss_bytes = Some(limit);
            owned_budget::check_rss_budget(&opts).map_err(|e| invalid(&e))?;
        }
        Ok(())
    };
    check_controls()?;
    let stats = if !is_matroska_source(source)? {
        if source
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("ass"))
        {
            try_convert_ass_inner(
                source,
                destination,
                streams,
                max_packet_bytes,
                max_packets,
                Some(controls),
                &check_controls,
                false,
            )?
        } else {
            try_convert_srt_inner(
                source,
                destination,
                streams,
                max_packet_bytes,
                max_packets,
                Some(controls),
                &check_controls,
                false,
            )?
        }
    } else {
        try_convert_matroska_inner(
            source,
            destination,
            streams,
            max_packet_bytes,
            max_packets,
            Some(controls),
            &check_controls,
            false,
        )?
    };
    let Some(stats) = stats else {
        return Ok(None);
    };
    if max_packets.is_some_and(|limit| stats.packets_out > limit) {
        let _ = std::fs::remove_file(destination);
        return Err(invalid("packet work limit exceeded"));
    }
    check_controls()?;
    if let Some(hook) = progress {
        hook.emit(media_control::ProgressEvent {
            packets: stats.packets_out,
            payload_bytes: stats.payload_bytes,
            done: true,
        });
    }
    Ok(Some(stats))
}

type SubtitleMatroskaMetadata = (
    String,
    String,
    FileMetadata,
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<usize, std::collections::BTreeMap<String, String>>,
);

fn apply_subtitle_matroska_metadata(
    default_name: &str,
    default_language: &str,
    base_file: Option<&FileMetadata>,
    base_text: Option<&std::collections::BTreeMap<String, String>>,
    base_track_tags: Option<&std::collections::BTreeMap<String, String>>,
    options: Option<&media_control::CopyOptions>,
) -> Result<SubtitleMatroskaMetadata> {
    let mut name = default_name.to_owned();
    let mut language = default_language.to_owned();
    let mut metadata = base_file.cloned().unwrap_or_default();
    let mut text = base_text.cloned().unwrap_or_default();
    let mut scoped = std::collections::BTreeMap::<
        usize,
        std::collections::BTreeMap<String, String>,
    >::new();
    if let Some(tags) = base_track_tags.filter(|t| !t.is_empty()) {
        scoped.insert(0, tags.clone());
    }
    if let Some(o) = options {
        if o.metadata_set.len() + o.metadata_delete.len() > 64 {
            return Err(invalid("at most 64 container metadata mutations"));
        }
        if o.stream_metadata_set.len() + o.stream_metadata_delete.len() > 64 {
            return Err(invalid("at most 64 stream metadata mutations"));
        }
        for (key, value) in &o.metadata_set {
            if key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
            {
                return Err(invalid("invalid Matroska container metadata"));
            }
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
            if !metadata.tags.set(key, value) && !value.is_empty() {
                text.insert(key.to_ascii_uppercase(), value.clone());
            }
        }
        for key in &o.metadata_delete {
            if key.is_empty() || key.len() > 128 || key.contains('\0') {
                return Err(invalid("invalid Matroska container metadata key"));
            }
            metadata.tags.set(key, "");
            text.retain(|k, _| !k.eq_ignore_ascii_case(key));
        }
        for (index, key, value) in &o.stream_metadata_set {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || value.len() > 1024
                || key.contains('\0')
                || value.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
                || (key.eq_ignore_ascii_case("language") && value.len() > 128)
            {
                return Err(invalid("invalid Matroska stream metadata"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clone_from(value);
            } else if key.eq_ignore_ascii_case("language") {
                language.clone_from(value);
            } else {
                let entries = scoped.entry(0).or_default();
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
                if !value.is_empty() {
                    entries.insert(key.to_ascii_uppercase(), value.clone());
                }
            }
        }
        for (index, key) in &o.stream_metadata_delete {
            if *index != 0
                || key.is_empty()
                || key.len() > 128
                || key.contains('\0')
                || matches!(
                    key.to_ascii_lowercase().as_str(),
                    "rotate" | "stereo_mode" | "alpha_mode"
                )
            {
                return Err(invalid("invalid Matroska stream metadata deletion"));
            }
            if key.eq_ignore_ascii_case("title") {
                name.clear();
            } else if key.eq_ignore_ascii_case("language") {
                language.clear();
            } else if let Some(entries) = scoped.get_mut(&0) {
                entries.retain(|k, _| !k.eq_ignore_ascii_case(key));
            }
        }
    }
    Ok((name, language, metadata, text, scoped))
}

fn estimate_subtitle_matroska_metadata_working_bytes(
    (name, language, metadata, text, scoped): &SubtitleMatroskaMetadata,
) -> usize {
    let mut raw = name.len().saturating_add(language.len());
    for (k, v) in metadata.tags.entries() {
        if !v.is_empty() {
            raw = raw
                .saturating_add(k.len())
                .saturating_add(v.len())
                .saturating_add(16);
        }
    }
    for (k, v) in text {
        raw = raw
            .saturating_add(k.len())
            .saturating_add(v.len())
            .saturating_add(16);
    }
    for tags in scoped.values() {
        for (k, v) in tags {
            raw = raw
                .saturating_add(k.len())
                .saturating_add(v.len())
                .saturating_add(16);
        }
    }
    raw.saturating_mul(8)
}

fn try_convert_matroska_inner(
    source: &Path,
    destination: &Path,
    streams: &[usize],
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    controls: Option<&media_control::CopyOptions>,
    check_controls: &impl Fn() -> Result<()>,
    dry_run: bool,
) -> Result<Option<SubtitleStats>> {
    let max_controlled_bytes = controls.and_then(|c| c.max_controlled_bytes);
    let effective_packet_bytes = max_packet_bytes.min(max_controlled_bytes.unwrap_or(usize::MAX));
    let memory_packets = max_controlled_bytes
        .map(|limit| limit / std::mem::size_of::<webm::Packet>());
    let mut limits = webm::Limits::default();
    limits.packet_bytes = limits.packet_bytes.min(effective_packet_bytes);
    if let Some(maximum) = memory_packets {
        limits.packets = limits.packets.min(maximum);
    }
    let map_packet_limit = |error: Error| {
        if memory_packets.is_some_and(|max| max < webm::Limits::default().packets)
            && error
                .to_string()
                .contains("WebM packet count exceeds limit")
        {
            invalid("controlled memory budget exceeded: Matroska packet index limit")
        } else {
            error
        }
    };
    let mut reader = webm::WebmReader::open(
        std::io::BufReader::new(File::open(source)?),
        limits,
    )
    .map_err(|e| map_packet_limit(e.into()))?;
    reader.restrict_packet_bytes(effective_packet_bytes);
    reader.scan_all().map_err(|e| map_packet_limit(e.into()))?;
    if let Some(limit) = max_controlled_bytes {
        let index_bytes = reader
            .estimated_index_payload_bytes()
            .map_err(|e| invalid(&e.to_string()))?
            .saturating_add(reader.packets.len().saturating_mul(64))
            .saturating_add(16384);
        if index_bytes > limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    if !reader.chapters.is_empty() {
        return Err(invalid("convert-subtitles with chapters is not qualified"));
    }
    let index = match streams {
        [] => reader
            .tracks
            .iter()
            .position(|track| track.kind == 17)
            .ok_or_else(|| invalid("input has no subtitle stream"))?,
        [index] => *index,
        _ => {
            return Err(invalid(
                "convert-subtitles requires exactly one selected subtitle stream",
            ));
        }
    };
    let track = reader
        .tracks
        .get(index)
        .ok_or_else(|| invalid("subtitle stream index is out of range"))?;
    if track.kind != 17 {
        return Err(invalid("selected stream is not a subtitle"));
    }
    let base_file = FileMetadata {
        tags: reader.tags.clone(),
        chapters: Vec::new(),
    };
    let mut base_text = reader.metadata.clone();
    if !reader.writing_app.is_empty()
        && reader.writing_app != "FVid"
        && !base_text.keys().any(|k| k.eq_ignore_ascii_case("encoder"))
    {
        base_text.insert("ENCODER".into(), reader.writing_app.clone());
    }
    let mut base_track_tags = reader.track_metadata.get(&0).cloned().unwrap_or_default();
    if let Some(uid) = reader.track_uids.get(&track.number)
        && let Some(scoped) = reader.track_metadata.get(uid)
    {
        base_track_tags.extend(scoped.clone());
    }
    let source_language = reader
        .track_languages
        .get(&track.number)
        .unwrap_or(&track.language)
        .clone();
    if track.codec == "S_TEXT/ASS" {
        if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
            return Err(invalid("convert-subtitles requires Matroska (.mkv) output"));
        }
        let (number, name, configuration) = (
            track.number,
            track.name.clone(),
            track.codec_private.clone(),
        );
        let meta = apply_subtitle_matroska_metadata(
            &name,
            &source_language,
            Some(&base_file),
            Some(&base_text),
            Some(&base_track_tags),
            controls,
        )?;
        let meta_working_bytes = estimate_subtitle_matroska_metadata_working_bytes(&meta);
        if let Some(limit) = max_controlled_bytes
            && meta_working_bytes.saturating_add(16384) > limit
        {
            return Err(invalid("controlled memory budget exceeded"));
        }
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for i in 0..reader.packets.len() {
            check_controls()?;
            let packet = &reader.packets[i];
            if packet.track != number {
                continue;
            }
            if max_packets.is_some_and(|limit| events.len() as u64 >= limit) {
                return Err(invalid("packet work limit exceeded"));
            }
            let start = u64::try_from(packet.pts_ns)
                .map_err(|_| invalid("subtitle requires nonnegative timestamps"))?;
            let Some(duration) = packet.duration_ns.filter(|d| *d > 0) else {
                return Ok(None);
            };
            let end = start
                .checked_add(duration)
                .filter(|end| *end <= i64::MAX as u64)
                .ok_or_else(|| invalid("subtitle timestamp overflow"))?;
            let packet_size = reader.packet_payload_size(i)?;
            if packet_size > max_packet_bytes {
                return Err(invalid("subtitle packet exceeds packet byte limit"));
            }
            bytes = bytes
                .checked_add(packet_size)
                .filter(|n| *n <= 64 << 20)
                .ok_or_else(|| invalid("subtitle metadata exceeds 64 MiB"))?;
            if let Some(limit) = max_controlled_bytes {
                let working = bytes
                    .saturating_add(events.len().saturating_add(1).saturating_mul(48))
                    .saturating_add(configuration.len())
                    .saturating_add(meta_working_bytes)
                    .saturating_add(16384);
                if working > limit {
                    return Err(invalid("controlled memory budget exceeded"));
                }
            }
            let payload = reader.read_packet(i)?;
            let text =
                std::str::from_utf8(&payload).map_err(|_| invalid("Matroska ASS is not UTF-8"))?;
            if text.is_empty() || text.contains('\0') || text.splitn(9, ',').count() != 9 {
                return Err(invalid("invalid Matroska ASS event"));
            }
            events.push((start, end, payload));
        }
        if events.is_empty() {
            return Err(invalid("selected subtitle stream has no cues"));
        }
        return publish_events_with_metadata(
            events,
            destination,
            &configuration,
            meta,
            max_packet_bytes,
            dry_run,
        );
    }
    if track.codec != "S_TEXT/UTF8" {
        return Ok(None);
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("convert-subtitles requires Matroska (.mkv) output"));
    }
    let (number, name) = (track.number, track.name.clone());
    let meta = apply_subtitle_matroska_metadata(
        &name,
        &source_language,
        Some(&base_file),
        Some(&base_text),
        Some(&base_track_tags),
        controls,
    )?;
    let meta_working_bytes = estimate_subtitle_matroska_metadata_working_bytes(&meta);
    if let Some(limit) = max_controlled_bytes
        && meta_working_bytes.saturating_add(16384) > limit
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let mut cues = Vec::new();
    let mut bytes = 0usize;
    for i in 0..reader.packets.len() {
        check_controls()?;
        let packet = &reader.packets[i];
        if packet.track != number {
            continue;
        }
        if max_packets.is_some_and(|limit| cues.len() as u64 >= limit) {
            return Err(invalid("packet work limit exceeded"));
        }
        let start = u64::try_from(packet.pts_ns)
            .map_err(|_| invalid("subtitle requires nonnegative timestamps"))?;
        let Some(duration) = packet.duration_ns.filter(|d| *d > 0) else {
            return Ok(None);
        };
        let end = start
            .checked_add(duration)
            .filter(|end| *end <= i64::MAX as u64)
            .ok_or_else(|| invalid("subtitle timestamp overflow"))?;
        let packet_size = reader.packet_payload_size(i)?;
        if packet_size > max_packet_bytes {
            return Err(invalid("subtitle packet exceeds packet byte limit"));
        }
        bytes = bytes
            .checked_add(packet_size)
            .ok_or_else(|| invalid("subtitle size overflow"))?;
        if bytes > 64 << 20 {
            return Err(invalid("subtitle metadata exceeds 64 MiB"));
        }
        if let Some(limit) = max_controlled_bytes {
            let working = bytes
                .saturating_add(cues.len().saturating_add(1).saturating_mul(48))
                .saturating_add(meta_working_bytes)
                .saturating_add(16384);
            if working > limit {
                return Err(invalid("controlled memory budget exceeded"));
            }
        }
        let data = reader.read_packet(i)?;
        let text =
            std::str::from_utf8(&data).map_err(|_| invalid("Matroska SubRip is not UTF-8"))?;
        let Some(text) = ass_text(&text.replace("\r\n", "\n")) else {
            return Ok(None);
        };
        if text.is_empty() {
            return Err(invalid("empty subtitle cue"));
        }
        if format!("{},0,Default,,0,0,0,,{text}", cues.len()).len() > max_packet_bytes {
            return Err(invalid("subtitle packet exceeds packet byte limit"));
        }
        cues.push((start, end, text));
    }
    if cues.is_empty() {
        return Err(invalid("selected subtitle stream has no cues"));
    }
    publish_with_metadata(cues, destination, meta, max_packet_bytes, dry_run)
}
/// `None` preserves the adapter for unsupported text encodings or markup.
/// Recognized invalid timing never publishes a partial output.
/// Preserve standalone UTF-8 ASS scripts and dialogue fields in Matroska.
/// Complete event formats may reorder fields; Text must be last.
pub fn try_convert_ass(
    source: &Path,
    destination: &Path,
    streams: &[usize],
) -> Result<Option<SubtitleStats>> {
    try_convert_ass_inner(
        source,
        destination,
        streams,
        usize::MAX,
        None,
        None,
        &|| Ok(()),
        false,
    )
}

fn try_convert_ass_inner(
    source: &Path,
    destination: &Path,
    streams: &[usize],
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    controls: Option<&media_control::CopyOptions>,
    check_controls: &impl Fn() -> Result<()>,
    dry_run: bool,
) -> Result<Option<SubtitleStats>> {
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("ASS conversion requires Matroska (.mkv) output"));
    }
    if !streams.is_empty() && streams != [0] {
        return Err(invalid("ASS has only subtitle stream 0"));
    }
    let meta = apply_subtitle_matroska_metadata("", "", None, None, None, controls)?;
    let meta_working_bytes = estimate_subtitle_matroska_metadata_working_bytes(&meta);
    let max_controlled_bytes = controls.and_then(|c| c.max_controlled_bytes);
    if let Some(limit) = max_controlled_bytes
        && meta_working_bytes.saturating_add(16384) > limit
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let file_len = std::fs::metadata(source)?.len();
    if file_len > 64 << 20 {
        return Err(invalid("ASS metadata exceeds 64 MiB"));
    }
    if let Some(limit) = max_controlled_bytes {
        let working = usize::try_from(file_len)
            .unwrap_or(usize::MAX)
            .saturating_mul(3)
            .saturating_add(meta_working_bytes)
            .saturating_add(16384);
        if working > limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let mut bytes = Vec::new();
    File::open(source)?
        .take((64 << 20) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 << 20 {
        return Err(invalid("ASS metadata exceeds 64 MiB"));
    }
    if let Some(limit) = max_controlled_bytes {
        let working = bytes
            .len()
            .saturating_mul(3)
            .saturating_add(meta_working_bytes)
            .saturating_add(16384);
        if working > limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if normalized.contains('\0') {
        return Err(invalid("NUL in ASS script"));
    }
    let mut configuration = String::new();
    let mut events = Vec::new();
    let mut in_events = false;
    let mut format: Option<[usize; 10]> = None;
    let canonical = "layer,start,end,style,name,marginl,marginr,marginv,effect,text";
    let ass_time = |value: &str| -> Result<u64> {
        let (clock, fraction) = value
            .trim()
            .rsplit_once('.')
            .ok_or_else(|| invalid("invalid ASS timestamp"))?;
        if !(1..=3).contains(&fraction.len()) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid("ASS timestamp requires 1..=3 fractional digits"));
        }
        let millis = match fraction.len() {
            1 => format!("{fraction}00"),
            2 => format!("{fraction}0"),
            _ => fraction.to_owned(),
        };
        timestamp(&format!("{clock},{millis}"))
    };
    for line in normalized.lines() {
        check_controls()?;
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            format = None;
        }
        if in_events {
            if let Some((key, value)) = trimmed.split_once(':') {
                if key.trim().eq_ignore_ascii_case("Format") {
                    let declared: Vec<_> = value.split(',').map(|s| s.trim().to_ascii_lowercase()).collect();
                    let names: Vec<_> = canonical.split(',').collect();
                    if declared.len()!=10 || declared.last().map(String::as_str)!=Some("text") { return Ok(None); }
                    let mut order = [0;10];
                    for (index,name) in names.iter().enumerate() {
                        if declared.iter().filter(|s|s.as_str()==*name).count()!=1 { return Ok(None); }
                        order[index] = declared.iter().position(|s|s==name).unwrap();
                    }
                    format = Some(order);
                    if order.iter().enumerate().any(|(i,at)|i!=*at) {
                        configuration.push_str("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
                        continue;
                    }
                } else if key.trim().eq_ignore_ascii_case("Dialogue") {
                    if max_packets.is_some_and(|limit| events.len() as u64 >= limit) {
                        return Err(invalid("packet work limit exceeded"));
                    }
                    let Some(order) = format else { return Ok(None); };
                    let fields: Vec<_> = value.trim_start().splitn(10, ',').collect();
                    if fields.len() != 10 {
                        return Err(invalid("invalid ASS dialogue fields"));
                    }
                    let layer = fields[order[0]]
                        .trim()
                        .parse::<i32>()
                        .map_err(|_| invalid("invalid ASS layer"))?;
                    let start = ass_time(fields[order[1]])?;
                    let end = ass_time(fields[order[2]])?;
                    if start >= end {
                        return Err(invalid("ASS dialogue requires start before end"));
                    }
                    let payload =
                        format!("{},{layer},{}", events.len(), order[3..].iter().map(|at|fields[*at]).collect::<Vec<_>>().join(",")).into_bytes();
                    if payload.len() > max_packet_bytes {
                        return Err(invalid("subtitle packet exceeds packet byte limit"));
                    }
                    events.push((start, end, payload));
                    continue;
                }
            }
        }
        configuration.push_str(line);
        configuration.push('\n');
    }
    if events.is_empty() {
        return Err(invalid("ASS script has no dialogue cues"));
    }
    events.sort_by_key(|event| event.0);
    publish_events_with_metadata(
        events,
        destination,
        configuration.as_bytes(),
        meta,
        max_packet_bytes,
        dry_run,
    )
}

pub fn try_convert_srt(
    source: &Path,
    destination: &Path,
    streams: &[usize],
) -> Result<Option<SubtitleStats>> {
    try_convert_srt_inner(
        source,
        destination,
        streams,
        usize::MAX,
        None,
        None,
        &|| Ok(()),
        false,
    )
}

fn try_convert_srt_inner(
    source: &Path,
    destination: &Path,
    streams: &[usize],
    max_packet_bytes: usize,
    max_packets: Option<u64>,
    controls: Option<&media_control::CopyOptions>,
    check_controls: &impl Fn() -> Result<()>,
    dry_run: bool,
) -> Result<Option<SubtitleStats>> {
    if source
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("srt"))
        != Some(true)
    {
        return Ok(None);
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("convert-subtitles requires Matroska (.mkv) output"));
    }
    if !streams.is_empty() && streams != [0] {
        return Err(invalid("SRT has only subtitle stream 0"));
    }
    let meta = apply_subtitle_matroska_metadata("", "und", None, None, None, controls)?;
    let meta_working_bytes = estimate_subtitle_matroska_metadata_working_bytes(&meta);
    let max_controlled_bytes = controls.and_then(|c| c.max_controlled_bytes);
    if let Some(limit) = max_controlled_bytes
        && meta_working_bytes.saturating_add(16384) > limit
    {
        return Err(invalid("controlled memory budget exceeded"));
    }
    let file_len = std::fs::metadata(source)?.len();
    if file_len > 64 << 20 {
        return Err(invalid("SRT metadata exceeds 64 MiB"));
    }
    if let Some(limit) = max_controlled_bytes {
        let working = usize::try_from(file_len)
            .unwrap_or(usize::MAX)
            .saturating_mul(3)
            .saturating_add(meta_working_bytes)
            .saturating_add(16384);
        if working > limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let mut bytes = Vec::new();
    File::open(source)?
        .take((64 << 20) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 << 20 {
        return Err(invalid("SRT metadata exceeds 64 MiB"));
    }
    if let Some(limit) = max_controlled_bytes {
        let working = bytes
            .len()
            .saturating_mul(3)
            .saturating_add(meta_working_bytes)
            .saturating_add(16384);
        if working > limit {
            return Err(invalid("controlled memory budget exceeded"));
        }
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let normalized = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let text = normalized
        .lines()
        .map(|line| if line.trim().is_empty() { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n").filter(|b| !b.trim().is_empty()) {
        check_controls()?;
        if max_packets.is_some_and(|limit| cues.len() as u64 >= limit) {
            return Err(invalid("packet work limit exceeded"));
        }
        let mut lines = block.trim_matches('\n').lines();
        let first = lines.next().ok_or_else(|| invalid("empty SRT cue"))?;
        let timing = if first.contains("-->") {
            first
        } else {
            if first.trim().parse::<u64>().is_err() {
                return Err(invalid("invalid SRT cue counter"));
            }
            lines
                .next()
                .ok_or_else(|| invalid("missing SRT cue timing"))?
        };
        let (start, end) = timing
            .split_once("-->")
            .ok_or_else(|| invalid("invalid SRT cue timing"))?;
        let end = end.split_whitespace().next().unwrap_or("");
        let (start, end) = (timestamp(start)?, timestamp(end)?);
        if end <= start {
            return Err(invalid("SRT cue must end after its start"));
        }
        let body = lines.collect::<Vec<_>>().join("\n");
        if body.is_empty() {
            return Err(invalid("empty SRT cue text"));
        }
        let Some(body) = ass_text(&body) else {
            return Ok(None);
        };
        if format!("{},0,Default,,0,0,0,,{body}", cues.len()).len() > max_packet_bytes {
            return Err(invalid("subtitle packet exceeds packet byte limit"));
        }
        cues.push((start, end, body));
    }
    if cues.is_empty() {
        return Err(invalid("SRT has no cues"));
    }
    publish_with_metadata(cues, destination, meta, max_packet_bytes, dry_run)
}

fn publish_with_metadata(
    cues: Vec<(u64, u64, String)>,
    destination: &Path,
    meta: SubtitleMatroskaMetadata,
    max_packet_bytes: usize,
    dry_run: bool,
) -> Result<Option<SubtitleStats>> {
    let events = cues
        .into_iter()
        .enumerate()
        .map(|(index, (start, end, text))| {
            (
                start,
                end,
                format!("{index},0,Default,,0,0,0,,{text}").into_bytes(),
            )
        })
        .collect();
    publish_events_with_metadata(events, destination, HEADER, meta, max_packet_bytes, dry_run)
}

fn publish_events_with_metadata(
    events: Vec<(u64, u64, Vec<u8>)>,
    destination: &Path,
    configuration: &[u8],
    (name, language, metadata, text, scoped): SubtitleMatroskaMetadata,
    max_packet_bytes: usize,
    dry_run: bool,
) -> Result<Option<SubtitleStats>> {
    let mut total_payload_bytes = 0u64;
    for (_, _, payload) in &events {
        if payload.len() > max_packet_bytes {
            return Err(invalid("subtitle packet exceeds packet byte limit"));
        }
        total_payload_bytes = total_payload_bytes.saturating_add(payload.len() as u64);
    }
    if dry_run {
        return Ok(Some(SubtitleStats {
            backend: "fvid",
            encoder: "ass".into(),
            cues: events.len() as u64,
            packets_in: events.len() as u64,
            packets_out: events.len() as u64,
            payload_bytes: total_payload_bytes,
        }));
    }
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, mut file) = loop {
        let path = directory.join(format!(
            ".fvid-subtitle-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (Temporary(path), file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let mut writer = PacketWriter::new_with_text_metadata(
        &mut file,
        &[TrackSpec {
            encoding: Encoding::Ass { configuration },
            name: &name,
            language: &language,
        }],
        &[],
        &metadata,
        &text,
        &scoped,
    )?;
    for (start, end, payload) in &events {
        writer.write_packet(0, *start, end - start, true, payload)?;
    }
    let event = writer.finish()?;
    file.flush()?;
    file.sync_all()?;
    std::fs::hard_link(&temporary.0, destination)?;
    Ok(Some(SubtitleStats {
        backend: "fvid",
        encoder: "ass".into(),
        cues: events.len() as u64,
        packets_in: events.len() as u64,
        packets_out: event.packets,
        payload_bytes: event.payload_bytes,
    }))
}

/// Convert supported subtitle sources with backend-independent operation options.
/// Unsupported profiles return `None` before creating the destination.
pub fn try_convert_with_options(
    source: &Path,
    destination: &Path,
    options: &media_info::SubtitleConvertOptions,
) -> Result<Option<SubtitleStats>> {
    match options.codec {
        media_info::SubtitleCodec::Ass => {
            try_convert_with_controls(source, destination, options, &Default::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ass_text;
    #[test]
    fn entities_are_literal_text_not_ass_or_html_commands() {
        assert_eq!(
            ass_text("A&amp;B &lt;i&gt; &#x41;&#66; &quot;x&quot; &apos;y&apos; &nbsp;"),
            Some("A&B <i> AB \"x\" 'y' \u{a0}".into())
        );
        assert_eq!(ass_text("H&M"), Some("H&M".into()));
        for text in ["&#123;", "&#92;", "&#0;", "&#xD800;", "&unknown;"] {
            assert!(ass_text(text).is_none(), "{text}");
        }
    }
}

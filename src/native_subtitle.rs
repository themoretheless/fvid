//! Owned SubRip text conversion and atomic ASS Matroska publication.
use crate::container::matroska_write::{Encoding, PacketWriter, TrackSpec};
use crate::{Result, invalid};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const HEADER: &[u8]=b"[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,16,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
pub use crate::media_info::NativeSubtitleStats as SubtitleStats;
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn timestamp(value: &str) -> Result<u64> {
    let (clock, fraction) = value
        .trim()
        .split_once(',')
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
fn ass_text(text: &str) -> Option<String> {
    if text.contains(['{', '}', '\\', '&', '\0']) {
        return None;
    }
    let mut output = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let end = rest.find('>')?;
            let tag = rest[..=end].to_ascii_lowercase();
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
/// `None` preserves the adapter for unsupported text encodings or markup.
/// Recognized invalid timing never publishes a partial output.
pub fn try_convert_srt(
    source: &Path,
    destination: &Path,
    streams: &[usize],
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
    let mut bytes = Vec::new();
    File::open(source)?
        .take((64 << 20) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 << 20 {
        return Err(invalid("SRT metadata exceeds 64 MiB"));
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(None);
    };
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n").filter(|b| !b.trim().is_empty()) {
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
        cues.push((start, end, body));
    }
    if cues.is_empty() {
        return Err(invalid("SRT has no cues"));
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
    let mut writer = PacketWriter::new(
        &mut file,
        &[TrackSpec {
            encoding: Encoding::Ass {
                configuration: HEADER,
            },
            name: "",
            language: "und",
        }],
    )?;
    for (index, (start, end, text)) in cues.iter().enumerate() {
        let payload = format!("{index},0,Default,,0,0,0,,{text}");
        writer.write_packet(0, *start, end - start, true, payload.as_bytes())?;
    }
    let event = writer.finish()?;
    file.flush()?;
    file.sync_all()?;
    std::fs::hard_link(&temporary.0, destination)?;
    Ok(Some(SubtitleStats {
        backend: "fvid",
        encoder: "ass".into(),
        cues: cues.len() as u64,
        packets_in: cues.len() as u64,
        packets_out: event.packets,
        payload_bytes: event.payload_bytes,
    }))
}

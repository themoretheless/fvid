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
    if text.contains(['{', '}', '\\', '\0']) {
        return None;
    }
    let mut output = String::new();
    let mut colors: Vec<String> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.starts_with('&') {
            let Some(end) = rest.find(';').filter(|end| *end <= 16) else {
                output.push('&');
                rest = &rest[1..];
                continue;
            };
            let entity = &rest[1..end];
            let ch = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                _ => {
                    let number = if let Some(hex) = entity
                        .strip_prefix("#x")
                        .or_else(|| entity.strip_prefix("#X"))
                    {
                        u32::from_str_radix(hex, 16).ok()?
                    } else {
                        entity.strip_prefix('#')?.parse::<u32>().ok()?
                    };
                    char::from_u32(number)?
                }
            };
            if matches!(ch, '{' | '}' | '\\') || ch.is_control() {
                return None;
            }
            output.push(ch);
            rest = &rest[end + 1..];
        } else if rest.starts_with('<') {
            let end = rest.find('>')?;
            let tag = rest[..=end].to_ascii_lowercase();
            if let Some(color) = tag
                .strip_prefix("<font color=\"#")
                .and_then(|s| s.strip_suffix("\">"))
            {
                if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let override_text =
                    format!("{{\\c&H{}{}{}&}}", &color[4..6], &color[2..4], &color[..2]);
                output.push_str(&override_text);
                colors.push(override_text);
                rest = &rest[end + 1..];
                continue;
            }
            if tag == "</font>" {
                colors.pop()?;
                output.push_str(colors.last().map_or("{\\c}", String::as_str));
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
    if !crate::native_export::is_matroska_source(source)? {
        return try_convert_srt(source, destination, streams);
    }
    let mut reader = crate::container::webm::WebmReader::open(
        std::io::BufReader::new(File::open(source)?),
        Default::default(),
    )?;
    reader.scan_all()?;
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
    if track.codec != "S_TEXT/UTF8" {
        return Ok(None);
    }
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("convert-subtitles requires Matroska (.mkv) output"));
    }
    let (number, name, language) = (track.number, track.name.clone(), track.language.clone());
    let mut cues = Vec::new();
    let mut bytes = 0usize;
    for i in 0..reader.packets.len() {
        let packet = &reader.packets[i];
        if packet.track != number {
            continue;
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
        bytes = bytes
            .checked_add(packet.size)
            .ok_or_else(|| invalid("subtitle size overflow"))?;
        if bytes > 64 << 20 {
            return Err(invalid("subtitle metadata exceeds 64 MiB"));
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
        cues.push((start, end, text));
    }
    if cues.is_empty() {
        return Err(invalid("selected subtitle stream has no cues"));
    }
    publish(cues, destination, &name, &language)
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
    publish(cues, destination, "", "und")
}

fn publish(
    cues: Vec<(u64, u64, String)>,
    destination: &Path,
    name: &str,
    language: &str,
) -> Result<Option<SubtitleStats>> {
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
            name,
            language,
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

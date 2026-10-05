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
    if !is_matroska_source(source)? {
        if source.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("ass")) {
            return try_convert_ass(source,destination,streams);
        }
        return try_convert_srt(source, destination, streams);
    }
    let mut reader = webm::WebmReader::open(
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
    if track.codec == "S_TEXT/ASS" {
        if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
            return Err(invalid("convert-subtitles requires Matroska (.mkv) output"));
        }
        let (number, name, language, configuration) = (
            track.number,
            track.name.clone(),
            track.language.clone(),
            track.codec_private.clone(),
        );
        let mut events = Vec::new();
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
                .filter(|n| *n <= 64 << 20)
                .ok_or_else(|| invalid("subtitle metadata exceeds 64 MiB"))?;
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
        return publish_events(events, destination, &name, &language, &configuration);
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
/// Preserve standalone UTF-8 ASS scripts and dialogue fields in Matroska.
/// Complete event formats may reorder fields; Text must be last.
pub fn try_convert_ass(
    source: &Path,
    destination: &Path,
    streams: &[usize],
) -> Result<Option<SubtitleStats>> {
    if destination.extension().and_then(|s| s.to_str()) != Some("mkv") {
        return Err(invalid("ASS conversion requires Matroska (.mkv) output"));
    }
    if !streams.is_empty() && streams != [0] {
        return Err(invalid("ASS has only subtitle stream 0"));
    }
    let mut bytes = Vec::new();
    File::open(source)?
        .take((64 << 20) + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 << 20 {
        return Err(invalid("ASS metadata exceeds 64 MiB"));
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
        if fraction.len() != 2 {
            return Err(invalid("ASS timestamp requires centiseconds"));
        }
        timestamp(&format!("{clock},{fraction}0"))
    };
    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            format = None;
        }
        if in_events {
            if let Some(value) = trimmed.strip_prefix("Format:") {
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
            }
            if let Some(value) = line.trim_start().strip_prefix("Dialogue:") {
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
                events.push((start, end, payload));
                continue;
            }
        }
        configuration.push_str(line);
        configuration.push('\n');
    }
    if events.is_empty() {
        return Err(invalid("ASS script has no dialogue cues"));
    }
    events.sort_by_key(|event| event.0);
    publish_events(events, destination, "", "", configuration.as_bytes())
}

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
    publish_events(events, destination, name, language, HEADER)
}

fn publish_events(
    events: Vec<(u64, u64, Vec<u8>)>,
    destination: &Path,
    name: &str,
    language: &str,
    configuration: &[u8],
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
            encoding: Encoding::Ass { configuration },
            name,
            language,
        }],
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
        media_info::SubtitleCodec::Ass => try_convert(source, destination, &options.streams),
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

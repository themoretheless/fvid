//! Plain-text and PAL8 bitmap subtitle cues, SRT/WebVTT/ASS parsing, and compositing.
type Result<T> = std::result::Result<T, String>;


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleCue {
    pub start_us: i64,
    pub end_us: i64,
    pub text: String,
}

/// Decoded PAL8 bitmap subtitle plane for overlay (VLC PGS/VobSub path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapSubtitle {
    pub start_us: i64,
    pub end_us: i64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Packed `0xAARRGGBB` top-left first.
    pub pixels: Vec<u32>,
}

/// Map one PAL8 index through a 256-entry RGBA palette (`[R,G,B,A] * 256`).
pub fn palette_rgba_pixel(index: u8, palette: &[u8]) -> u32 {
    let i = usize::from(index) * 4;
    if palette.len() < i + 4 {
        return 0;
    }
    let r = u32::from(palette[i]);
    let g = u32::from(palette[i + 1]);
    let b = u32::from(palette[i + 2]);
    let a = u32::from(palette[i + 3]);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Expand a tightly packed or strided PAL8 bitmap into `0xAARRGGBB` pixels.
pub fn pal8_to_rgba(
    width: u32,
    height: u32,
    bitmap: &[u8],
    stride: usize,
    palette: &[u8],
) -> Result<Vec<u32>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return Err("bitmap subtitle has empty size".into());
    }
    if stride < w {
        return Err("bitmap subtitle stride is too small".into());
    }
    if bitmap.len() < stride.saturating_mul(h.saturating_sub(1)).saturating_add(w) {
        return Err("bitmap subtitle buffer is incomplete".into());
    }
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            pixels.push(palette_rgba_pixel(bitmap[row + x], palette));
        }
    }
    Ok(pixels)
}

/// Alpha-blend `src` (`0xAARRGGBB`) onto opaque `dst` (`0x00RRGGBB`).
pub fn blend_rgba_over_rgb(dst: u32, src: u32) -> u32 {
    let a = ((src >> 24) & 0xff) as u32;
    if a == 0 {
        return dst;
    }
    if a == 255 {
        return src & 0x00ff_ffff;
    }
    let inv = 255 - a;
    let dr = (dst >> 16) & 0xff;
    let dg = (dst >> 8) & 0xff;
    let db = dst & 0xff;
    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;
    let r = (sr * a + dr * inv) / 255;
    let g = (sg * a + dg * inv) / 255;
    let b = (sb * a + db * inv) / 255;
    (r << 16) | (g << 8) | b
}

/// Composite one bitmap plane onto a packed RGB frame. Clips to frame bounds.
pub fn blit_bitmap_subtitle(frame: &mut [u32], frame_w: u32, frame_h: u32, sub: &BitmapSubtitle) {
    if frame_w == 0 || frame_h == 0 || sub.width == 0 || sub.height == 0 {
        return;
    }
    let Some(frame_pixels) = (frame_w as usize).checked_mul(frame_h as usize) else {
        return;
    };
    let Some(sub_pixels) = (sub.width as usize).checked_mul(sub.height as usize) else {
        return;
    };
    if frame.len() < frame_pixels || sub.pixels.len() < sub_pixels {
        return;
    }
    let fw = i64::from(frame_w);
    let fh = i64::from(frame_h);
    let sw = i64::from(sub.width);
    let sh = i64::from(sub.height);
    for sy in 0..sh {
        let dy = i64::from(sub.y) + sy;
        if dy < 0 || dy >= fh {
            continue;
        }
        for sx in 0..sw {
            let dx = i64::from(sub.x) + sx;
            if dx < 0 || dx >= fw {
                continue;
            }
            let src = sub.pixels[(sy as usize) * (sub.width as usize) + (sx as usize)];
            let di = (dy as usize) * (frame_w as usize) + (dx as usize);
            frame[di] = blend_rgba_over_rgb(frame[di], src);
        }
    }
}

pub fn active_bitmap_subtitle<'a>(
    planes: &'a [BitmapSubtitle],
    now_us: i64,
) -> Option<&'a BitmapSubtitle> {
    planes
        .iter()
        .rev()
        .find(|plane| plane.start_us <= now_us && now_us < plane.end_us)
}

/// Plain text from an SRT line or an ASS/SSA event.
pub fn plain_subtitle(raw: &str) -> String {
    let payload = subtitle_payload(raw.trim());
    let mut out = String::new();
    let mut tag = false;
    let mut override_block = false;
    let mut chars = payload.chars().peekable();
    while let Some(ch) = chars.next() {
        if override_block {
            if ch == '}' {
                override_block = false;
            }
            continue;
        }
        if tag {
            if ch == '>' {
                tag = false;
            }
            continue;
        }
        if ch == '{' {
            override_block = true;
            continue;
        }
        if ch == '<' {
            tag = true;
            continue;
        }
        if ch == '\u{feff}' {
            continue;
        }
        if ch == '\\' {
            match chars.peek().copied() {
                Some('N') | Some('n') => {
                    chars.next();
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    continue;
                }
                Some('h') => {
                    chars.next();
                    out.push('\u{a0}');
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn subtitle_payload(raw: &str) -> &str {
    if let Some((key, rest)) = raw.split_once(':')
        && key.trim().eq_ignore_ascii_case("dialogue")
        && let Some(text) = field_after_commas(rest.trim_start(), 9)
    {
        return text;
    }
    let mut parts = raw.splitn(3, ',');
    if let (Some(head), Some(second)) = (parts.next(), parts.next())
        && !head.is_empty()
        && head.chars().all(|ch| ch.is_ascii_digit())
    {
        let second = second.trim();
        let layer_ok = second.bytes().all(|b| b.is_ascii_digit())
            || second.split_once('=').is_some_and(|(k, v)| {
                k.trim().eq_ignore_ascii_case("marked")
                    && !v.trim().is_empty()
                    && v.trim().bytes().all(|b| b.is_ascii_digit())
            });
        if layer_ok && let Some(text) = field_after_commas(raw, 8) {
            return text;
        }
    }
    raw
}

fn field_after_commas(raw: &str, commas: usize) -> Option<&str> {
    let mut rest = raw;
    for _ in 0..commas {
        let index = rest.find(',')?;
        rest = &rest[index + 1..];
    }
    Some(rest)
}

/// Display window for one decoded cue. Times are media microseconds.
pub fn subtitle_window(packet_us: i64, start_ms: u32, end_ms: u32) -> (i64, i64) {
    let start = packet_us.saturating_add(i64::from(start_ms).saturating_mul(1_000));
    let mut end = packet_us.saturating_add(i64::from(end_ms).saturating_mul(1_000));
    if end <= start {
        end = start.saturating_add(5_000_000);
    }
    (start, end)
}

/// The latest cue that covers `now_us`.
pub fn active_subtitle(cues: &[SubtitleCue], now_us: i64) -> Option<&str> {
    cues.iter()
        .rev()
        .find(|cue| now_us >= cue.start_us && now_us < cue.end_us)
        .map(|cue| cue.text.as_str())
}

/// Parse `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, or ASS `H:MM:SS.cs` into microseconds.
pub fn parse_subtitle_clock(token: &str) -> Option<i64> {
    let token = token.trim();
    let (hms, frac) = token
        .split_once(',')
        .or_else(|| token.split_once('.'))
        .unwrap_or((token, "0"));
    let mut fields = hms.split(':');
    let first = fields.next()?;
    let second = fields.next()?;
    let third = fields.next();
    if fields.next().is_some() { return None; }
    let (hours, minutes, seconds, bounded_minutes) = match third {
        Some(seconds) => (first, second, seconds, true),
        None => ("0", first, second, false),
    };
    let component = |value: &str| -> Option<i64> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) { return None; }
        value.parse().ok()
    };
    let hours = component(hours)?;
    let minutes = component(minutes)?;
    let seconds = component(seconds)?;
    if seconds >= 60 || (bounded_minutes && minutes >= 60)
        || frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = frac.len().min(3);
    let millis = frac.bytes().take(3).fold(0i64, |n, b| n * 10 + i64::from(b - b'0'))
        * 10i64.pow(3 - digits as u32);
    hours.checked_mul(3600)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(millis)?
        .checked_mul(1000)
}

/// Cues from an SRT document. Blocks without a timing line are skipped.
pub fn parse_srt(text: &str) -> Vec<SubtitleCue> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in normalized.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
        let Some(first) = lines.next() else {
            continue;
        };
        let timing = if first.contains("-->") {
            first
        } else if let Some(next) = lines.next() {
            next
        } else {
            continue;
        };
        let Some((start, end)) = timing.split_once("-->") else {
            continue;
        };
        let Some(start_token) = start.split_whitespace().last() else {
            continue;
        };
        let Some(start_us) = parse_subtitle_clock(start_token) else {
            continue;
        };
        let Some(end_token) = end.split_whitespace().next() else {
            continue;
        };
        let Some(end_us) = parse_subtitle_clock(end_token) else {
            continue;
        };
        let body = lines.collect::<Vec<_>>().join("\n");
        let text = plain_subtitle(&body);
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue {
            start_us,
            end_us: end_us.max(start_us.saturating_add(1)),
            text,
        });
    }
    cues
}

fn parse_ass(text: &str) -> Vec<SubtitleCue> {
    let has_sections = text.lines().any(|line| {
        let trimmed = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        trimmed.eq_ignore_ascii_case("[Script Info]") || trimmed.eq_ignore_ascii_case("[Events]")
    });
    let mut in_events = !has_sections;
    let mut format_cols: Option<(usize, usize, usize, usize)> = None;
    let mut cues = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_events = trimmed.eq_ignore_ascii_case("[Events]");
            format_cols = None;
            continue;
        }
        if !in_events {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("format") {
            let cols: Vec<String> = rest
                .split(',')
                .map(|col| col.trim().to_ascii_lowercase())
                .collect();
            if let (Some(start_idx), Some(end_idx), Some(text_idx)) = (
                cols.iter().position(|c| c == "start"),
                cols.iter().position(|c| c == "end"),
                cols.iter().position(|c| c == "text"),
            ) && cols.len() >= 4
                && text_idx + 1 == cols.len()
            {
                format_cols = Some((cols.len(), start_idx, end_idx, text_idx));
            }
            continue;
        }
        if !key.trim().eq_ignore_ascii_case("dialogue") {
            continue;
        }
        let (field_count, start_idx, end_idx, text_idx) = format_cols.unwrap_or((10, 1, 2, 9));
        let fields: Vec<&str> = rest.splitn(field_count, ',').collect();
        if fields.len() < field_count {
            continue;
        }
        let Some(start_us) = parse_subtitle_clock(fields[start_idx].trim()) else {
            continue;
        };
        let Some(end_us) = parse_subtitle_clock(fields[end_idx].trim()) else {
            continue;
        };
        let text = plain_subtitle(fields[text_idx]);
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue {
            start_us,
            end_us: end_us.max(start_us.saturating_add(1)),
            text,
        });
    }
    cues
}

/// Parse an SRT, WebVTT, or ASS/SSA document.
pub fn parse_subtitle_text(text: &str) -> Result<Vec<SubtitleCue>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let is_ass = text.lines().any(|line| {
        let trimmed = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        trimmed.eq_ignore_ascii_case("[Script Info]")
            || trimmed.eq_ignore_ascii_case("[Events]")
            || trimmed
                .split_once(':')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("dialogue"))
    });
    let cues = if is_ass {
        let ass = parse_ass(text);
        if ass.is_empty() && text.contains("-->") {
            parse_srt(text)
        } else {
            ass
        }
    } else if text.contains("-->") {
        parse_srt(text)
    } else {
        parse_ass(text)
    };
    if cues.is_empty() {
        return Err("subtitle file has no cues".into());
    }
    Ok(cues)
}

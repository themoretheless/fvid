//! Text subtitles read from a file next to the video.
//!
//! VLC renders these with libass; FVid's own player has no external
//! dependency, so SubRip, WebVTT, ASS/SSA and SAMI events are parsed here into
//! plain text cues. Styling a file carries is dropped deliberately — painting
//! styled text would need a font engine and a shaper, and the player draws the
//! cue with its own font, outline and position instead.

use std::path::Path;
use std::time::Duration;

/// One displayed line of text with the media time it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    /// First media time the text is on screen.
    pub start: Duration,
    /// First media time it is gone.
    pub end: Duration,
    /// Text with tags and override blocks removed, lines split by `\n`.
    pub text: String,
}

/// How far one press of the subtitle delay key moves the cue.
pub const DELAY_STEP: Duration = Duration::from_millis(50);

/// Read cues from whatever subtitle text is handed over, sniffing the format.
pub fn parse(text: &str) -> Vec<Cue> {
    if text.trim_start().starts_with("WEBVTT") {
        parse_webvtt(text)
    } else if text.contains("[Events]") {
        parse_ass(text)
    } else if is_sami(text) {
        parse_sami(text)
    } else {
        parse_srt(text)
    }
}

/// A SAMI file says what it is at its head, and no other caption format puts
/// `<sami` there; an XML declaration and a document type may stand in front of
/// the word, so the look reaches past the first bytes rather than only at them.
fn is_sami(text: &str) -> bool {
    let mut head = text.len().min(1 << 12);
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    text[..head].to_ascii_lowercase().contains("<sami")
}

/// `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, and the WebVTT short form `MM:SS.mmm`.
/// The fractional part is a decimal fraction of a second, so `,5` is 500 ms.
fn clock(text: &str) -> Option<Duration> {
    let (times, fraction) = match text.rsplit_once(['.', ',']) {
        Some((times, fraction)) => (times, Some(fraction)),
        None => (text, None),
    };
    let mut millis = 0;
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let digits: u64 = fraction.parse().ok()?;
        let scale = 10u64.checked_pow(fraction.len() as u32)?;
        millis = digits * 1_000 / scale;
    }
    let mut parts = times.split(':').rev();
    let seconds = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let hours = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Duration::from_secs(hours * 3_600 + minutes * 60 + seconds)
        .checked_add(Duration::from_millis(millis))
}

/// Parse a `start --> end` line. WebVTT cue settings after the end time are
/// ignored, and so is a cue identifier before the start time.
fn range(line: &str) -> Option<(Duration, Duration)> {
    let (left, right) = line.split_once("-->")?;
    let start = clock(left.split_whitespace().last()?)?;
    let end = clock(right.split_whitespace().next()?)?;
    (start < end).then_some((start, end))
}

/// Drop `<i>`-style tags and ASS `{…}` override blocks, and turn `\N` into a
/// real line break.
pub(crate) fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut tag = false;
    let mut override_block = false;
    for ch in text.chars() {
        match ch {
            '{' => override_block = true,
            '}' => override_block = false,
            '<' => tag = true,
            '>' => tag = false,
            _ if tag || override_block => {}
            _ => out.push(ch),
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Consume the text of a cue, which runs to the next blank line.
fn body(lines: &[&str], index: &mut usize) -> String {
    let start = *index;
    while *index < lines.len() && !lines[*index].trim().is_empty() {
        *index += 1;
    }
    plain(&lines[start..*index].join("\n"))
}

/// SubRip: an optional counter line, a range line, then text until a blank line.
pub fn parse_srt(text: &str) -> Vec<Cue> {
    cues_by_range(text.lines())
}

/// WebVTT shares SubRip's block layout; the header and the `NOTE`, `STYLE` and
/// `REGION` blocks carry no timing, so the range test skips them.
pub fn parse_webvtt(text: &str) -> Vec<Cue> {
    cues_by_range(text.lines())
}

fn cues_by_range<'a, I: Iterator<Item = &'a str>>(lines: I) -> Vec<Cue> {
    let lines: Vec<&str> = lines.collect();
    let mut cues = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        index += 1;
        let Some((start, end)) = range(line) else {
            continue;
        };
        let text = body(&lines, &mut index);
        if !text.is_empty() {
            cues.push(Cue { start, end, text });
        }
    }
    cues
}

/// ASS and SSA `[Events]` lines:
/// `Dialogue: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text`.
/// Only the timing and the text are read.
pub fn parse_ass(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Dialogue:") else {
            continue;
        };
        // The text field may itself contain commas, so it keeps the remainder.
        let fields: Vec<&str> = rest.splitn(10, ',').collect();
        if fields.len() < 10 {
            continue;
        }
        let (Some(start), Some(end)) = (clock(fields[1].trim()), clock(fields[2].trim())) else {
            continue;
        };
        let text = plain(fields[9]);
        if !text.is_empty() && start < end {
            cues.push(Cue { start, end, text });
        }
    }
    cues
}

/// The text one block of the ASS-family tracks (`S_TEXT/ASS` and `S_TEXT/SSA`)
/// shows, and the range it names for itself when it names one.
///
/// A block reaches the player in one of three shapes. The Matroska
/// documentation shows a whole `Dialogue:` line of ten fields and `mkvmerge`
/// writes it; `ffmpeg` leaves the timing to the container instead and writes
/// the same fields with the two clocks taken out, which the measured blocks of
/// a file it muxed show as `0,0,Default,,0,0,0,,text`; and a writer that knows
/// nothing about ASS leaves the text alone. Only a line whose own clocks make a
/// range can move off the time of its block — clocks zeroed out are the writer
/// saying that the container decides. A fourth shape a dialogue line has, the
/// field list of an older script, names no text the reader can place and shows
/// nothing.
pub fn ass_block(block: &str) -> (Option<(Duration, Duration)>, String) {
    let line = block.trim();
    let (prefixed, fields) = match line.split_once(':') {
        Some(("Dialogue", rest)) => (true, rest),
        // The other event lines of a script are not for showing.
        Some(("Comment" | "Format" | "Style", _)) => return (None, String::new()),
        _ => (false, line),
    };
    let ten = fields.splitn(10, ',').collect::<Vec<_>>();
    if ten.len() == 10
        && let Some((start, end)) = clock(ten[1].trim()).zip(clock(ten[2].trim()))
    {
        return (
            Some((start, end)).filter(|(start, end)| start < end),
            plain(ten[9].trim()),
        );
    }
    // Without two clocks to read the line is told from plain text by the two
    // numbers that head its field list, which no line of text starts with.
    let nine = fields.splitn(9, ',').collect::<Vec<_>>();
    let fields_form = nine.len() == 9
        && nine[..2]
            .iter()
            .all(|field| !field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()));
    if fields_form {
        return (None, plain(nine[8].trim()));
    }
    // A dialogue line of neither shape belongs to a script whose fields are
    // listed differently than ASS lists them, and where its text then sits
    // cannot be told — the older SSA scripts put it after six fields, for
    // instance. Such a line would show its own field list, so it shows nothing.
    if prefixed {
        return (None, String::new());
    }
    (None, plain(fields.trim()))
}

/// One caption line of a SAMI file, with what the tag around it said.
struct SamiLine {
    /// The language the tag names for itself, folded; one file commonly holds
    /// the same film captioned in several.
    language: String,
    /// The time the line comes on, which a line without one takes from the line
    /// before it.
    start: Option<Duration>,
    /// The time the file says it leaves, when it says one at all.
    end: Option<Duration>,
    /// Text with the markup taken out and `<br>` become a real line break.
    text: String,
}

/// SAMI: the captions of a Windows Media file, where every line is a `<p>` whose
/// timing sits in the tag's own attributes instead of in a range line. `Begin`
/// and `End` count milliseconds, and the older `Sync` names a clock the way
/// SubRip does, so the two forms are read apart rather than guessed at.
pub fn parse_sami(text: &str) -> Vec<Cue> {
    let folded = text.to_ascii_lowercase();
    let mut lines = Vec::new();
    let mut heard = None;
    for (tag, body) in sami_tags(&folded, text) {
        let start = attribute(tag, "begin")
            .and_then(milliseconds)
            .or_else(|| attribute(tag, "sync").filter(|clock| clock.contains(':')).and_then(clock));
        let start = start.or(heard);
        heard = start;
        lines.push(SamiLine {
            language: attribute(tag, "class").unwrap_or_default().to_owned(),
            start,
            end: attribute(tag, "end").and_then(milliseconds),
            text: plain(&line_breaks(body)),
        });
    }
    let wanted = sami_language(&lines);
    let mut shown: Vec<(Duration, Option<Duration>, String)> = Vec::new();
    for line in lines
        .iter()
        .filter(|line| line.language == wanted && !line.text.is_empty())
    {
        let Some(start) = line.start else { continue };
        match shown.last_mut().filter(|last| last.0 == start) {
            // A line that names no time of its own is the second line of the
            // caption before it, not a caption that replaces it.
            Some(last) => {
                last.1 = last.1.max(line.end);
                last.2.push('\n');
                last.2.push_str(&line.text);
            }
            None => shown.push((start, line.end, line.text.clone())),
        }
    }
    let starts: Vec<Duration> = shown.iter().map(|(start, _, _)| *start).collect();
    let mut cues = Vec::with_capacity(shown.len());
    for (nth, (start, end, text)) in shown.into_iter().enumerate() {
        let next = starts.get(nth + 1).copied();
        let end = end
            .filter(|end| *end > start)
            .or_else(|| next.filter(|next| *next > start))
            .unwrap_or_else(|| start.saturating_add(LAST_LINE));
        cues.push(Cue { start, end, text });
    }
    cues
}

/// The language a SAMI file captions in: the one its lines mostly carry, since a
/// file written for two audiences holds both in the same paragraphs and a viewer
/// reads one at a time. Ties go to the language that appears first, which is the
/// one the writer put in front.
fn sami_language(lines: &[SamiLine]) -> &str {
    let mut counted: Vec<(&str, usize)> = Vec::new();
    for line in lines.iter().filter(|line| !line.text.is_empty()) {
        match counted
            .iter_mut()
            .find(|(seen, _)| *seen == line.language.as_str())
        {
            Some((_, seen)) => *seen += 1,
            None => counted.push((line.language.as_str(), 1)),
        }
    }
    let mut best = 0;
    let mut wanted = "";
    for (language, seen) in &counted {
        if *seen > best {
            best = *seen;
            wanted = language;
        }
    }
    wanted
}

/// The `<p>` tags of a SAMI body, each with the text that runs until the next
/// one. A tag comes back folded because its attributes are matched case-blind;
/// its text keeps the letters the file wrote.
fn sami_tags<'a>(folded: &'a str, text: &'a str) -> Vec<(&'a str, &'a str)> {
    let bytes = folded.as_bytes();
    let last = folded.find("</body").unwrap_or(folded.len());
    let first = folded
        .find("<body")
        .map_or(0, |at| at + "<body".len())
        .min(last);
    let mut opens = Vec::new();
    let mut at = first;
    while at < last {
        // `<p` opens a caption; `</p>` closes one, and `<param>` and `<pre>` are
        // neither, so the byte after the name has to be the end of the name.
        if bytes[at] == b'<'
            && bytes.get(at + 1) == Some(&b'p')
            && !bytes.get(at + 2).is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            opens.push(at);
            at += 2;
        } else {
            at += 1;
        }
    }
    let mut found = Vec::with_capacity(opens.len());
    for (nth, open) in opens.iter().enumerate() {
        let tag_end = tag_closed_at(bytes, *open).min(last);
        let body_end = opens.get(nth + 1).copied().unwrap_or(last);
        let tag = folded.get(*open..tag_end).unwrap_or_default();
        let body = text.get(tag_end..body_end).unwrap_or_default();
        found.push((tag, body));
    }
    found
}

/// The byte past the `>` that ends a tag, which an attribute value may have put
/// inside itself by quoting one: `<p class="a>b">` is one tag.
fn tag_closed_at(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    let mut quote = None;
    while at < bytes.len() {
        match bytes[at] {
            byte @ (b'"' | b'\'') if Some(byte) == quote => quote = None,
            byte @ (b'"' | b'\'') if quote.is_none() => quote = Some(byte),
            b'>' if quote.is_none() => return at + 1,
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

/// The value a folded tag gives an attribute: `name=value`, `name = "value"`, in
/// either case the writer chose. A name that only appears inside another
/// attribute's value names nothing.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut from = 0;
    loop {
        let at = tag[from..].find(name)? + from;
        from = at + name.len();
        if at != 0 && bytes[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        let Some(rest) = tag.get(at + name.len()..).map(str::trim_start) else {
            continue;
        };
        let Some(value) = rest.strip_prefix('=').map(str::trim_start) else {
            continue;
        };
        return match value.strip_prefix(['"', '\'']) {
            Some(quoted) => Some(quoted.split(['"', '\'']).next().unwrap_or_default()),
            None => Some(value.split_whitespace().next()?.trim_end_matches(['/', '>'])),
        };
    }
}

/// The time a `Begin` or `End` attribute names, in the milliseconds the format
/// counts them in. A value that is not a plain number names no time.
fn milliseconds(value: &str) -> Option<Duration> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().map(Duration::from_millis)
}

/// The line breaks of a SAMI caption. The format writes one as `<br>`, `<br/>`
/// or `<br class=…>`; every other tag is markup `plain()` already drops.
fn line_breaks(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut at, mut from) = (0, 0);
    while at < bytes.len() {
        if bytes[at] == b'<'
            && bytes.get(at + 1).is_some_and(|byte| *byte == b'b')
            && bytes.get(at + 2).is_some_and(|byte| *byte == b'r')
            && !bytes.get(at + 3).is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            out.push_str(&text[from..at]);
            out.push('\n');
            at = tag_closed_at(bytes, at);
            from = at;
        } else {
            at += 1;
        }
    }
    out.push_str(&text[from..]);
    out
}

/// The cue to show at a media time, with the cues shifted by `delay`.
pub fn active(cues: &[Cue], at: Duration, delay: Duration) -> Option<&Cue> {
    cues.iter().find(|cue| {
        cue.start
            .checked_add(delay)
            .is_some_and(|start| start <= at && at < cue.end.saturating_add(delay))
    })
}

/// How long the last line of a container's subtitle track stays when the track
/// declares no length of its own and no later line can end it.
pub const LAST_LINE: Duration = Duration::from_secs(2);

/// When one embedded line leaves the screen. The length the container states
/// for the line wins, because that is what the track promises; without one the
/// line lasts until its successor starts, and the last of a track gets the
/// fallback length. Both containers that carry lines in their own stream —
/// Matroska blocks and MP4 samples — ask this.
pub fn cue_end(start: Duration, stated: Duration, next: Option<Duration>) -> Duration {
    if !stated.is_zero() {
        return start.saturating_add(stated);
    }
    next.filter(|at| *at > start)
        .unwrap_or_else(|| start.saturating_add(LAST_LINE))
}

/// Sidecar subtitle files for a video: an exact name match first, then any
/// other subtitle whose name contains the video's stem.
pub fn sidecars(video: &Path) -> Vec<std::path::PathBuf> {
    let (Some(directory), Some(stem)) = (video.parent(), video.file_stem()) else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy().to_ascii_lowercase();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut exact = Vec::new();
    let mut loose = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_subtitle = matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .as_deref(),
            Some("srt") | Some("vtt") | Some("ass") | Some("ssa") | Some("smi") | Some("smil")
        );
        if !is_subtitle {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
        else {
            continue;
        };
        if name == stem {
            exact.push(path);
        } else if name.contains(&stem) {
            loose.push(path);
        }
    }
    exact.sort();
    loose.sort();
    exact.extend(loose);
    exact
}

/// The high half of the windows-1251 mapping, the encoding Cyrillic
/// sidecars still commonly arrive in and the one VLC's text reader falls
/// back to when a file is not UTF-8. Every byte stands for something except
/// the one value the codec never defined.
const WINDOWS_1251: [char; 128] = [
    '\u{0402}', '\u{0403}', '\u{201A}', '\u{0453}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{20AC}', '\u{2030}', '\u{0409}', '\u{2039}', '\u{040A}', '\u{040C}', '\u{040B}', '\u{040F}',
    '\u{0452}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{FFFD}', '\u{2122}', '\u{0459}', '\u{203A}', '\u{045A}', '\u{045C}', '\u{045B}', '\u{045F}',
    '\u{00A0}', '\u{040E}', '\u{045E}', '\u{0408}', '\u{00A4}', '\u{0490}', '\u{00A6}', '\u{00A7}',
    '\u{0401}', '\u{00A9}', '\u{0404}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{0407}',
    '\u{00B0}', '\u{00B1}', '\u{0406}', '\u{0456}', '\u{0491}', '\u{00B5}', '\u{00B6}', '\u{00B7}',
    '\u{0451}', '\u{2116}', '\u{0454}', '\u{00BB}', '\u{0458}', '\u{0405}', '\u{0455}', '\u{0457}',
    '\u{0410}', '\u{0411}', '\u{0412}', '\u{0413}', '\u{0414}', '\u{0415}', '\u{0416}', '\u{0417}',
    '\u{0418}', '\u{0419}', '\u{041A}', '\u{041B}', '\u{041C}', '\u{041D}', '\u{041E}', '\u{041F}',
    '\u{0420}', '\u{0421}', '\u{0422}', '\u{0423}', '\u{0424}', '\u{0425}', '\u{0426}', '\u{0427}',
    '\u{0428}', '\u{0429}', '\u{042A}', '\u{042B}', '\u{042C}', '\u{042D}', '\u{042E}', '\u{042F}',
    '\u{0430}', '\u{0431}', '\u{0432}', '\u{0433}', '\u{0434}', '\u{0435}', '\u{0436}', '\u{0437}',
    '\u{0438}', '\u{0439}', '\u{043A}', '\u{043B}', '\u{043C}', '\u{043D}', '\u{043E}', '\u{043F}',
    '\u{0440}', '\u{0441}', '\u{0442}', '\u{0443}', '\u{0444}', '\u{0445}', '\u{0446}', '\u{0447}',
    '\u{0448}', '\u{0449}', '\u{044A}', '\u{044B}', '\u{044C}', '\u{044D}', '\u{044E}', '\u{044F}',
];

/// The character windows-1251 gives a byte: it agrees with ASCII below the
/// high half, where `WINDOWS_1251` takes over.
fn windows_1251(byte: u8) -> char {
    if byte < 0x80 {
        char::from(byte)
    } else {
        WINDOWS_1251[usize::from(byte) - 0x80]
    }
}

/// UTF-16 units in the byte order the file's leading mark chose. A final odd
/// byte is a half-unit and goes unheard, which `chunks_exact` already means.
fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Read the bytes of a subtitle file as text, the way VLC's reader chooses
/// its encoding: a leading byte-order mark names UTF-8 or UTF-16 outright,
/// and text without one is UTF-8 if it is and windows-1251 otherwise. The
/// fallback maps every byte, so this cannot fail — a file of neither text
/// simply yields no cues downstream.
fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE][..]) {
        return decode_utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF][..]) {
        return decode_utf16(rest, false);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| windows_1251(byte)).collect(),
    }
}

/// Read a sidecar, giving up on one that holds no cues. `decode_text` picks
/// the encoding, so a windows-1251 or UTF-16 file loads as well as a UTF-8
/// one.
pub fn load(path: &Path) -> Option<Vec<Cue>> {
    let cues = parse(&decode_text(&std::fs::read(path).ok()?));
    (!cues.is_empty()).then_some(cues)
}

#[cfg(test)]
mod tests {
    use super::{
        DELAY_STEP, LAST_LINE, active, ass_block, clock, cue_end, decode_text, load, parse,
        parse_ass, parse_sami, parse_srt, parse_webvtt, sidecars,
    };
    use std::time::Duration;

    fn seconds(value: f64) -> Duration {
        Duration::try_from_secs_f64(value).unwrap()
    }

    #[test]
    fn clocks_accept_both_separators_and_the_short_form() {
        assert_eq!(clock("00:01:02,500"), Some(seconds(62.5)));
        assert_eq!(clock("00:01:02.500"), Some(seconds(62.5)));
        assert_eq!(clock("01:02.5"), Some(seconds(62.5)));
        assert_eq!(clock("0:00:00,000"), Some(Duration::ZERO));
        assert_eq!(clock("2:03:04"), Some(seconds(7384.0)));
        assert_eq!(clock("not a clock"), None);
        assert_eq!(clock("1:2:3:4"), None);
        assert_eq!(clock("00:01:02,abc"), None);
    }

    #[test]
    fn subrip_cues_keep_their_counter_and_blank_lines() {
        let cues = parse_srt(
            "1\n00:00:01,000 --> 00:00:02,000\nFirst line\nsecond line\n\n2\n00:00:03,000 --> 00:00:04,000\n<i>Styled</i> out\n",
        );
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "First line\nsecond line");
        assert_eq!(cues[1].text, "Styled out");
        assert_eq!(cues[0].start, seconds(1.0));
        assert_eq!(cues[0].end, seconds(2.0));
    }

    #[test]
    fn webvtt_skips_its_header_notes_and_cue_settings() {
        let cues = parse_webvtt(
            "WEBVTT\n\nNOTE a comment\n\n1\n00:00:01.000 --> 00:00:02.000 align:start position:10%\n- Hello\n\n",
        );
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "- Hello");
        assert_eq!(cues[0].end, seconds(2.0));
    }

    #[test]
    fn ass_events_keep_text_after_the_tenth_field() {
        let cues = parse_ass(
            "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an2}Hello, world{\\*}\\Nsecond\nComment: 0,0:00:01.00,0:00:02.00,,0,0,0,,hidden\n",
        );
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Hello, world\nsecond");
    }

    /// The shapes an ASS block reaches the player in. The first two are what
    /// real muxers write: the ten-field line the Matroska documentation shows,
    /// and the line `ffmpeg` muxes with its two clocks taken out — the second
    /// string below is the first block of a file it muxed, byte for byte.
    #[test]
    fn an_ass_block_gives_up_its_text_whichever_fields_it_keeps() {
        let (range, text) =
            ass_block("Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an2}One, two\\Nthree");
        assert_eq!(range, Some((seconds(1.0), seconds(2.0))));
        assert_eq!(text, "One, two\nthree");
        let (range, text) = ass_block("0,0,Default,,0,0,0,,{\\an2}First\\Nsecond line");
        assert_eq!(range, None);
        assert_eq!(text, "First\nsecond line");
        // Clocks zeroed out are the writer handing the time back to the file.
        let (range, text) = ass_block("Dialogue: 0,0:00:00.00,0:00:00.00,Default,,0,0,0,,Plain");
        assert_eq!((range, text), (None, "Plain".to_owned()));
        // Plain text keeps every comma it has.
        let (range, text) = ass_block("Hello, world, this is text");
        assert_eq!(
            (range, text),
            (None, "Hello, world, this is text".to_owned())
        );
        // A line the script does not mean to show.
        assert_eq!(
            ass_block("Comment: 0,0:00:01.00,0:00:02.00,,0,0,0,,hidden"),
            (None, String::new())
        );
        // Clocks the wrong way round name no range.
        assert_eq!(
            ass_block("Dialogue: 0,0:00:05.00,0:00:02.00,Default,,0,0,0,,Backwards").0,
            None
        );
    }

    /// A script line that calls itself a dialogue but is laid out with fewer or
    /// more fields than ASS is puts its text nowhere the reader can tell, so it
    /// shows its own field list instead of a line — which is why it shows
    /// nothing at all.
    #[test]
    fn a_script_line_of_another_field_list_shows_nothing() {
        assert_eq!(
            ass_block("Dialogue: 0,0:00:02.00,0:00:04.00,Default,,Six fields"),
            (None, String::new())
        );
        assert_eq!(
            ass_block("Dialogue: 0,0:00:02.00,0:00:04.00,,0,,Old style"),
            (None, String::new())
        );
        // The same words without the head of a script line are still text.
        let line = "Six fields, 0:00:02.00, not a dialogue";
        assert_eq!(ass_block(line), (None, line.to_owned()));
    }

    #[test]
    fn sniffing_picks_the_format_by_its_header() {
        assert_eq!(
            parse("WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHi\n").len(),
            1
        );
        assert_eq!(parse("1\n00:00:01,000 --> 00:00:02,000\nHi\n").len(), 1);
        assert_eq!(
            parse("[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,,,,,,,Hi\n").len(),
            1
        );
        assert_eq!(parse("<SAMI>\n<BODY>\n<p class=eng Begin=0 End=1000>Hi\n").len(), 1);
        // A declaration in front of the word is the format's own, too.
        assert_eq!(
            parse(
                "<?xml version=\"1.0\"?>\n<SAMI Class=\"SMIL\">\n<BODY>\n<p class=eng Begin=0 End=1000>Hi\n"
            )
            .len(),
            1
        );
    }

    /// A SAMI paragraph keeps its own clocks in its tag, spelled in either case
    /// and with values either bare or quoted; its markup is dropped the way an
    /// ASS line's is, and its `<br>` is the line break the format writes. The
    /// head of the file speaks about the film, not about a caption, so nothing
    /// there shows.
    #[test]
    fn a_sami_paragraph_carries_its_own_clock() {
        let cues = parse_sami(
            "<SAMI>\n<HEAD><TITLE>Sample</TITLE></HEAD>\n<BODY>\n<Div Class=English>\n\
             <P CLASS=English BEGIN=1000 END=2500>Hello<br><font color=\"#FFFF00\">world</font>\n\
             <p class=\"English\" begin=3000 end=4000>Second cue\n\
             </Div>\n</BODY>\n",
        );
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Hello\nworld");
        assert_eq!((cues[0].start, cues[0].end), (seconds(1.0), seconds(2.5)));
        assert_eq!((cues[1].start, cues[1].end), (seconds(3.0), seconds(4.0)));
    }

    /// A paragraph that names no time of its own continues the caption before
    /// it rather than replacing it, and one that names no end holds the screen
    /// until its successor starts — or, for the last of a file, the length every
    /// other format's untimed line takes too. A time that only appears inside a
    /// quoted attribute value names nothing, so the tag has to be read whole.
    #[test]
    fn a_sami_line_inherits_the_time_it_was_given() {
        let cues = parse_sami(
            "<SAMI>\n<BODY>\n<p class=eng Begin=1000>One\n<p class=eng>Still one\n\
             <p class=eng Begin=4000>Two\n<p class=eng Begin=9000>Three\n",
        );
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].text, "One\nStill one");
        assert_eq!((cues[0].start, cues[0].end), (seconds(1.0), seconds(4.0)));
        assert_eq!((cues[1].start, cues[1].end), (seconds(4.0), seconds(9.0)));
        assert_eq!(cues[2].end, seconds(9.0) + LAST_LINE);
        // The `>` inside the value is part of the value, so the tag runs past it.
        let timed = parse_sami(
            "<SAMI>\n<BODY>\n<p Note=\"a>b\" class=eng Begin=7000 End=8000>Odd\n",
        );
        assert_eq!(
            (timed.len(), timed[0].text.as_str(), timed[0].start, timed[0].end),
            (1, "Odd", seconds(7.0), seconds(8.0)),
        );
    }

    /// Two audiences, one file: the language the paragraphs mostly carry is the
    /// one shown, because a viewer reads one caption at a time, and where the
    /// two are even the writer's own order decides.
    #[test]
    fn a_sami_file_shows_the_language_it_captions_in() {
        let cues = parse_sami(
            "<SAMI>\n<BODY>\n<p class=rus Begin=0 End=2000>Привет\n\
             <p class=eng Begin=0 End=2000>Hello\n<p class=eng Begin=3000 End=5000>Second\n",
        );
        assert_eq!(
            cues.iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>()
                .join(" / "),
            "Hello / Second"
        );
        let tied = parse_sami("<SAMI>\n<BODY>\n<p class=rus Begin=0>Привет\n<p class=eng Begin=1000>Hello\n");
        assert_eq!(tied.len(), 1);
        assert_eq!(tied[0].text, "Привет");
    }

    /// Before milliseconds were the unit, `Sync` named a clock the way SubRip
    /// does. It is a different shape of number, so the two are told apart by what
    /// they hold rather than by which attribute wrote them.
    #[test]
    fn an_old_sami_sync_names_a_clock() {
        let cues = parse_sami(
            "<SAMI>\n<BODY>\n<p class=eng Sync=0:00:02.000>Old form\n\
             <p class=eng Sync=0:00:05.500>Next\n",
        );
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start, cues[0].end), (seconds(2.0), seconds(5.5)));
        // A paragraph the file gives no time at all has nowhere to sit.
        assert!(parse_sami("<SAMI>\n<BODY>\n<p class=eng>Nowhere\n").is_empty());
    }

    #[test]
    fn a_cue_is_live_between_its_times() {
        let cues = parse_srt("1\n00:00:01,000 --> 00:00:02,000\nHi\n");
        assert_eq!(active(&cues, seconds(0.5), Duration::ZERO), None);
        assert_eq!(
            active(&cues, seconds(1.0), Duration::ZERO).map(|c| c.text.as_str()),
            Some("Hi")
        );
        assert_eq!(active(&cues, seconds(2.0), Duration::ZERO), None);
    }

    /// What an embedded line leaves on screen, and for how long: the length the
    /// container states beats the gap to the next line, which beats the
    /// fallback the last line of a bare track needs.
    #[test]
    fn a_stated_length_outlines_the_line_it_states() {
        let milliseconds = Duration::from_millis;
        assert_eq!(
            cue_end(
                milliseconds(1_000),
                milliseconds(1_500),
                Some(milliseconds(4_000))
            ),
            milliseconds(2_500)
        );
        assert_eq!(
            cue_end(
                milliseconds(1_000),
                Duration::ZERO,
                Some(milliseconds(4_000))
            ),
            milliseconds(4_000)
        );
        // A line that outlives its successor keeps the screen until the fallback.
        assert_eq!(
            cue_end(milliseconds(1_000), Duration::ZERO, Some(milliseconds(900))),
            milliseconds(3_000)
        );
        assert_eq!(
            cue_end(milliseconds(1_000), Duration::ZERO, None),
            milliseconds(3_000)
        );
        assert_eq!(LAST_LINE, milliseconds(2_000));
    }

    /// Positive delay shows the cue later, which is what a late voice track
    /// needs; the cue itself never moves.
    #[test]
    fn delay_shifts_the_whole_file() {
        let cues = parse_srt("1\n00:00:01,000 --> 00:00:02,000\nHi\n");
        assert_eq!(active(&cues, seconds(1.04), DELAY_STEP), None);
        assert_eq!(active(&cues, seconds(1.05), DELAY_STEP).unwrap().text, "Hi");
        assert_eq!(active(&cues, seconds(0.96), DELAY_STEP), None);
        assert_eq!(active(&cues, seconds(2.04), DELAY_STEP).unwrap().text, "Hi");
        assert_eq!(active(&cues, seconds(2.05), DELAY_STEP), None);
    }

    #[test]
    fn an_end_before_the_start_is_not_a_cue() {
        assert!(parse_srt("1\n00:00:02,000 --> 00:00:01,000\nBad\n").is_empty());
        assert!(parse_srt("not a subtitle file at all\n").is_empty());
    }

    /// What sits next to the video decides the default track: the files named
    /// after it come first (case-insensitively), then anything merely
    /// containing its name, and unrelated files not at all.
    #[test]
    fn sidecars_sort_exact_names_before_loose_ones() {
        let directory = std::env::temp_dir().join("fvid-subtitle-sidecars");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let cue = "1\n00:00:01,000 --> 00:00:02,000\nHi\n";
        for name in ["Movie.mp4", "movie.srt", "MOVIE.vtt", "movie.extra.srt"] {
            std::fs::write(directory.join(name), cue).unwrap();
        }
        std::fs::write(directory.join("other.srt"), cue).unwrap();
        std::fs::write(directory.join("movie.txt"), "not a subtitle file\n").unwrap();
        std::fs::write(
            directory.join("movie.smi"),
            "<SAMI>\n<BODY>\n<p class=eng Begin=0 End=1000>Hi\n",
        )
        .unwrap();

        let video = directory.join("Movie.mp4");
        let found: Vec<String> = sidecars(&video)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["MOVIE.vtt", "movie.smi", "movie.srt", "movie.extra.srt"]);
        // A loose match only counts when it carries the whole stem.
        assert!(!found.contains(&"other.srt".to_string()));
        // Reading is separate from listing: an unparseable sibling yields none.
        assert!(load(&directory.join("movie.txt")).is_none());
        assert_eq!(load(&video.clone().with_extension("srt")).unwrap().len(), 1);
        assert_eq!(load(&video.with_extension("smi")).unwrap()[0].text, "Hi");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_mark_at_the_head_of_a_file_names_its_encoding() {
        // UTF-8 with its mark: the mark is dropped, the text stands.
        assert_eq!(decode_text("\u{FEFF}Привет".as_bytes()), "Привет");
        // UTF-16 in either order, which the mark alone can tell.
        let mut little = vec![0xFF, 0xFE];
        little.extend("Привет".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_text(&little), "Привет");
        let mut big = vec![0xFE, 0xFF];
        big.extend("Привет".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode_text(&big), "Привет");
        // Unmarked UTF-8 needs no mark to pass through unchanged.
        assert_eq!(decode_text("привет world".as_bytes()), "привет world");
    }

    #[test]
    fn a_windows_1251_file_reads_as_the_text_it_holds() {
        // «Привет, мир» in the bytes a Cyrillic rip carries.
        let bytes = [
            0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, 0x2C, 0x20, 0xEC, 0xE8, 0xF0,
        ];
        assert_eq!(decode_text(&bytes), "Привет, мир");
        // The two halves of the letters run in alphabet order, and the
        // outlier Ё keeps its places outside them.
        assert_eq!(decode_text(&[0xC0, 0xE0, 0xA8, 0xB8]), "АаЁё");
        // The one byte the codec never defined says so rather than passing.
        assert_eq!(decode_text(&[0x98]), "\u{FFFD}");
    }

    #[test]
    fn a_sidecar_in_a_legacy_encoding_loads_with_its_text() {
        let directory = std::env::temp_dir().join("fvid-subtitle-encoding");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut bytes = b"1\n00:00:01,000 --> 00:00:02,000\n".to_vec();
        bytes.extend([0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]); // Привет, windows-1251
        bytes.push(b'\n');
        std::fs::write(directory.join("movie.srt"), &bytes).unwrap();
        let cues = load(&directory.join("movie.srt")).expect("the file holds a cue");
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Привет");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

//! MP4 and QuickTime text subtitle tracks.
//!
//! Reads the samples of one `tx3g` track and turns them into the cue list the
//! player paints, so a file with embedded subtitles needs no sidecar. The
//! timestamps stay in the track's own clock, which is what the picture of the
//! same file is timed in, and the edits of the track are left out of it for the
//! same reason the video source leaves them out.

use crate::container::mp4::{Limits, Mp4Reader};
use crate::playback_mp4::MediaTime;
use crate::subtitles::{self, Cue};
use crate::{Result, invalid};
use std::io::{Read, Seek};
use std::time::Duration;

/// The four handler types a muxer gives a text subtitle track: `sbtl` is what
/// ffmpeg writes, `text` and `subt` what QuickTime writes.
const HANDLERS: [[u8; 4]; 3] = [*b"sbtl", *b"text", *b"subt"];

/// One embedded subtitle track, described by what the container carries.
pub struct Mp4SubtitleTrack {
    /// Handler type of the track, e.g. `sbtl`.
    pub handler: [u8; 4],
    /// Sample entry type, which is `tx3g` for every track this reads.
    pub codec: [u8; 4],
    /// `trak/udta/name`, empty when the file names no track.
    pub name: String,
    /// The language the track's `mdhd` states, empty when it states none.
    pub language: String,
}

impl Mp4SubtitleTrack {
    /// Name for the on-screen track list: the title the file gives the track,
    /// else the language it states, else the format it is stored in. A file
    /// that states neither is known by its format, as it was before.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        if !self.language.is_empty() {
            return self.language.clone();
        }
        match &self.codec {
            b"tx3g" => "mov_text".to_owned(),
            other => String::from_utf8_lossy(other).into_owned(),
        }
    }
}

/// Reader over the plain-text subtitle tracks of an MP4 or QuickTime file.
pub struct Mp4SubtitleReader<R> {
    demuxer: Mp4Reader<R>,
}

impl<R: Read + Seek> Mp4SubtitleReader<R> {
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        Ok(Self {
            demuxer: Mp4Reader::open(reader, limits)?,
        })
    }

    /// Indices into the track list of the readable subtitle tracks, in the
    /// order the file holds them.
    fn supported(&self) -> Vec<usize> {
        self.demuxer
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, track)| HANDLERS.contains(&track.handler) && track.codec == *b"tx3g")
            .map(|(index, _)| index)
            .collect()
    }

    /// The readable tracks, in container order.
    pub fn tracks(&self) -> Vec<Mp4SubtitleTrack> {
        self.supported()
            .iter()
            .map(|index| {
                let track = &self.demuxer.tracks()[*index];
                Mp4SubtitleTrack {
                    handler: track.handler,
                    codec: track.codec,
                    name: track.name.clone(),
                    language: track.language.clone(),
                }
            })
            .collect()
    }

    /// Cues of the `nth` readable track, counting in container order.
    pub fn cues(&mut self, nth: usize) -> Result<Vec<Cue>> {
        let &track_index = self
            .supported()
            .get(nth)
            .ok_or_else(|| invalid("MP4 has no such subtitle track"))?;
        let track = &self.demuxer.tracks()[track_index];
        let timescale = track.timescale;
        // The samples are indexed in decode order, which for a text track is
        // the order the lines are read in; sorting anyway keeps a file whose
        // edit list reorders them in the same shape as one that does not.
        let mut lines: Vec<(i64, u32, usize)> = (0..track.samples.len())
            .filter_map(|index| {
                let sample = track.samples.get(index)?;
                Some((sample.pts, sample.duration, index))
            })
            .collect();
        lines.sort_by_key(|(pts, _, index)| (*pts, *index));
        let clock = |ticks: i64| {
            MediaTime { ticks, timescale }
                .nanoseconds()
                .ok()
                .and_then(|nanos| u64::try_from(nanos).ok())
                .map(Duration::from_nanos)
        };
        let mut packet = Vec::new();
        let mut cues = Vec::with_capacity(lines.len());
        for (position, (pts, duration, index)) in lines.iter().enumerate() {
            self.demuxer.read_packet(track_index, *index, &mut packet)?;
            let Some(text) = sample_text(&packet) else {
                continue;
            };
            let start = clock(*pts).unwrap_or_default();
            let stated = clock(i64::from(*duration)).unwrap_or_default();
            let next = lines.get(position + 1).and_then(|(pts, _, _)| clock(*pts));
            cues.push(Cue {
                start,
                end: subtitles::cue_end(start, stated, next),
                text,
            });
        }
        Ok(cues)
    }
}

/// The letters one `tx3g` sample carries, with the markup of an `html` box
/// stripped the way a sidecar's are.
///
/// Three muxers, three spellings. `ffmpeg -c:s mov_text` and QuickTime's own
/// text media write a 16-bit length and the string after it. A 3GPP timed text
/// group puts a length-stated run of display-parameter boxes in front of that,
/// and the string gets a length of its own. A sample that opens with two zero
/// bytes states no length at all: there the bytes are the high half of the
/// first box's 32-bit size, and the sample is a chain of boxes. Style, karaoke
/// and font boxes say how a line is drawn, which this player asks nowhere, so
/// they are dropped for the same reason an ASS sidecar's styling is.
fn sample_text(sample: &[u8]) -> Option<String> {
    let head = sample.get(..2)?;
    let prefix = u16::from_be_bytes([head[0], head[1]]) as usize;
    let body = &sample[2..];
    if prefix > body.len() {
        // A string the sample does not hold is not worth guessing at.
        return None;
    }
    if prefix == 0 {
        // Two zero bytes state no text. There the sample is either a chain of
        // boxes, whose first 32-bit size begins with them, or a timed text
        // group that carries no display parameters at all.
        return box_text(sample).or_else(|| group_text(body, 0));
    }
    group_text(body, prefix).or_else(|| plain(&body[..prefix]))
}

/// The string a timed text group states after `params` bytes of display
/// parameters. With parameters the group ends at its string, so the length has
/// to cover what is left of the sample exactly; a shorter run of bytes is a
/// padded string rather than a group. With none, `ffmpeg -c:s mov_text` still
/// leaves the style boxes of a marked-up line after its string, and only the
/// string is shown.
fn group_text(body: &[u8], params: usize) -> Option<String> {
    let header = body.get(params..params + 2)?;
    let stated = u16::from_be_bytes([header[0], header[1]]) as usize;
    let text = body.get(params + 2..)?;
    if text.len() < stated || (params > 0 && text.len() != stated) {
        return None;
    }
    let text = &text[..stated];
    // Without parameters the length is also the first box's size and the state
    // is only settled by what the bytes it names are: a line rather than the
    // boxes around one.
    if params == 0 && !looks_like_text(text) {
        return None;
    }
    plain(text)
}

/// Bytes that read as a line of subtitles rather than as the boxes around one:
/// UTF-8 with no control characters beyond the whitespace a line may hold.
fn looks_like_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
        && !bytes
            .iter()
            .any(|byte| *byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r'))
}

/// Letters inside a chain of sample boxes: `text` holds them plain and `html`
/// holds them marked up, while every other box in the chain is skipped.
fn box_text(sample: &[u8]) -> Option<String> {
    let mut found: Vec<&[u8]> = Vec::new();
    let mut at = 0;
    while at + 8 <= sample.len() {
        let size = u32::from_be_bytes(sample[at..at + 4].try_into().ok()?) as usize;
        if size < 8 || at + size > sample.len() {
            return None;
        }
        let kind: &[u8; 4] = sample[at + 4..at + 8].try_into().ok()?;
        if kind == b"text" || kind == b"html" {
            let payload = &sample[at + 8..at + size];
            // A box ends at its first NUL; what follows it is a second string
            // this player has no use for.
            let cut = payload
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(payload.len());
            found.push(&payload[..cut]);
        }
        at += size;
    }
    if found.is_empty() {
        return None;
    }
    let joined = found
        .iter()
        .map(|bytes| subtitles::plain(&String::from_utf8_lossy(bytes)))
        .filter(|text| !text.is_empty())
        .collect::<Vec<String>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// Lossy UTF-8 with the markup of a sidecar stripped, dropped when nothing is
/// left to show.
fn plain(bytes: &[u8]) -> Option<String> {
    let text = subtitles::plain(&String::from_utf8_lossy(bytes));
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::{Mp4SubtitleReader, sample_text};
    use crate::container::mp4::Limits;
    use std::io::Cursor;

    /// `tests/fixtures/subtitles/mov-text.mp4`, made with the command written
    /// out in `tests/mp4.rs`: twelve AVC frames, one AAC track and a
    /// `ffmpeg -c:s mov_text` subtitle track of two lines.
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/subtitles/mov-text.mp4");

    fn fixture() -> Mp4SubtitleReader<Cursor<&'static [u8]>> {
        Mp4SubtitleReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a text track")
    }

    fn lines(cues: &[super::Cue]) -> Vec<(&str, u64, u64)> {
        cues.iter()
            .map(|cue| {
                (
                    cue.text.as_str(),
                    cue.start.as_millis() as u64,
                    cue.end.as_millis() as u64,
                )
            })
            .collect()
    }

    #[test]
    fn a_mov_text_track_is_listed_and_read() {
        let mut reader = fixture();
        let tracks = reader.tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].handler, *b"sbtl");
        assert_eq!(tracks[0].label(), "mov_text");
        // The track holds five samples, three of them the empty ones ffmpeg
        // writes at the cut points; the two lines keep the lengths the sample
        // table states for them.
        assert_eq!(
            lines(&reader.cues(0).expect("the only track")),
            [("first line", 100, 300), ("second line", 400, 500)]
        );
        let error = reader.cues(1).expect_err("only one track");
        assert!(error.to_string().contains("subtitle track"));
    }

    /// `tests/fixtures/subtitles/mov-text-tracks.mp4`, made with:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=10:duration=0.3 \
    ///        -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.3 -ac 1 \
    ///        -i one.srt -i two.srt -map 0:v -map 1:a -map 2:0 -map 3:0 \
    ///        -c:v libx264 -pix_fmt yuv420p -preset ultrafast -c:a aac \
    ///        -c:s mov_text tests/fixtures/subtitles/mov-text-tracks.mp4
    /// ```
    ///
    /// `one.srt` holds the two `plain` lines at 100 ms and 400 ms and `two.srt`
    /// the other two at 200 ms and 420 ms, the last of them written as
    /// `<i>tagged words</i>`: the encoder moves that markup into a style box
    /// beside the line, so the cue is the words on their own.
    #[test]
    fn both_mov_text_tracks_are_listed_and_read_apart() {
        const BOTH: &[u8] = include_bytes!("../tests/fixtures/subtitles/mov-text-tracks.mp4");
        let mut reader = Mp4SubtitleReader::open(Cursor::new(BOTH), Limits::default())
            .expect("fixture has two text tracks");
        let tracks = reader.tracks();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|track| track.label() == "mov_text"));
        assert_eq!(
            lines(&reader.cues(0).expect("first track")),
            [("first line", 100, 300), ("second line", 400, 500)]
        );
        assert_eq!(
            lines(&reader.cues(1).expect("second track")),
            [("other track", 200, 350), ("tagged words", 420, 480)]
        );
    }

    #[test]
    fn a_length_stated_sample_gives_its_string() {
        assert_eq!(
            sample_text(b"\x00\x0bsecond line").as_deref(),
            Some("second line")
        );
        // The two zero bytes of an empty line leave nothing to show.
        assert_eq!(sample_text(b"\x00\x00"), None);
        // A length past the end of the sample is refused, not filled with
        // whatever the next sample holds.
        assert_eq!(sample_text(b"\x00\x09abc"), None);
        // Under two bytes there is no length, and nothing to read either.
        assert_eq!(sample_text(b""), None);
    }

    #[test]
    fn a_group_of_display_parameters_states_its_own_text() {
        // A 3GPP timed text group: a 19-byte `styl` box, then the line.
        let mut styl = Vec::new();
        styl.extend_from_slice(&19u32.to_be_bytes());
        styl.extend_from_slice(b"styl");
        styl.extend_from_slice(&[0, 20, 0, 20, 0xff, 0xff, 0xff, 0xff, 0]);
        let mut group = (styl.len() as u16).to_be_bytes().to_vec();
        group.extend_from_slice(&styl);
        group.extend_from_slice(&(b"styled line".len() as u16).to_be_bytes());
        group.extend_from_slice(b"styled line");
        assert_eq!(
            sample_text(&group).as_deref(),
            Some("styled line"),
            "the style run is skipped and its bytes are not shown"
        );
    }

    #[test]
    fn a_chain_of_boxes_keeps_only_its_text() {
        let mut chain = Vec::new();
        for (kind, payload) in [
            (b"styl", &[0u8, 1, 2, 3][..]),
            (b"text", &b"from a box\0Arial"[..]),
            (b"html", &b"<i>markup</i> drops"[..]),
        ] {
            chain.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
            chain.extend_from_slice(kind);
            chain.extend_from_slice(payload);
        }
        // The sample opens with a box whose 32-bit size starts with two zero
        // bytes, which is where a length-stated sample has its length.
        assert_eq!(
            sample_text(&chain).as_deref(),
            Some("from a box\nmarkup drops")
        );
        // A chain with no text box in it is not a line.
        assert_eq!(sample_text(&chain[..12]), None);
        // A chain whose second box overruns the sample gives no line either.
        let mut broken = chain[..12].to_vec();
        broken.extend_from_slice(&0x00ff_ffffu32.to_be_bytes());
        broken.extend_from_slice(b"text");
        assert_eq!(sample_text(&broken), None);
    }

    /// A file that titles its subtitle track puts that title in the list the
    /// player walks, and one that states nothing is still known by its format.
    /// The same `tests/fixtures/tracks/named.mp4` the audio and container tests
    /// read, whose command `tests/mp4.rs` records.
    #[test]
    fn a_titled_subtitle_track_is_listed_by_its_title() {
        const NAMED: &[u8] = include_bytes!("../tests/fixtures/tracks/named.mp4");
        let reader =
            Mp4SubtitleReader::open(Cursor::new(NAMED), Limits::default()).expect("tracks");
        let labels: Vec<String> = reader
            .tracks()
            .iter()
            .map(super::Mp4SubtitleTrack::label)
            .collect();
        assert_eq!(labels, ["Титры", "mov_text"]);
    }
}

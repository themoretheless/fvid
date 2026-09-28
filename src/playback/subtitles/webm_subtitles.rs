//! Matroska text subtitle tracks.
//!
//! Reads the blocks of one subtitle track and turns them into the cue list the
//! player paints, so a file with embedded text subtitles needs no sidecar. Both
//! text codecs the container spells out are read: plain text, whose timing only
//! the container knows, and the ASS-family ones whose block may name a range of
//! its own and may reach the player with the fields of that range kept, dropped
//! or zeroed out — `subtitles::ass_block` settles which of the three a block
//! is. Styles, positions and colours of a script are not applied: the line is
//! laid out as any other text.

use crate::container::webm::{Limits, WebmReader};
use crate::subtitles::{self, Cue};
use crate::{Result, invalid};
use std::io::{Read, Seek};
use std::time::Duration;

/// Matroska `TrackType` for a subtitle track.
const SUBTITLE_KIND: u64 = 17;

/// The codec IDs whose blocks are text this reader can lay out.
const TEXT_CODECS: &[&str] = &["S_TEXT/UTF8", "S_TEXT/ASS", "S_TEXT/SSA"];

/// Of those, the IDs stored as the script lines of the ASS family rather than
/// as bare text, so only their blocks are read field by field.
const SCRIPT_CODECS: &[&str] = &["S_TEXT/ASS", "S_TEXT/SSA"];

/// One embedded subtitle track, described by what the container carries.
pub struct SubtitleTrack {
    /// `Name`, empty when the file names no track.
    pub name: String,
    /// The language the track states, empty when it states none.
    pub language: String,
    /// Codec ID, e.g. `S_TEXT/UTF8`.
    pub codec: String,
}

impl SubtitleTrack {
    /// Name for the on-screen track list: the track's own name, or else the
    /// language it states, or else the text format it is stored in.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        if !self.language.is_empty() {
            return self.language.clone();
        }
        match self.codec.as_str() {
            "S_TEXT/UTF8" => "UTF-8".to_owned(),
            "S_TEXT/ASS" => "ASS".to_owned(),
            "S_TEXT/SSA" => "SSA".to_owned(),
            other => other.to_owned(),
        }
    }
}

/// Matroska reader over the text subtitle tracks it can lay out.
pub struct WebmSubtitleReader<R> {
    demuxer: WebmReader<R>,
}

impl<R: Read + Seek> WebmSubtitleReader<R> {
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        Ok(Self {
            demuxer: WebmReader::open(reader, limits)?,
        })
    }

    /// Track numbers of the readable subtitle tracks, in container order.
    fn supported(&self) -> Vec<u64> {
        self.demuxer
            .tracks
            .iter()
            .filter(|track| {
                track.kind == SUBTITLE_KIND && TEXT_CODECS.contains(&track.codec.as_str())
            })
            .map(|track| track.number)
            .collect()
    }

    /// The readable tracks, in container order.
    pub fn tracks(&self) -> Vec<SubtitleTrack> {
        self.supported()
            .iter()
            .map(|number| {
                let track = self
                    .demuxer
                    .tracks
                    .iter()
                    .find(|track| track.number == *number)
                    .expect("supported lists existing tracks");
                SubtitleTrack {
                    name: track.name.clone(),
                    language: track.language.clone(),
                    codec: track.codec.clone(),
                }
            })
            .collect()
    }

    /// Cues of the `nth` readable track, counting in container order.
    pub fn cues(&mut self, nth: usize) -> Result<Vec<Cue>> {
        // A subtitle file is read as a whole, so the list wants every block in
        // it rather than the clusters walked so far.
        self.demuxer.scan_all()?;
        let number = *self
            .supported()
            .get(nth)
            .ok_or_else(|| invalid("WebM has no such subtitle track"))?;
        let track = self
            .demuxer
            .tracks
            .iter()
            .find(|track| track.number == number)
            .expect("supported lists existing tracks");
        let default = Duration::from_nanos(track.default_duration_ns);
        let script = SCRIPT_CODECS.contains(&track.codec.as_str());
        let mut blocks: Vec<(i64, usize)> = self
            .demuxer
            .packets
            .iter()
            .enumerate()
            .filter(|(_, packet)| packet.track == number)
            .map(|(index, packet)| (packet.pts_ns, index))
            .collect();
        // Subtitle blocks ride in the same clusters as the picture, so they
        // arrive in cluster order rather than in order of their own track.
        blocks.sort_by_key(|(pts, _)| *pts);
        let mut cues = Vec::with_capacity(blocks.len());
        for (position, (pts, index)) in blocks.iter().enumerate() {
            let block = self.demuxer.read_packet(*index)?;
            let block = String::from_utf8_lossy(&block);
            let (stated, text) = if script {
                subtitles::ass_block(&block)
            } else {
                (None, subtitles::plain(block.trim()))
            };
            if text.is_empty() {
                continue;
            }
            let container = Duration::from_nanos(u64::try_from(*pts).unwrap_or_default());
            let next = blocks
                .get(position + 1)
                .map(|(pts, _)| Duration::from_nanos(u64::try_from(*pts).unwrap_or_default()));
            let (start, end) = match stated {
                Some(range) => range,
                None => (container, subtitles::cue_end(container, default, next)),
            };
            cues.push(Cue { start, end, text });
        }
        Ok(cues)
    }
}

#[cfg(test)]
mod tests {
    use super::{SubtitleTrack, WebmSubtitleReader};
    use crate::container::webm::Limits;
    use crate::subtitles::LAST_LINE;
    use std::io::Cursor;

    /// `tests/fixtures/subtitles/text-tracks.mkv`, made with:
    /// ffmpeg -f lavfi -i testsrc=duration=2:size=128x72:rate=10 \
    ///   -i one.srt -i two.srt -map 0:v -map 1:0 -map 2:0 \
    ///   -c:v libvpx-vp9 -pix_fmt yuv420p -c:s srt \
    ///   tests/fixtures/subtitles/text-tracks.mkv
    /// `one.srt` holds the two `plain` cues at 200 ms and 1.1 s, `two.srt` the
    /// other two at 500 ms and 1.2 s.
    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/subtitles/text-tracks.mkv");

    fn fixture() -> WebmSubtitleReader<Cursor<&'static [u8]>> {
        WebmSubtitleReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has subtitle tracks")
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
    fn both_text_tracks_are_listed_and_read_apart() {
        let mut reader = fixture();
        let tracks = reader.tracks();
        assert_eq!(tracks.len(), 2);
        // The muxer wrote no `TrackName`, so both are known by their format.
        assert!(tracks.iter().all(|track| track.label() == "UTF-8"));
        // The tracks share clusters, so each one picks out and sorts its own
        // blocks: 200 ms and 1.1 s here, not the file's 200/500/1.1/1.2.
        assert_eq!(
            lines(&reader.cues(0).expect("first track")),
            [
                ("plain first", 200, 1_100),
                ("plain second", 1_100, 1_100 + LAST_LINE.as_millis() as u64),
            ]
        );
        assert_eq!(
            lines(&reader.cues(1).expect("second track")),
            [
                ("other track", 500, 1_200),
                // Markup inside a block is stripped like it is in a sidecar.
                ("tagged words", 1_200, 1_200 + LAST_LINE.as_millis() as u64),
            ]
        );
        let error = reader.cues(2).expect_err("only two tracks");
        assert!(error.to_string().contains("subtitle track"));
    }

    /// A Matroska file written by hand, because ffmpeg names its tracks through
    /// a tag element rather than `TrackName` and leaves `DefaultDuration` out.
    fn size(value: usize) -> Vec<u8> {
        let digits = (1..=8usize)
            .find(|&digits| value < (1usize << (7 * digits)))
            .unwrap_or(8);
        let mut out = Vec::with_capacity(digits);
        for (position, shift) in (0..digits).rev().enumerate() {
            let byte = ((value >> (8 * shift)) & 0xff) as u8;
            out.push(if position == 0 {
                byte | (0x80 >> (digits - 1))
            } else {
                byte
            });
        }
        out
    }

    fn atom(id: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend(size(payload.len()));
        out.extend(payload);
        out
    }

    /// An EBML unsigned element: the value in the fewest big-endian bytes. Only
    /// block track numbers and element sizes are variable integers.
    fn uint(value: u64) -> Vec<u8> {
        let bytes = value.to_be_bytes();
        let first = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len() - 1);
        bytes[first..].to_vec()
    }

    fn track_entry(number: u64, kind: u64, codec: &str, name: &str, default_ns: u64) -> Vec<u8> {
        let mut fields = atom(&[0xd7], &uint(number));
        fields.extend(atom(&[0x83], &uint(kind)));
        fields.extend(atom(&[0x86], codec.as_bytes()));
        if !name.is_empty() {
            fields.extend(atom(&[0x53, 0x6e], name.as_bytes()));
        }
        if default_ns > 0 {
            fields.extend(atom(&[0x23, 0xe3, 0x83], &uint(default_ns)));
        }
        atom(&[0xae], &fields)
    }

    fn written(tracks: &[Vec<u8>], blocks: &[(u64, i16, &str)]) -> Vec<u8> {
        let mut entries = Vec::new();
        for entry in tracks {
            entries.extend(entry.iter().copied());
        }
        let mut cluster = atom(&[0xe7], &uint(0));
        for (number, offset, text) in blocks {
            let mut block = size(*number as usize);
            block.extend((*offset as u16).to_be_bytes());
            // A SimpleBlock carries its flags in the byte after the timestamp.
            block.push(0x80);
            block.extend(text.as_bytes());
            cluster.extend(atom(&[0xa3], &block));
        }
        let mut segment = atom(&[0x16, 0x54, 0xae, 0x6b], &entries);
        segment.extend(atom(&[0x1f, 0x43, 0xb6, 0x75], &cluster));
        let mut out = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"matroska"));
        out.extend(atom(&[0x18, 0x53, 0x80, 0x67], &segment));
        out
    }

    fn built(file: &[u8]) -> WebmSubtitleReader<Cursor<&[u8]>> {
        WebmSubtitleReader::open(Cursor::new(file), Limits::default())
            .expect("hand-written Matroska")
    }

    #[test]
    fn a_track_name_beats_the_format_and_a_picture_track_is_left_out() {
        let file = written(
            &[
                track_entry(1, 1, "V_VP9", "", 0),
                track_entry(2, 17, "S_TEXT/UTF8", "Comments", 0),
            ],
            &[(2, 500, "named first"), (2, 1_200, "named second")],
        );
        let mut reader = built(&file);
        let tracks = reader.tracks();
        assert_eq!(tracks.len(), 1, "a picture track is not a subtitle track");
        assert_eq!(tracks[0].codec, "S_TEXT/UTF8");
        assert_eq!(tracks[0].label(), "Comments");
        assert_eq!(
            lines(&reader.cues(0).expect("the only track")),
            [
                ("named first", 500, 1_200),
                ("named second", 1_200, 1_200 + LAST_LINE.as_millis() as u64),
            ]
        );
    }

    #[test]
    fn a_default_duration_ends_the_plain_text_blocks() {
        let file = written(
            &[track_entry(3, 17, "S_TEXT/UTF8", "", 1_500_000_000)],
            &[(3, 300, "three hundred"), (3, 2_000, "two seconds")],
        );
        let mut reader = built(&file);
        assert_eq!(
            lines(&reader.cues(0).expect("the only track")),
            [
                // 300 ms plus the promised 1.5 s, not the 1.7 s gap to the next.
                ("three hundred", 300, 1_800),
                ("two seconds", 2_000, 3_500),
            ]
        );
    }

    #[test]
    fn a_track_is_named_by_its_title_then_its_language_then_its_format() {
        assert_eq!(
            SubtitleTrack {
                name: String::new(),
                language: String::new(),
                codec: "S_TEXT/UTF8".to_owned(),
            }
            .label(),
            "UTF-8"
        );
        assert_eq!(
            SubtitleTrack {
                name: String::new(),
                language: "por".to_owned(),
                codec: "S_TEXT/UTF8".to_owned(),
            }
            .label(),
            "por"
        );
        assert_eq!(
            SubtitleTrack {
                name: "Spanish".to_owned(),
                language: "spa".to_owned(),
                codec: "S_TEXT/UTF8".to_owned(),
            }
            .label(),
            "Spanish"
        );
        assert_eq!(
            SubtitleTrack {
                name: String::new(),
                language: String::new(),
                codec: "S_TEXT/ASS".to_owned(),
            }
            .label(),
            "ASS"
        );
        assert_eq!(
            SubtitleTrack {
                name: String::new(),
                language: String::new(),
                codec: "S_TEXT/SSA".to_owned(),
            }
            .label(),
            "SSA"
        );
    }

    /// The older ID of the same family of scripts. No muxer here writes it — a
    /// file `ffmpeg` makes from an SSA script carries `S_TEXT/ASS`, which is how
    /// its blocks were measured — so the track is written out here the way the
    /// container spells it.
    #[test]
    fn an_ssa_track_is_listed_and_read_by_its_script_lines() {
        let laid_out = "Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Old layout";
        let shorter = "Dialogue: 0,0:00:04.00,0:00:05.00,Default,,Six fields";
        let untimed = "Dialogue: 0,0:00:00.00,0:00:00.00,Default,,0,0,0,,Untimed";
        let file = written(
            &[
                track_entry(1, 1, "V_VP9", "", 0),
                track_entry(2, 17, "S_TEXT/SSA", "", 0),
                track_entry(3, 17, "S_TEXT/SSA", "Старые", 0),
            ],
            &[(2, 500, laid_out), (2, 3_000, shorter), (3, 1_000, untimed)],
        );
        let mut reader = built(&file);
        let labels: Vec<String> = reader.tracks().iter().map(SubtitleTrack::label).collect();
        assert_eq!(labels, ["SSA", "Старые"]);
        assert_eq!(
            lines(&reader.cues(0).expect("the untitled SSA track")),
            [("Old layout", 1_000, 3_000)]
        );
        // The line of a shorter field list leaves no cue, and its successor
        // still starts on the time of the block the file gives it.
        assert_eq!(
            lines(&reader.cues(1).expect("the titled SSA track")),
            [("Untimed", 1_000, 1_000 + LAST_LINE.as_millis() as u64)]
        );
    }

    /// `tests/fixtures/subtitles/ass-track.mkv`, made with:
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=25:duration=3 \
    ///   -i one.ass -i one.srt -map 0:v -map 1:0 -map 2:0 \
    ///   -c:v libvpx-vp9 -pix_fmt yuv420p -b:v 40k \
    ///   -c:s:0 ass -c:s:1 srt \
    ///   -metadata:s:s:0 title=События -metadata:s:s:0 language=rus \
    ///   tests/fixtures/subtitles/ass-track.mkv
    /// `one.ass` holds the two lines read here and a `Comment:` line, which the
    /// muxer leaves out of the file, and its clocks never reach the blocks:
    /// `ffmpeg` writes the fields with the two timing ones taken out, which is
    /// why the first line ends where the next block starts.
    #[test]
    fn an_ass_track_is_listed_by_its_title_and_read_from_its_blocks() {
        const ASS: &[u8] = include_bytes!("../../../tests/fixtures/subtitles/ass-track.mkv");
        let mut reader = WebmSubtitleReader::open(Cursor::new(ASS), Limits::default())
            .expect("fixture has an ASS track");
        let labels: Vec<String> = reader.tracks().iter().map(SubtitleTrack::label).collect();
        assert_eq!(labels, ["События", "UTF-8"]);
        assert_eq!(
            lines(&reader.cues(0).expect("the ASS track")),
            [
                ("First\nsecond line", 500, 2_000),
                (
                    "Hello, world with commas",
                    2_000,
                    2_000 + LAST_LINE.as_millis() as u64
                ),
            ]
        );
    }

    /// The other two shapes a block reaches the reader in, which no muxer here
    /// writes: the whole ten-field `Dialogue:` line the Matroska documentation
    /// shows, and the `Comment:` line of the same script.
    #[test]
    fn an_ass_block_that_names_its_own_range_is_timed_by_the_script() {
        let file = written(
            &[track_entry(2, 17, "S_TEXT/ASS", "", 0)],
            &[
                (
                    2,
                    500,
                    "Dialogue: 0,0:00:04.00,0:00:06.00,Default,,0,0,0,,{\\i1}Own time",
                ),
                (
                    2,
                    700,
                    "Comment: 0,0:00:07.00,0:00:08.00,Default,,0,0,0,,hidden",
                ),
                (
                    2,
                    900,
                    "Dialogue: 0,0:00:00.00,0:00:00.00,Default,,0,0,0,,File time",
                ),
            ],
        );
        let mut reader = built(&file);
        assert_eq!(
            lines(&reader.cues(0).expect("the ASS track")),
            [
                // Out of the block's own 500 ms, because the line states a range.
                ("Own time", 4_000, 6_000),
                // Clocks zeroed out are the writer asking the file to decide,
                // and the last line of the track gets the fallback length.
                ("File time", 900, 900 + LAST_LINE.as_millis() as u64),
            ]
        );
    }

    /// The same two subtitle tracks out of a Matroska file, where the title
    /// rides in the element the reader now knows by its spec ID.
    /// `tests/fixtures/tracks/named.mkv`, whose command the container's test
    /// records.
    #[test]
    fn a_titled_subtitle_track_is_listed_by_its_title() {
        const NAMED: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mkv");
        let reader = WebmSubtitleReader::open(Cursor::new(NAMED), Limits::default())
            .expect("fixture has subtitle tracks");
        let labels: Vec<String> = reader.tracks().iter().map(SubtitleTrack::label).collect();
        assert_eq!(labels, ["Титры", "UTF-8"]);
    }
}

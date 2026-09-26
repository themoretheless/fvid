//! WebM audio packet extraction.
//!
//! Extracts encoded audio packets from WebM containers for decoding.

use crate::audio::{AudioStream, EncodedPacket};
use crate::container::webm::{Limits, Track, WebmReader};
use crate::{Result, invalid, unsupported};
use std::io::{Read, Seek};

/// Timestamps in a Matroska segment are nanoseconds.
const TIMESCALE_NS: u32 = 1_000_000_000;

/// Codec IDs the player has a decoder for, as Matroska spells them. The three
/// `A_PCM/*` IDs are uncompressed audio, whose width the track states in
/// `BitDepth`.
const CODECS: [&str; 9] = [
    "A_VORBIS",
    "A_MPEG/L3",
    "A_FLAC",
    "A_ALAC",
    "A_AC3",
    "A_PCM/INT/LIT",
    "A_PCM/INT/BIG",
    "A_PCM/FLOAT/IEEE",
    "A_EAC3",
];

/// The codec setup bytes a decoder needs for a track. Vorbis keeps its header
/// packets in `CodecPrivate`; FLAC keeps its metadata blocks, from which the
/// decoder reads only the STREAMINFO body; Apple Lossless keeps its magic cookie,
/// which is the whole of its geometry; MP3 frames describe themselves and want
/// nothing.
fn setup_data(track: &Track) -> Result<Vec<u8>> {
    match track.codec.as_str() {
        "A_VORBIS" => vorbis_setup_headers(&track.codec_private),
        "A_FLAC" => Ok(flac_stream_info(&track.codec_private)),
        "A_ALAC" => Ok(track.codec_private.clone()),
        _ => Ok(Vec::new()),
    }
}

/// The STREAMINFO body of a FLAC setup block. Matroska stores the metadata
/// blocks in `CodecPrivate`, sometimes with the `fLaC` marker some writers keep
/// in front; a block is a flag byte, a 24-bit length and its body, and the
/// decoder reads only the 34-byte STREAMINFO body, so its header goes too.
fn flac_stream_info(private: &[u8]) -> Vec<u8> {
    let blocks = private.strip_prefix(b"fLaC").unwrap_or(private);
    if blocks.len() >= 38 && blocks[0] & 0x7f == 0 && blocks[1..4] == [0, 0, 34] {
        return blocks[4..].to_vec();
    }
    blocks.to_vec()
}

/// Audio packet with presentation timestamp.
pub struct AudioPacket {
    /// Encoded audio data (Vorbis frame).
    pub data: Vec<u8>,
    /// Presentation timestamp in nanoseconds.
    pub pts_ns: i64,
    /// Track number this packet belongs to.
    pub track: u64,
}

/// WebM audio track reader. Extracts encoded packets for decoding.
pub struct WebmAudioReader<R> {
    demuxer: WebmReader<R>,
    track_number: u64,
    packet_index: usize,
    /// Vorbis setup headers in the layout the decoder expects.
    extra_data: Vec<u8>,
}

impl<R: Read + Seek> WebmAudioReader<R> {
    /// Open the first audio track this player can decode.
    pub fn open(reader: R, limits: Limits) -> Result<Self> {
        Self::open_at(reader, limits, 0)
    }

    /// Open the `nth` audio track this player can decode, counting in container
    /// order. Each one carries its own Vorbis setup headers, so the choice is
    /// made before the track is validated rather than after.
    pub fn open_at(reader: R, limits: Limits, nth: usize) -> Result<Self> {
        let demuxer = WebmReader::open(reader, limits)?;
        let number = match Self::supported(&demuxer).get(nth).copied() {
            Some(number) => number,
            // As in the MP4 reader: a listed audio track states its own coding, so
            // naming it separates a missing decoder arm from a container this reader
            // never got through.
            None => match demuxer.tracks.iter().find(|t| t.kind == 2) {
                Some(track) => {
                    return Err(unsupported(&format!(
                        "WebM audio track coded `{}` has no decoder here",
                        track.codec
                    )));
                }
                None => return Err(invalid("WebM has no such audio track")),
            },
        };
        Self::from_track(demuxer, number)
    }

    /// Track numbers of the audio tracks a decoder exists for, in container order.
    fn supported(demuxer: &WebmReader<R>) -> Vec<u64> {
        demuxer
            .tracks
            .iter()
            .filter(|t| t.kind == 2 && CODECS.contains(&t.codec.as_str()))
            .map(|t| t.number)
            .collect()
    }

    /// Open a track the demuxer already listed.
    fn from_track(demuxer: WebmReader<R>, track_number: u64) -> Result<Self> {
        let track = demuxer
            .tracks
            .iter()
            .find(|t| t.number == track_number)
            .ok_or_else(|| invalid("WebM has no such audio track"))?;
        let extra_data = setup_data(track)?;
        if track.sample_rate == 0 || track.sample_rate > u64::from(u32::MAX) {
            return Err(invalid("WebM audio track has no usable sample rate"));
        }
        if track.channels == 0 || track.channels > u64::from(u16::MAX) {
            return Err(invalid("WebM audio track has no usable channel count"));
        }
        Ok(Self {
            demuxer,
            track_number,
            packet_index: 0,
            extra_data,
        })
    }

    /// Read the next audio packet. Returns None at end of track.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        while self.packet_index < self.demuxer.packets.len() {
            let idx = self.packet_index;
            let (pts_ns, track) = {
                let packet = &self.demuxer.packets[idx];
                (packet.pts_ns, packet.track)
            };
            self.packet_index += 1;

            if track != self.track_number {
                continue;
            }

            let data = self.demuxer.read_packet(idx)?;

            return Ok(Some(AudioPacket {
                data,
                pts_ns,
                track,
            }));
        }
        Ok(None)
    }

    /// The audio track metadata.
    pub fn track(&self) -> &Track {
        self.demuxer
            .tracks
            .iter()
            .find(|t| t.number == self.track_number)
            .expect("audio track must exist")
    }

    /// Restart from the first packet.
    pub fn rewind(&mut self) {
        self.packet_index = 0;
    }

    /// Seek to the packet with the greatest PTS at or before `pts_ns`.
    /// Returns that packet's PTS, or the first one when the whole track follows.
    pub fn seek(&mut self, pts_ns: i64) -> i64 {
        let mut best: Option<(usize, i64)> = None;
        for (i, packet) in self.demuxer.packets.iter().enumerate() {
            if packet.track != self.track_number || packet.pts_ns > pts_ns {
                continue;
            }
            if best.is_none_or(|(_, at)| packet.pts_ns >= at) {
                best = Some((i, packet.pts_ns));
            }
        }
        let (index, landed) = best.unwrap_or((0, 0));
        self.packet_index = index;
        landed
    }
}

/// Vorbis stores its three header packets in `CodecPrivate`, but a decoder only
/// needs the identification and setup ones. Writers disagree about whether
/// those packets carry Xiph lacing, and symphonia unpacks the laced form
/// itself, so only the raw concatenation is rewritten here.
fn vorbis_setup_headers(private: &[u8]) -> Result<Vec<u8>> {
    const IDENT_TYPE: u8 = 1;
    const COMMENT: &[u8; 7] = b"\x03vorbis";
    const SETUP: &[u8; 7] = b"\x05vorbis";

    if private.first() != Some(&IDENT_TYPE) {
        return Ok(private.to_vec());
    }
    let find = |marker: &[u8; 7], from: usize| {
        private[from..]
            .windows(marker.len())
            .position(|w| w == marker)
            .map(|at| at + from)
    };
    let comment = find(COMMENT, 1).ok_or_else(|| invalid("WebM Vorbis headers are truncated"))?;
    let setup =
        find(SETUP, comment).ok_or_else(|| invalid("WebM Vorbis headers have no setup header"))?;
    let mut headers = private[..comment].to_vec();
    headers.extend_from_slice(&private[setup..]);
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::{WebmAudioReader, flac_stream_info, vorbis_setup_headers};
    use crate::audio::{AudioStream, AudioTrack};
    use crate::container::webm::Limits;
    use std::io::Cursor;

    fn header(kind: u8, tail: &[u8]) -> Vec<u8> {
        let mut v = vec![kind, b'v', b'o', b'r', b'b', b'i', b's'];
        v.extend_from_slice(tail);
        v
    }

    /// The container lists the tracks a decoder exists for, so the player's
    /// track key knows when there is nothing to choose. A request past the end
    /// is an error rather than a silent fallback to the first track.
    #[test]
    fn the_fixture_lists_its_audio_tracks() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/vorbis-stereo.webm");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default()).unwrap();
        assert_eq!(
            stream.audio_tracks(),
            vec![AudioTrack {
                sample_rate: 44_100,
                channels: 2,
                name: String::new(),
                language: String::new(),
            }]
        );
        let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 0).unwrap();
        assert_eq!((chosen.sample_rate(), chosen.channels()), (44_100, 2));
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).is_err());
    }

    /// A file that says nothing about a track leaves the layout as its whole
    /// name; a title or a language is said first, and the layout follows it
    /// because two tracks can share a name and still differ in what they hold.
    #[test]
    fn a_track_is_named_by_what_the_file_says_and_what_it_holds() {
        let said = |name: &str, language: &str| {
            AudioTrack {
                sample_rate: 48_000,
                channels: 6,
                name: name.to_owned(),
                language: language.to_owned(),
            }
            .label()
        };
        assert_eq!(said("", ""), "6 ch 48000 Hz");
        assert_eq!(said("", "eng"), "eng · 6 ch 48000 Hz");
        assert_eq!(said("Surround", ""), "Surround · 6 ch 48000 Hz");
        assert_eq!(said("Surround", "eng"), "Surround · 6 ch 48000 Hz");
    }

    /// The same list out of a Matroska file: the muxer wrote the title into the
    /// `Name` element and the language into the one beside it, and the reader
    /// reaches both. `tests/fixtures/tracks/named.mkv`, whose command the
    /// container's own test records.
    #[test]
    fn a_named_file_lists_its_audio_tracks_by_what_it_says() {
        const NAMED: &[u8] = include_bytes!("../tests/fixtures/tracks/named.mkv");
        let stream = WebmAudioReader::open(Cursor::new(NAMED), Limits::default()).unwrap();
        let labels: Vec<String> = stream
            .audio_tracks()
            .iter()
            .map(AudioTrack::label)
            .collect();
        assert_eq!(labels, ["Первая · 1 ch 48000 Hz", "fre · 1 ch 32000 Hz"]);
    }

    #[test]
    fn raw_headers_drop_the_comment_packet() {
        let mut private = header(1, b"id");
        private.extend_from_slice(&header(3, b"comment"));
        private.extend_from_slice(&header(5, b"setup"));
        let mut expected = header(1, b"id");
        expected.extend_from_slice(&header(5, b"setup"));
        assert_eq!(vorbis_setup_headers(&private).unwrap(), expected);
    }

    #[test]
    fn laced_headers_pass_through_untouched() {
        let private = vec![2, 3, 7, 1, b'v', b'o', b'r', b'b', b'i', b's'];
        assert_eq!(vorbis_setup_headers(&private).unwrap(), private);
    }

    #[test]
    fn missing_setup_header_is_an_error() {
        let private = header(1, b"id");
        assert!(vorbis_setup_headers(&private).is_err());
    }

    /// The whole WebM audio path over a real file: demux, rebuild the setup
    /// headers, decode. The fixture is 2 s of 440 Hz sine recorded at −20.9 dBFS,
    /// which is what ffmpeg's own `volumedetect` reports for it, so the decoded
    /// peak is checked against that rather than against full scale.
    #[test]
    fn vorbis_fixture_decodes_to_pcm() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/vorbis-stereo.webm");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a decodable audio track");
        assert_eq!(stream.codec(), "A_VORBIS");
        assert_eq!(stream.timescale(), 1_000_000_000);
        assert_eq!(stream.sample_rate(), 44_100);
        assert_eq!(stream.channels(), 2);
        let extra = stream.extra_data();
        // ffmpeg stores the headers laced, so the first byte is a lace count and
        // the decoder unpacks them; only the bare concatenation gets rewritten.
        assert_eq!(extra, stream.track().codec_private);
        for marker in [b"\x01vorbis" as &[u8], b"\x03vorbis", b"\x05vorbis"] {
            assert!(extra.windows(7).any(|w| w == marker));
        }

        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("Vorbis decoder");
        let mut frames = 0usize;
        let mut peak = 0.0f32;
        let bytes = bytes_per_frame(stream.channels());
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 44_100));
            assert_eq!(pcm.data.len() % bytes, 0);
            frames += pcm.data.len() / bytes;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        // The 88 packets' block sizes sum to exactly 2.02 s. Total rather than a
        // range: a decoder that swallowed or doubled one packet would still land
        // inside any plausible band.
        assert_eq!(frames, 89_088);
        // 10^(-20.9/20) = 0.090, to within the encoder's own rounding.
        assert!(
            (0.088..=0.092).contains(&peak),
            "peak={peak} against ffmpeg's -20.9 dBFS"
        );

        // The same packets again, from the cursor's own rewind.
        stream.rewind();
        decoder.reset();
        let mut again = 0usize;
        while let Some(packet) = stream.next_packet().expect("packet") {
            if let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            {
                again += pcm.data.len() / bytes;
            }
        }
        assert_eq!(again, frames);
    }

    /// Decode every packet of a fixture and report how much sound came out.
    fn decode_all(stream: &mut WebmAudioReader<Cursor<&'static [u8]>>) -> (usize, f32) {
        let mut decoder = crate::codec::make_audio_decoder(
            stream.codec(),
            stream.extra_data(),
            stream.sample_rate(),
            stream.channels(),
            stream.bits_per_sample(),
        )
        .expect("decoder");
        let rate = stream.sample_rate();
        let bytes = bytes_per_frame(stream.channels());
        let mut frames = 0usize;
        let mut peak = 0.0f32;
        while let Some(packet) = stream.next_packet().expect("packet") {
            let Some(pcm) = decoder
                .decode_encoded(&packet.data, packet.pts.max(0) as u64, 0)
                .expect("decode")
            else {
                continue;
            };
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, rate));
            assert_eq!(pcm.data.len() % bytes, 0);
            frames += pcm.data.len() / bytes;
            for sample in pcm.data.chunks_exact(size_of::<f32>()) {
                peak = peak.max(f32::from_le_bytes(sample.try_into().unwrap()).abs());
            }
        }
        (frames, peak)
    }

    /// The FLAC fixture is 2 s of a 0.3-amplitude stereo sine, made with:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=2:s=44100' \
    ///   -vn -c:a flac tests/fixtures/audio/flac-stereo.mkv
    /// The coding is lossless, so both the frame count and the peak are exact
    /// and match what ffmpeg's own decode of the same file reports.
    #[test]
    fn a_flac_track_decodes_to_its_exact_length() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/flac-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a FLAC track");
        assert_eq!(stream.codec(), "A_FLAC");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        // Of the 42 setup bytes the muxer stored, only the 34-byte STREAMINFO
        // body reaches the decoder.
        let private = stream.track().codec_private.clone();
        assert_eq!(stream.extra_data().len(), 34);
        assert_eq!(stream.extra_data(), flac_stream_info(&private));
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 88_200);
        assert!((0.2999..=0.3001).contains(&peak), "peak={peak}");
    }

    /// The MP3 fixture is the same sine at 128 kbit/s:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=2:s=44100' \
    ///   -vn -c:a libmp3lame -b:a 128k tests/fixtures/audio/mp3-stereo.mkv
    /// One frame is 1152 samples and the encoder puts its delay in front of the
    /// first packet, so the total is the packet count times 1152 rather than the
    /// nominal two seconds. The peak is a band: lossy coding decides it, and
    /// ffmpeg's own decode of this file peaks at 0.291.
    #[test]
    fn an_mp3_track_decodes_frame_by_frame() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/mp3-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an MP3 track");
        assert_eq!(stream.codec(), "A_MPEG/L3");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        assert!(stream.extra_data().is_empty(), "a frame says all of it");
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 78 * 1_152);
        assert!((0.28..=0.30).contains(&peak), "peak={peak}");
    }

    /// Three audio tracks, one of them AC-3, from one source:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=44100' \
    ///   -vn -map 0:a -c:a:0 ac3 -b:a:0 96k -map 0:a -c:a:1 flac \
    ///   -map 0:a -c:a:2 libmp3lame -b:a:2 96k tests/fixtures/audio/ac3-flac-mp3.mkv
    /// The list holds only the two the player can decode, in the order the file
    /// gives them.
    #[test]
    fn the_track_list_skips_a_codec_there_is_no_decoder_for() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/ac3-flac-mp3.mkv");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has audio tracks");
        assert_eq!(stream.audio_tracks().len(), 3);
        for (nth, codec) in [(0, "A_AC3"), (1, "A_FLAC"), (2, "A_MPEG/L3")] {
            let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), nth)
                .unwrap_or_else(|error| panic!("track {nth}: {error}"));
            assert_eq!(chosen.codec(), codec);
        }
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 2).is_err());
    }

    /// One quarter second of the 0.3-amplitude stereo sine, written as the lossless
    /// integer PCM Matroska calls `A_PCM/INT/LIT`:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=0.25:s=44100' \
    ///   -vn -c:a pcm_s24le tests/fixtures/audio/pcm-int.mkv
    /// ```
    /// Eleven blocks of 6144 bytes, so 11025 frames of three little-endian bytes
    /// per channel. `BitDepth` is the only place the width is stated, which is
    /// what a decoder has to trust here and cannot in an ISO BMFF fourcc.
    #[test]
    fn a_24_bit_pcm_track_decodes_block_by_block() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/pcm-int.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a PCM track");
        assert_eq!(stream.codec(), "A_PCM/INT/LIT");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        assert_eq!(stream.bits_per_sample(), 24);
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 11_025);
        // 2516582 of 2^23, the exact top of the sine as ffmpeg reads it.
        assert_eq!(peak, 2_516_582.0 / 8_388_608.0);
    }

    /// The same source and the same block layout in Matroska's float coding:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=0.25:s=44100' \
    ///   -vn -c:a pcm_f32le tests/fixtures/audio/pcm-float.mkv
    /// ```
    /// Float PCM is already the shape the pipeline carries, so this is the path
    /// where the decoder only has to read the width and hand the samples over.
    #[test]
    fn a_float_pcm_track_decodes_block_by_block() {
        const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/audio/pcm-float.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has a float PCM track");
        assert_eq!(stream.codec(), "A_PCM/FLOAT/IEEE");
        assert_eq!(stream.bits_per_sample(), 32);
        assert!(stream.extra_data().is_empty(), "PCM has no setup record");
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 11_025);
        assert!((0.2999..=0.3001).contains(&peak), "peak={peak}");
    }

    /// A listener with no picture takes the timeline total from the track, and
    /// Matroska states it one block at a time: the span of the track's own
    /// blocks plus one more interval. Block stamps sit on the cluster's grid, so
    /// the estimate lands within a block of the sound the fixture carries.
    #[test]
    fn the_blocks_state_the_track_length() {
        for (name, fixture, low, high) in [
            (
                "mp3",
                include_bytes!("../tests/fixtures/audio/mp3-stereo.mkv") as &[u8],
                2.02,
                2.05,
            ),
            (
                "flac",
                include_bytes!("../tests/fixtures/audio/flac-stereo.mkv") as &[u8],
                1.98,
                2.06,
            ),
            (
                "vorbis",
                include_bytes!("../tests/fixtures/audio/vorbis-stereo.webm") as &[u8],
                1.99,
                2.06,
            ),
            (
                "pcm",
                include_bytes!("../tests/fixtures/audio/pcm-int.mkv") as &[u8],
                0.24,
                0.26,
            ),
        ] {
            let stream = WebmAudioReader::open(Cursor::new(fixture), Limits::default())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let at = stream
                .duration()
                .unwrap_or_else(|| panic!("{name} states no length"));
            assert!(
                (low..=high).contains(&at.as_secs_f64()),
                "{name}: {at:?} outside {low}..={high}"
            );
        }
    }

    #[test]
    fn only_the_stream_info_body_of_a_flac_setup_block_is_read() {
        let mut private = b"fLaC".to_vec();
        // Last-block flag set, type 0 (STREAMINFO), a 34-byte body.
        private.extend_from_slice(&[0x80, 0, 0, 34]);
        private.extend_from_slice(&[7u8; 34]);
        assert_eq!(flac_stream_info(&private), vec![7u8; 34]);
        // A block that is not STREAMINFO, or one that lies about its length, is
        // handed to the decoder as it stands.
        let comment = vec![0x04, 0, 0, 10];
        assert_eq!(flac_stream_info(&comment), comment);
        assert_eq!(
            flac_stream_info(&[0, 0, 0, 9, 1, 2]),
            vec![0, 0, 0, 9, 1, 2]
        );
    }

    fn bytes_per_frame(channels: u16) -> usize {
        usize::from(channels) * size_of::<f32>()
    }
}

impl<R: Read + Seek + Send> AudioStream for WebmAudioReader<R> {
    fn codec(&self) -> &str {
        &self.track().codec
    }

    fn timescale(&self) -> u32 {
        TIMESCALE_NS
    }

    fn sample_rate(&self) -> u32 {
        self.track().sample_rate as u32
    }

    fn channels(&self) -> u16 {
        self.track().channels as u16
    }

    /// How long the track is, as its own blocks say: the span between the first
    /// and the last of them plus one more interval, which is `DefaultDuration`
    /// where the writer declared one. A lone block states nothing about its
    /// length, so the total stays unknown rather than guessed.
    fn duration(&self) -> Option<std::time::Duration> {
        let mut stamps = self
            .demuxer
            .packets
            .iter()
            .filter(|p| p.track == self.track_number)
            .map(|p| p.pts_ns);
        let first = stamps.next()?;
        let mut last = first;
        let mut gap = None;
        for pts in stamps {
            if pts > last {
                gap = Some(pts - last);
                last = pts;
            }
        }
        let tail = gap.or_else(|| {
            i64::try_from(self.track().default_duration_ns)
                .ok()
                .filter(|declared| *declared > 0)
        })?;
        let nanos = i128::from(last) - i128::from(first) + i128::from(tail);
        Some(std::time::Duration::from_nanos(u64::try_from(nanos).ok()?))
    }

    fn bits_per_sample(&self) -> u16 {
        // A width too large for the type cannot be a real sample depth, so it
        // reaches the decoder as one it will refuse rather than as the 0 that
        // means "undeclared" and stands for 16-bit.
        u16::try_from(self.track().bit_depth).unwrap_or(u16::MAX)
    }

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    fn audio_tracks(&self) -> Vec<crate::audio::AudioTrack> {
        WebmAudioReader::supported(&self.demuxer)
            .into_iter()
            .map(|number| {
                let track = self
                    .demuxer
                    .tracks
                    .iter()
                    .find(|t| t.number == number)
                    .expect("supported lists existing tracks");
                crate::audio::AudioTrack {
                    sample_rate: track.sample_rate as u32,
                    channels: track.channels as u16,
                    name: track.name.clone(),
                    language: track.language.clone(),
                }
            })
            .collect()
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        Ok(WebmAudioReader::read_packet(self)?.map(|p| EncodedPacket {
            data: p.data,
            pts: p.pts_ns,
            duration: 0,
        }))
    }

    fn rewind(&mut self) {
        WebmAudioReader::rewind(self);
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        WebmAudioReader::seek(self, pts)
    }
}

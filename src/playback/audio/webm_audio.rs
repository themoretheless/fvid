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
/// `BitDepth`. `A_AAC` is spelled `mp4a` by the decoder dispatch, whichever
/// container named it, and needs its setup block rebuilt (see `aac_setup`).
const CODECS: [&str; 10] = [
    "A_VORBIS",
    "A_MPEG/L3",
    "A_MPEG/L2",
    "A_FLAC",
    "A_ALAC",
    "A_AC3",
    "A_AAC",
    "A_PCM/INT/LIT",
    "A_PCM/INT/BIG",
    "A_PCM/FLOAT/IEEE",
];

/// The codec setup bytes a decoder needs for a track. Vorbis keeps its header
/// packets in `CodecPrivate`; FLAC keeps its metadata blocks, from which the
/// decoder reads only the STREAMINFO body; Apple Lossless keeps its magic cookie,
/// which is the whole of its geometry; both MPEG audio layers describe their own
/// frames and want nothing.
fn setup_data(track: &Track) -> Result<Vec<u8>> {
    match track.codec.as_str() {
        "A_VORBIS" => vorbis_setup_headers(&track.codec_private),
        "A_FLAC" => Ok(flac_stream_info(&track.codec_private)),
        "A_ALAC" => Ok(track.codec_private.clone()),
        "A_AAC" => aac_setup(&track.codec_private),
        _ => Ok(Vec::new()),
    }
}

/// Matroska states an AAC track's setup as the bare AudioSpecificConfig, while
/// the decoder here reads an MP4 `esds` box that holds the same bytes inside its
/// descriptors. A file written by a tool that knows only Matroska carries the
/// short form, so the wrapper is rebuilt around it; a track whose setup block is
/// too short to name a coding is refused by the reason the config parser gives.
fn aac_setup(private: &[u8]) -> Result<Vec<u8>> {
    let Some(esds) = crate::playback_aac::esds_for(private) else {
        return Err(unsupported(&format!(
            "AAC in WebM names its setup block {} bytes, and a config shorter than two states no coding",
            private.len()
        )));
    };
    // Let the decoder's own parser judge the rebuilt box, so a config this
    // reader wraps wrongly is refused here rather than two frames in.
    crate::codec::config::aac_specific_config(&esds)?;
    Ok(esds)
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
    presentation_floor: Option<i64>,
    /// The setup bytes the decoder asks for, in the layout it reads.
    extra_data: Vec<u8>,
    /// The name the decoder dispatch answers: the container's own tag for every
    /// coding but AAC, which dispatches as `mp4a` whichever container named it.
    codec_tag: String,
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
        let codec_tag = if track.codec == "A_AAC" {
            "mp4a".to_string()
        } else {
            track.codec.clone()
        };
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
            presentation_floor: None,
            extra_data,
            codec_tag,
        })
    }

    /// Read the next audio packet. Returns None at end of track.
    pub fn read_packet(&mut self) -> Result<Option<AudioPacket>> {
        // Blocks are indexed a cluster at a time, so running out of them asks
        // the file for more rather than meaning the track has ended.
        while self.packet_index < self.demuxer.packets.len() || self.demuxer.scan_more()? {
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
        self.presentation_floor = None;
    }

    /// Seek to the packet with the greatest PTS at or before `pts_ns`.
    /// Returns that packet's PTS, or the first one when the whole track follows.
    pub fn seek(&mut self, pts_ns: i64) -> i64 {
        // A jump needs the blocks behind its target, which an index that grew a
        // cluster at a time has not necessarily reached yet. The infallible
        // signature is the audio stream's own; a walk that fails leaves the
        // index short, and the next packet read reports the same failure where
        // the caller can still hear it.
        let _ = self.demuxer.scan_until(pts_ns);
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
    use super::{WebmAudioReader, aac_setup, flac_stream_info, vorbis_setup_headers};
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm");
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
        const NAMED: &[u8] = include_bytes!("../../../tests/fixtures/tracks/named.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/mp3-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an MP3 track");
        assert_eq!(stream.codec(), "A_MPEG/L3");
        assert_eq!((stream.sample_rate(), stream.channels()), (44_100, 2));
        assert!(stream.extra_data().is_empty(), "a frame says all of it");
        let (frames, peak) = decode_all(&mut stream);
        assert_eq!(frames, 78 * 1_152);
        assert!((0.28..=0.30).contains(&peak), "peak={peak}");
    }

    /// A track in a coding with no decoder is left out of the list entirely, so the
    /// keys the player is handed count only what it can actually play:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=0.25:s=44100' \
    ///   -vn -map 0:a -c:a:0 libopus -b:a:0 32k -map 0:a -c:a:1 flac \
    ///   tests/fixtures/audio/opus-flac.mkv
    /// Opus is the file's first track and absent from the list, so the one track
    /// there is to choose is numbered zero.
    #[test]
    fn the_track_list_skips_a_codec_there_is_no_decoder_for() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/opus-flac.mkv");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has audio tracks");
        assert_eq!(stream.audio_tracks().len(), 1, "the Opus track is not offered");
        let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 0)
            .unwrap_or_else(|error| panic!("track 0: {error}"));
        assert_eq!(chosen.codec(), "A_FLAC");
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 1).is_err());
    }

    /// Three audio tracks from one source, every one of them decodable:
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=44100' \
    ///   -vn -map 0:a -c:a:0 ac3 -b:a:0 96k -map 0:a -c:a:1 flac \
    ///   -map 0:a -c:a:2 libmp3lame -b:a:2 96k tests/fixtures/audio/ac3-flac-mp3.mkv
    /// The Dolby track used to be the one this list dropped. It is here now, in
    /// the order the file gives, and only a key past all three is an error.
    #[test]
    fn a_dolby_track_is_listed_with_the_lossy_ones_it_sits_beside() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/ac3-flac-mp3.mkv");
        let stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has audio tracks");
        assert_eq!(stream.audio_tracks().len(), 3);
        for (nth, codec) in [(0, "A_AC3"), (1, "A_FLAC"), (2, "A_MPEG/L3")] {
            let chosen = WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), nth)
                .unwrap_or_else(|error| panic!("track {nth}: {error}"));
            assert_eq!(chosen.codec(), codec);
        }
        assert!(WebmAudioReader::open_at(Cursor::new(FIXTURE), Limits::default(), 3).is_err());
    }

    /// One second of two sines coded as AAC and copied into Matroska, which is
    /// what a remux of an `.m4a` leaves behind:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(1100*PI*t):d=1:s=48000' \
    ///   -c:a aac -b:a 128k /tmp/aac-src.m4a
    /// ffmpeg -i /tmp/aac-src.m4a -vn -c copy tests/fixtures/audio/aac-stereo.mka
    /// ```
    /// The track names itself `A_AAC` and carries its setup as the bare
    /// AudioSpecificConfig — five bytes, which `ffprobe` reports as the file's
    /// `extradata_size`. The decoder asks for an MP4 `esds`, so the wrapper is
    /// rebuilt, and the dispatch answers this coding as `mp4a` whatever the
    /// container called it.
    #[test]
    fn an_aac_track_arrives_with_only_the_config_and_is_wrapped() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/aac-stereo.mka");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an AAC track");
        assert_eq!(stream.codec(), "mp4a");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        assert_eq!(
            crate::codec::config::aac_specific_config(stream.extra_data())
                .expect("a setup block the decoder's own parser accepts")
                .len(),
            5,
            "the whole config the container handed over, not a truncated copy"
        );
        let (frames, peak) = decode_all(&mut stream);
        // 48 kHz AAC blocks are 1024 samples; a second of sound is 47 of them and
        // the decoder hands over the priming frame the encoder put in front.
        assert!(
            (47 * 1024..49 * 1024).contains(&frames),
            "decoded {frames} frames of a one second track"
        );
        // The loudest sample this route produces is the one ffmpeg's own reading
        // of the same stream produces, to the last bit: 0.41567978. It is well
        // above the 0.3 the sine was written at, and that overshoot belongs to
        // the coding rather than to this decode, which is why the number is
        // checked against the reference instead of against the amplitude.
        assert!(
            (peak - 0.415_679_78).abs() < 5e-5,
            "peak={peak}, the reference reads 0.41567978"
        );
    }

    /// A setup block that cannot name a coding is refused with the length it
    /// states, so a file with an empty `CodecPrivate` does not reach the decoder
    /// and fail there instead.
    #[test]
    fn an_aac_track_without_a_usable_setup_block_is_refused_by_name() {
        let error = aac_setup(&[0x11]).expect_err("one byte states no coding");
        assert!(
            error.to_string().contains("1 bytes"),
            "the refusal quotes the setup length: {error}"
        );
        assert!(aac_setup(&[0x11, 0x90]).is_ok(), "two bytes is enough");
    }

    /// The coding DVB and broadcast rips keep in Matroska, one second of a 440 Hz
    /// sine in two channels:
    /// ```sh
    /// ffmpeg -f lavfi -i 'aevalsrc=0.3*sin(880*PI*t)|0.3*sin(880*PI*t):d=1:s=48000' \
    ///   -vn -c:a mp2 -b:a 256k tests/fixtures/audio/mp2-stereo.mkv
    /// ```
    /// MPEG Layer II has had a decoder arm here for a while, because a bare
    /// `.mp2` file names itself with this very tag; what it never had was a
    /// container route, so a track Matroska called `A_MPEG/L2` was dropped from
    /// the list before the dispatch was ever asked.
    #[test]
    fn a_layer_ii_track_reaches_the_arm_the_layer_iii_route_shares() {
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/mp2-stereo.mkv");
        let mut stream = WebmAudioReader::open(Cursor::new(FIXTURE), Limits::default())
            .expect("fixture has an MP2 track");
        assert_eq!(stream.codec(), "A_MPEG/L2");
        assert_eq!((stream.sample_rate(), stream.channels()), (48_000, 2));
        let (frames, peak) = decode_all(&mut stream);
        // Layer II blocks are 1152 samples at this rate, so a second is 41 of them
        // and change; the reference reads 47 903 frames after its own 481-sample
        // priming, and this route keeps the priming it is handed, so the count is
        // bounded rather than pinned.
        assert!(
            (41 * 1152..43 * 1152).contains(&frames),
            "decoded {frames} frames of a one second track"
        );
        // Both channels carry the same tone, and this is the number ffmpeg's
        // reading of the same stream gives.
        assert!(
            (peak - 0.301_391_6).abs() < 5e-5,
            "peak={peak}, the reference reads 0.3013916"
        );
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-int.mkv");
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
        const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/audio/pcm-float.mkv");
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
                include_bytes!("../../../tests/fixtures/audio/mp3-stereo.mkv") as &[u8],
                2.02,
                2.05,
            ),
            (
                "flac",
                include_bytes!("../../../tests/fixtures/audio/flac-stereo.mkv") as &[u8],
                1.98,
                2.06,
            ),
            (
                "vorbis",
                include_bytes!("../../../tests/fixtures/audio/vorbis-stereo.webm") as &[u8],
                1.99,
                2.06,
            ),
            (
                "pcm",
                include_bytes!("../../../tests/fixtures/audio/pcm-int.mkv") as &[u8],
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
    fn preroll_target(&self)->Option<i64> {
        (self.codec_tag=="mp4a").then(||self.presentation_floor.unwrap_or(0))
    }
    fn resume_preroll(&mut self,pts:i64)->bool {
        if self.preroll_target().is_none_or(|target|pts>target) {return false;}
        let Some(index)=self.demuxer.packets.iter().position(|p|p.track==self.track_number && p.pts_ns==pts) else {return false;};
        self.packet_index=index;true
    }

    fn codec(&self) -> &str {
        &self.codec_tag
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
        // Blocks are indexed a cluster at a time, so an index that has not met
        // the file's tail has no last block to measure against. What the muxer
        // stated for the whole item is the only answer such an index can give,
        // and the only one a file that states it needs no walk to reach.
        if !self.demuxer.fully_indexed() {
            return self
                .demuxer
                .duration_ns
                .map(std::time::Duration::from_nanos);
        }
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
        let landed = WebmAudioReader::seek(self, pts);
        if self.codec_tag == "mp4a" {
            self.rewind();
            self.presentation_floor = Some(landed);
        }
        landed
    }

    fn present_decoded(&self, mut packet: crate::audio::AudioPacket, source_pts: i64) -> Result<Option<crate::audio::AudioPacket>> {
        if self.codec_tag == "mp4a" {
            if self.presentation_floor.is_some_and(|floor| source_pts < floor) { return Ok(None); }
            // AAC decoder timestamps use a sample clock; Matroska supplies ns.
            packet.pts = source_pts.max(0) as u64;
            packet.timebase_num = 1;
            packet.timebase_den = TIMESCALE_NS;
        }
        Ok(Some(packet))
    }
}

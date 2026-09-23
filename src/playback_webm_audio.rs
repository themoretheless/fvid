//! WebM audio packet extraction.
//!
//! Extracts encoded audio packets from WebM containers for decoding.

use crate::audio::{AudioStream, EncodedPacket};
use crate::container::webm::{Limits, Track, WebmReader};
use crate::{Result, invalid};
use std::io::{Read, Seek};

/// Timestamps in a Matroska segment are nanoseconds.
const TIMESCALE_NS: u32 = 1_000_000_000;

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
        let demuxer = WebmReader::open(reader, limits)?;
        let track = demuxer
            .tracks
            .iter()
            .find(|t| t.kind == 2 && t.codec == "A_VORBIS")
            .ok_or_else(|| invalid("WebM has no supported audio track"))?;
        let extra_data = vorbis_setup_headers(&track.codec_private)?;
        if track.sample_rate == 0 || track.sample_rate > u64::from(u32::MAX) {
            return Err(invalid("WebM audio track has no usable sample rate"));
        }
        if track.channels == 0 || track.channels > u64::from(u16::MAX) {
            return Err(invalid("WebM audio track has no usable channel count"));
        }
        let track_number = track.number;
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
    use super::{WebmAudioReader, vorbis_setup_headers};
    use crate::audio::AudioStream;
    use crate::container::webm::Limits;
    use std::io::Cursor;

    fn header(kind: u8, tail: &[u8]) -> Vec<u8> {
        let mut v = vec![kind, b'v', b'o', b'r', b'b', b'i', b's'];
        v.extend_from_slice(tail);
        v
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

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
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

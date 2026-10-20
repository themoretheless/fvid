//! Thin player adapter over the library-owned Ogg Vorbis program.
use crate::container::ogg::{Limits, Ogg};
use crate::{
    Result,
    audio::{AudioPacket, AudioStep, AudioStream, AudioTrack, EncodedPacket},
};
use fvid_media::owned_ogg_vorbis as owned;
pub use owned::VORBIS;
#[cfg(test)]
use owned::{Geometry, geometry};
use std::{io::Read, time::Duration};
pub struct OggAudioReader {
    program: owned::Program,
}
impl OggAudioReader {
    pub fn open<R: Read>(source: R, limits: Limits) -> Result<Self> {
        Ok(Self {
            program: owned::Program::open(source, limits)?,
        })
    }
    pub fn open_track<R: Read>(source: R, limits: Limits, nth: usize) -> Result<Self> {
        Ok(Self {
            program: owned::Program::open_track(source, limits, nth)?,
        })
    }
    pub fn ogg(&self) -> &Ogg {
        self.program.ogg()
    }
}
fn encoded(p: fvid_media::owned_ogg_opus::EncodedPacket) -> EncodedPacket {
    EncodedPacket {
        data: p.data,
        pts: p.pts,
        duration: p.duration,
    }
}
impl AudioStream for OggAudioReader {
    fn codec(&self) -> &str {
        self.program.codec()
    }
    fn timescale(&self) -> u32 {
        self.program.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.program.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.program.channels()
    }
    fn duration(&self) -> Option<Duration> {
        self.program.duration()
    }
    fn extra_data(&self) -> &[u8] {
        self.program.extra_data()
    }
    fn audio_track_index(&self) -> usize {
        self.program.audio_track_index()
    }
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.program
            .audio_tracks()
            .into_iter()
            .map(|t| AudioTrack {
                sample_rate: t.sample_rate,
                channels: t.channels,
                name: t.name,
                language: t.language,
            })
            .collect()
    }
    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        self.program.next_packet().map(|p| p.map(encoded))
    }
    fn next_step(&mut self) -> Result<Option<AudioStep>> {
        self.program.next_step().map(|s| {
            s.map(|s| match s {
                fvid_media::owned_ogg_opus::AudioStep::ResetDecoder => AudioStep::ResetDecoder,
                fvid_media::owned_ogg_opus::AudioStep::Encoded(p) => AudioStep::Encoded(encoded(p)),
            })
        })
    }
    fn make_decoder(&self) -> Result<Box<dyn crate::audio::AudioDecode>> {
        let (ident, _) = fvid_codecs::vorbis_audio::configuration_headers(self.extra_data())?;
        let shape = fvid_codecs::vorbis::Identification::parse(ident)?;
        crate::codec::make_audio_decoder(
            self.codec(),
            self.extra_data(),
            shape.sample_rate,
            u16::from(shape.channels),
            0,
        )
    }
    fn reset_decoder(&mut self, decoder: &mut Box<dyn crate::audio::AudioDecode>) -> Result<()> {
        *decoder = self.make_decoder()?;
        Ok(())
    }
    fn rewind(&mut self) {
        self.program.rewind()
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.program.seek_to(pts)
    }
    fn preroll_target(&self) -> Option<i64> {
        self.program.preroll_target()
    }
    fn present_decoded(&self, p: AudioPacket, source: i64) -> Result<Option<AudioPacket>> {
        self.program
            .present_decoded(
                fvid_media::owned_ogg_opus::AudioPacket {
                    data: p.data,
                    pts: p.pts,
                    timebase_num: p.timebase_num,
                    timebase_den: p.timebase_den,
                },
                source,
            )
            .map(|p| {
                p.map(|p| AudioPacket {
                    data: p.data,
                    pts: p.pts,
                    timebase_num: p.timebase_num,
                    timebase_den: p.timebase_den,
                })
            })
    }
}
#[cfg(test)]
mod tests {
    use super::{Geometry, OggAudioReader, VORBIS, geometry};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::make_audio_decoder;
    use crate::container::ogg::{Coding, Limits};
    use crate::{Error, Result};

    /// `tests/fixtures/ogg/vorbis-stereo.ogg`, two seconds of 440 Hz sine written by
    /// this machine's FFmpeg:
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=44100:duration=2 \
    ///          -ac 2 -c:a vorbis -strict -2 -b:a 32k tests/fixtures/ogg/vorbis-stereo.ogg
    /// The page list, the packet lengths and the two granules asserted below are
    /// ffprobe's and the framing walk's, page for page.
    const TONE: &[u8] = include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg");
    /// `tests/fixtures/ogg/opus-mono.ogg`, the same tone out of the one Ogg bitstream
    /// on this machine with a writer and no arm here:
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///          -ac 1 -c:a libopus -b:a 24k tests/fixtures/ogg/opus-mono.ogg
    const OPUS: &[u8] = include_bytes!("../../../tests/fixtures/ogg/opus-mono.ogg");

    fn open(bytes: &[u8]) -> Result<OggAudioReader> {
        OggAudioReader::open(bytes, Limits::default())
    }

    /// Change one byte of a real file and re-seal every page over it. Without the
    /// resealing a doctored copy proves only that the checksum fires, which is a
    /// different rule from the one under test.
    fn doctored(bytes: &[u8], at: usize, value: u8) -> Vec<u8> {
        let mut copy = bytes.to_vec();
        copy[at] = value;
        crate::container::ogg::rechecksum(&mut copy);
        copy
    }

    /// The words a refusal went out with, so a test can name the field it means.
    fn said(result: &Result<OggAudioReader>) -> String {
        result.as_ref().err().expect("refused").to_string()
    }

    #[test]
    fn a_real_files_packets_are_where_its_own_pages_put_them() {
        let reader = open(TONE).expect("fixture opens");
        assert_eq!(reader.codec(), VORBIS);
        assert_eq!((reader.sample_rate(), reader.channels()), (44_100, 2));
        assert_eq!(reader.timescale(), 44_100);
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_secs_f64(88_256.0 / 44_100.0)),
            "the length is the granule the end-of-stream page states"
        );
        assert_eq!(reader.ogg().pages(), 4);
        assert_eq!(reader.ogg().chains().len(), 1);
        let stream = &reader.ogg().chains()[0];
        assert_eq!(stream.serial, 0x980c_7718);
        assert_eq!(stream.coding, Coding::Vorbis);
        // Three header packets and 88 audio ones, the last two the padding the
        // encoder writes behind the tone.
        assert_eq!(stream.packets.len(), 91);
        assert_eq!(stream.packets[0].data[..7], *b"\x01vorbis");
        assert_eq!(stream.packets[1].data[..7], *b"\x03vorbis");
        assert_eq!(stream.packets[2].data.len(), 3_247);
        // The setup header is longer than a page can hold, so it arrives in one
        // packet from segments spread over the page beside the comment.
        //
        // Every page numbers itself by the last packet it finishes: the first page
        // holds only the identification packet, the second ends on the setup one,
        // and both state zero because no audio had been written yet.
        assert_eq!(stream.packets[0].end_granule, Some(0));
        assert_eq!(stream.packets[1].end_granule, None);
        assert_eq!(stream.packets[2].end_granule, Some(0));
        assert_eq!(stream.packets[46].end_granule, Some(44_032));
        assert_eq!(stream.packets[90].end_granule, Some(88_256));
        assert_eq!(
            stream.packets[3].data.len(),
            182,
            "the first audio packet, header included"
        );
        // The identification and setup packets, concatenated with the comment out:
        // the form the decoder unpacks.
        let extra = reader.extra_data();
        assert_eq!(&extra[..30], &stream.packets[0].data[..]);
        assert_eq!(&extra[30..], &stream.packets[2].data[..]);
        assert_eq!(reader.audio_tracks()[0].label(), "2 ch 44100 Hz");
    }

    #[test]
    fn every_packet_is_stamped_by_its_decoded_overlap() {
        let reader = open(TONE).expect("fixture opens");
        let shape = geometry(&reader.ogg().chains()[0].packets[0].data).expect("geometry");
        assert_eq!(
            shape,
            Geometry {
                sample_rate: 44_100,
                channels: 2,
                blocks: [11, 11],
            },
            "both exponents share one byte of the identification header"
        );
        // One block of 2048 samples at 44.1 kHz is 1024 samples of audio, so the
        // 88 packets run the timeline to 90 112 samples - the granule's 88 256 plus
        // the pre-roll the encoder put in front of the tone.
        let last = reader.program.packet_times().last().expect("audio");
        assert_eq!((last.0, last.1), (88_064, 1_024));
        assert_eq!(reader.program.packet_times().len(), 88);
        assert!(
            reader
                .program
                .packet_times()
                .collect::<Vec<_>>()
                .windows(2)
                .all(|pair| pair[1].0 == pair[0].0 + pair[0].1),
            "one chain's packets follow each other without a gap"
        );
    }

    /// The whole path over a real file: pages, packets, setup headers, decode.
    #[test]
    fn the_fixture_decodes_to_pcm() {
        let mut reader = open(TONE).expect("fixture opens");
        let mut decoder = make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("Vorbis decoder");
        let mut heard = 0usize;
        let mut decoded = 0usize;
        let bytes = usize::from(reader.channels()) * std::mem::size_of::<f32>();
        while let Some(EncodedPacket {
            data,
            pts,
            duration,
        }) = reader.next_packet().expect("packet")
        {
            let Some(pcm) = decoder
                .decode_encoded(&data, pts as u64, duration as u64)
                .expect("decode")
            else {
                continue;
            };
            assert_eq!(pcm.data.len() % bytes, 0);
            let got = pcm.data.len() / bytes;
            // One block of the 2 048 the packet names is heard as 1 024 samples,
            // except at the front: the first packet's window reaches into the block
            // behind it, so the decoder holds it and gives nothing yet.
            assert_eq!(got, 1_024, "packet {decoded}");
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 44_100));
            heard += got;
            decoded += 1;
        }
        assert_eq!(
            decoded, 87,
            "the first packet primes overlap; subsequent packets return PCM"
        );
        // What a listener is handed, beside the timeline the reader stamped: one
        // block short of it, which is the block the decoder never got the packet for.
        assert_eq!(heard, 89_088);
        assert_eq!(reader.program.packet_times().len(), 88);
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn a_timestamp_lands_on_the_packet_at_or_before_it() {
        let mut reader = open(TONE).expect("fixture opens");
        assert_eq!(reader.seek_to(10_001), 10_001);
        assert_eq!(
            reader.next_packet().unwrap().unwrap().pts,
            0,
            "decode preroll begins at the chain start"
        );
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(1 << 40), 88_256);
        assert!(reader.next_packet().unwrap().is_none());
    }

    /// The sample the coverage matrix pins for the Vorbis row: two chained streams,
    /// each stating its own end, each carrying its own metadata. Its pages are read
    /// below because the file is the row - the player column asks this reader about
    /// it - and it is not a fixture this repo writes.
    #[test]
    fn a_chained_pair_of_streams_is_read_as_one_timeline() {
        let Some(bytes) = std::fs::read(CHAINED).ok() else {
            return;
        };
        let reader = open(&bytes).expect("the reference sample opens");
        assert_eq!((reader.sample_rate(), reader.channels()), (44_100, 1));
        assert_eq!(reader.ogg().chains().len(), 2);
        for chain in reader.ogg().chains() {
            assert_eq!(chain.coding, Coding::Vorbis);
            assert_eq!(chain.packets.len(), 6);
            assert_eq!(
                chain.packets[0].data,
                reader.ogg().chains()[0].packets[0].data
            );
        }
        assert_ne!(
            reader.ogg().chains()[0].serial,
            reader.ogg().chains()[1].serial,
            "a chained stream opens a new serial rather than reusing one"
        );
        // Both chains state 1323 samples of their own, and the second begins where
        // the first's audio leaves off: 3 packets of 256, 2048 and 2048 samples.
        //
        // ffprobe calls this file 0.03 s long, because it stamps a chained stream by
        // the last chain's granule alone. A player that runs the chains one after
        // another offers the pair, so the length it reports is their sum.
        assert_eq!(reader.program.end_samples(), 2 * 1_323);
        assert_eq!(
            reader.program.packet_times().collect::<Vec<_>>(),
            vec![
                (0, 0),
                (0, 576),
                (576, 1_024),
                (1_323, 0),
                (1_323, 576),
                (1_899, 1_024),
            ]
        );
    }

    const CHAINED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benchmarks/data/klite/vorbis--chained-meta.ogg"
    );

    /// The other Ogg coding this machine can write: the reader has the framing and
    /// the name, and no arm for what the name is. That is a coding to implement,
    /// not a file that failed to read, so it goes out as its own kind of error.
    #[test]
    fn an_opus_bitstream_is_refused_by_name() {
        let error = open(OPUS).err().expect("no arm for Opus here");
        let Error::Unsupported(coding) = error else {
            panic!("a read container refuses its coding by name, not as a broken file: {error:?}");
        };
        assert!(coding.contains("OpusHead"), "{coding}");
        assert_eq!(Coding::Opus.describe(b"OpusHead"), "OpusHead");
    }

    #[test]
    fn a_header_that_does_not_add_up_is_refused_rather_than_timed_wrong() {
        // Both the reference sample and the fixture state usable exponents; a header
        // whose two exponents are the wrong way round, out of range, or whose framing
        // marker is missing states a timeline nothing can be stamped on. Each
        // refusal is checked by its own words, since a doctored file can also fail
        // the checksum, and that would hide which rule fired.
        //
        // The identification packet starts at byte 28 of the fixture, behind the
        // first page's 27 header bytes and its one segment length.
        let (exponents, framing, version) = (56, 57, 35);
        let swapped = open(&doctored(TONE, exponents, 0x8b));
        assert!(
            said(&swapped).contains("short block size exponent 11 exceeds"),
            "{}",
            said(&swapped)
        );
        let out_of_range = open(&doctored(TONE, exponents, 0x1f));
        assert!(
            said(&out_of_range).contains("exponent 15 is out of the range"),
            "{}",
            said(&out_of_range)
        );
        let unframed = open(&doctored(TONE, framing, 0x00));
        assert!(
            said(&unframed).contains("framing marker"),
            "{}",
            said(&unframed)
        );
        let revised = open(&doctored(TONE, version, 0x02));
        assert!(
            said(&revised).contains("Vorbis version 2"),
            "{}",
            said(&revised)
        );
        // And the same bytes left alone still open, so what failed is the field.
        assert!(open(TONE).is_ok());
    }

    /// A page lying about its own checksum is not a page: the walk that rebuilds
    /// packets from a segment table has nothing left to trust.
    #[test]
    fn a_file_whose_page_checksum_lies_is_refused() {
        let mut copy = TONE.to_vec();
        let last = copy.len() - 1;
        copy[last] ^= 0xff;
        let error = open(&copy).err().expect("a corrupted page is refused");
        assert!(error.to_string().contains("checksum"), "{error}");
        assert!(open(TONE).is_ok(), "the same byte untouched");
    }

    /// Where a later chain states a different program, one decoder run cannot carry
    /// both - so the file is refused at the chain that disagrees rather than played
    /// with the first chain's codebooks.
    #[test]
    fn a_chained_pair_that_changes_coding_mid_file_is_refused() {
        let Some(bytes) = std::fs::read(CHAINED).ok() else {
            return;
        };
        // The second chain's identification packet begins where its second page
        // begins; move one of its block-size exponents and the pair no longer states
        // one program.
        let boundary = bytes
            .windows(7)
            .skip(30)
            .rposition(|w| w == *b"\x01vorbis")
            .expect("two identification packets");
        let at = boundary + 30 + 28;
        let copy = doctored(&bytes, at, 0x0c);
        assert_eq!(copy[at - 28..][..7], *b"\x01vorbis");
        let error = open(&copy).err().expect("a second chain that disagrees");
        assert!(error.to_string().contains("exponent"), "{error}");
        // The two chains' comments do differ - each states its own title - and that
        // is what the sample is named for, so the pair opens with one decoder run.
        assert!(
            open(&bytes).is_ok(),
            "the same file with both headers agreeing"
        );
    }
}

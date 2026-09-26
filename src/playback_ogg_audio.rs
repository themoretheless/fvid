//! Audio playback of Ogg Vorbis out of the `.ogg` file the codec ships in.
//!
//! Vorbis is the one coding on this machine's list of Ogg bitstreams that the
//! player already decodes, and the container gives the reader three things to find
//! in it: the geometry, which lives in the identification packet rather than in a
//! header beside the audio; the identification and setup packets a Vorbis decoder
//! needs before it sees a single sample; and the timeline, which Ogg states as a
//! sample count in the granule of each page while the packets themselves carry only
//! their block size. [`crate::container::ogg`] reads the framing and stops there,
//! because what a bitstream *is* is not a container question; naming the coding,
//! laying the timeline out and dropping the packet a Vorbis decoder has no use for
//! is this file's job.
//!
//! A stream is three header packets - identification, comment, setup - and then
//! audio for as far as its serial runs. The comment packet is metadata, and like
//! the WebM reader this one leaves it out of what it hands the decoder. Neither of
//! the timeline's numbers is stored anywhere either: a Vorbis packet holds half its
//! block size in samples, and which of the identification header's two block sizes
//! a packet uses is one bit of the packet's own first byte, so every timestamp
//! below is computed rather than read.
//!
//! Chained streams - one Vorbis bitstream ending and another beginning, which is
//! how a gapless album and half the Ogg samples in the K-Lite list are written -
//! are played end to end as one timeline, which is what a listener hears them as.
//! That is only possible while every chain states the same program, so a chain
//! whose identification or setup header differs from the first is refused rather
//! than decoded with the wrong codebooks.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::container::ogg::{Coding, Limits, Ogg, Packet};
use crate::{Result, invalid, unsupported};
use std::io::Read;
use std::time::Duration;

/// The tag [`crate::codec::make_audio_decoder`] answers for Vorbis, as Matroska
/// spells it - the same tag a WebM track of this codec arrives under.
pub const VORBIS: &str = "A_VORBIS";

/// The second and third of the three packets every Vorbis bitstream opens with.
/// The first is what names the bitstream, so the container has already read it.
const COMMENT: &[u8] = b"\x03vorbis";
const SETUP: &[u8] = b"\x05vorbis";

/// One audio packet of the timeline: which packet of which chain it is, and where
/// it sits in samples of the track's own rate.
#[derive(Clone, Copy, Debug)]
struct Stamp {
    chain: usize,
    packet: usize,
    pts: i64,
    duration: i64,
}

/// What an identification header states about the track, once checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Geometry {
    sample_rate: u32,
    channels: u16,
    /// The two block sizes as their powers of two, in the order a packet's own bit
    /// picks between them.
    blocks: [u8; 2],
}

/// Read the 30 bytes an identification header is made of. Every field is checked
/// against what the decoder down the line does with it: a header whose block sizes
/// are out of range or the wrong way round states a timeline this reader cannot lay
/// out, and symphonia refuses the same pair of exponents.
fn geometry(ident: &[u8]) -> Result<Geometry> {
    // The concatenated form handed to the decoder assumes the header is exactly as
    // long as the codec defines, since the setup packet starts at the byte after it.
    let header: [u8; 30] = ident.try_into().map_err(|_| {
        invalid(&format!(
            "a Vorbis identification header is 30 bytes and this one is {}",
            ident.len()
        ))
    })?;
    let version = u32::from_le_bytes(header[7..11].try_into().expect("four version bytes"));
    if version != 0 {
        return Err(invalid(&format!(
            "Vorbis version {version} is not the one bitstream this reader times"
        )));
    }
    let width = header[11];
    if width == 0 || width > 32 {
        return Err(invalid(&format!(
            "a Vorbis track of {width} channels states no layout a decoder maps"
        )));
    }
    let channels = u16::from(width);
    let sample_rate = u32::from_le_bytes(header[12..16].try_into().expect("four rate bytes"));
    if sample_rate == 0 {
        return Err(invalid("a Vorbis track states a sample rate of zero"));
    }
    // The two exponents share one byte and the framing marker follows them, which
    // is what tells this header from one a draft writer left half-finished.
    let blocks = [header[28] & 0x0f, header[28] >> 4];
    if header[29] != 1 {
        return Err(invalid(&format!(
            "a Vorbis identification header ends with {:#04x} where its framing marker is 1",
            header[29]
        )));
    }
    for (which, exponent) in [("short", blocks[0]), ("long", blocks[1])] {
        if !(6..=13).contains(&exponent) {
            return Err(invalid(&format!(
                "the {which} Vorbis block size exponent {exponent} is out of the range 6..=13"
            )));
        }
    }
    if blocks[0] > blocks[1] {
        return Err(invalid(&format!(
            "a Vorbis track's short block size exponent {} exceeds its long one {}",
            blocks[0], blocks[1]
        )));
    }
    Ok(Geometry {
        sample_rate,
        channels,
        blocks,
    })
}

/// How many samples a packet holds: half the block size its first byte names, the
/// long one where bit 1 of that byte is set and the short one where it is not.
fn samples(packet: &Packet, blocks: [u8; 2]) -> Result<i64> {
    let flag = packet
        .data
        .first()
        .ok_or_else(|| invalid("a Vorbis bitstream holds an audio packet with nothing in it"))?;
    Ok(1i64 << (usize::from(blocks[usize::from(flag & 2 != 0)]) - 1))
}

/// An Ogg file's Vorbis track, read as an audio stream.
pub struct OggAudioReader {
    ogg: Ogg,
    packets: Vec<Stamp>,
    packet: usize,
    shape: Geometry,
    /// The identification and setup packets concatenated, which is the form the
    /// decoder unpacks - without the comment packet that sits between them in the
    /// stream.
    extra_data: Vec<u8>,
    /// Samples the chains state that a listener hears, added up.
    stated: i64,
}

impl OggAudioReader {
    /// Open the file's Vorbis track, whichever chain of the file it sits in. A file
    /// carrying only another coding is refused by its own name: the container was
    /// read, and what it names is the coding with no arm here, which is a different
    /// gap from a container nobody opens.
    pub fn open<R: Read>(reader: R, limits: Limits) -> Result<Self> {
        let ogg = Self::read(reader, limits)?;
        let playable: Vec<usize> = ogg
            .chains()
            .iter()
            .enumerate()
            .filter(|(_, chain)| chain.coding == Coding::Vorbis)
            .map(|(number, _)| number)
            .collect();
        if playable.is_empty() {
            let named: Vec<String> = ogg
                .chains()
                .iter()
                .map(|chain| {
                    let opening = chain
                        .packets
                        .first()
                        .map(|packet| packet.data.as_slice())
                        .unwrap_or_default();
                    chain.coding.describe(opening)
                })
                .collect();
            return Err(unsupported(&format!(
                "Ogg bitstream coded {} has no decoder arm here",
                named.join(" / ")
            )));
        }
        // The first chain states the program, geometry and codebooks both.
        let headers = &ogg.chains()[playable[0]].packets;
        if headers.len() < 3
            || !headers[1].data.starts_with(COMMENT)
            || !headers[2].data.starts_with(SETUP)
        {
            return Err(invalid(
                "an Ogg Vorbis bitstream does not hold its three header packets in the order the codec defines",
            ));
        }
        let shape = geometry(&headers[0].data)?;
        let mut extra_data = headers[0].data.clone();
        extra_data.extend_from_slice(&headers[2].data);
        let mut stated = 0i64;
        let mut packets: Vec<Stamp> = Vec::new();
        let mut pts = 0i64;
        for chain in playable {
            let stream = &ogg.chains()[chain];
            // Every chain after the first is only the same program if the two
            // packets that state it - the geometry and the codebooks - agree with
            // the first byte for byte. The comment between them is metadata each
            // chain is free to carry in its own words, which is how a chained
            // album states a title per movement.
            if stream.packets.len() < 3
                || stream.packets[0].data != headers[0].data
                || stream.packets[2].data != headers[2].data
            {
                return Err(invalid(&format!(
                    "bitstream {} is coded differently from the chain ahead of it",
                    stream.serial
                )));
            }
            if stream.packets.len() < 4 {
                return Err(invalid(&format!(
                    "bitstream {} is three header packets and no audio",
                    stream.serial
                )));
            }
            let start = pts;
            for (number, packet) in stream.packets.iter().enumerate().skip(3) {
                let duration = samples(packet, shape.blocks)?;
                packets.push(Stamp {
                    chain,
                    packet: number,
                    pts,
                    duration,
                });
                pts += duration;
            }
            // The position a chain's pages state, taking the last one written in it.
            // Where none carries a position the samples its packets add up to are all
            // the file has to say.
            stated += stream
                .packets
                .iter()
                .rev()
                .find_map(|packet| packet.end_granule)
                .unwrap_or(pts - start);
        }
        Ok(Self {
            ogg,
            packets,
            packet: 0,
            shape,
            extra_data,
            stated,
        })
    }

    /// Take the whole file in: Ogg packets carry no lengths of their own, so the
    /// page walk that rebuilds them reads every byte.
    fn read<R: Read>(mut reader: R, limits: Limits) -> Result<Ogg> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        Ogg::parse(&bytes, &limits)
    }

    /// The container's own shape, for a caller that wants to say what it opened.
    pub fn ogg(&self) -> &Ogg {
        &self.ogg
    }
}

impl AudioStream for OggAudioReader {
    fn codec(&self) -> &str {
        VORBIS
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.shape.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.shape.sample_rate
    }

    fn channels(&self) -> u16 {
        self.shape.channels
    }

    /// What the chains' pages state as their ends, added up: the samples a listener
    /// hears, which is shorter than the timeline above by the pre-roll a Vorbis
    /// stream encodes in front of the audio.
    fn duration(&self) -> Option<Duration> {
        let samples = u64::try_from(self.stated).ok()?;
        Some(Duration::from_secs_f64(
            samples as f64 / f64::from(self.shape.sample_rate),
        ))
    }

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// One program: a file's chains run one after another rather than beside each
    /// other, so there is no second track to choose.
    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.shape.sample_rate,
            channels: self.shape.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.packets.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.ogg.chains()[at.chain].packets[at.packet].data.clone(),
            pts: at.pts,
            duration: at.duration,
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0);
        let index = self.packets.partition_point(|at| at.pts <= target);
        let index = index.saturating_sub(1);
        self.packet = index;
        self.packets.get(index).map_or(0, |at| at.pts)
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
    const TONE: &[u8] = include_bytes!("../tests/fixtures/ogg/vorbis-stereo.ogg");
    /// `tests/fixtures/ogg/opus-mono.ogg`, the same tone out of the one Ogg bitstream
    /// on this machine with a writer and no arm here:
    ///   ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.2 \
    ///          -ac 1 -c:a libopus -b:a 24k tests/fixtures/ogg/opus-mono.ogg
    const OPUS: &[u8] = include_bytes!("../tests/fixtures/ogg/opus-mono.ogg");

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
    fn every_packet_is_stamped_at_half_its_block_size_apart() {
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
        let last = reader.packets.last().expect("audio");
        assert_eq!((last.pts, last.duration), (89_088, 1_024));
        assert_eq!(reader.packets.len(), 88);
        assert!(
            reader
                .packets
                .windows(2)
                .all(|pair| pair[1].pts == pair[0].pts + pair[0].duration),
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
            assert_eq!(
                got,
                if decoded == 0 { 0 } else { 1_024 },
                "packet {decoded}"
            );
            assert_eq!((pcm.timebase_num, pcm.timebase_den), (1, 44_100));
            heard += got;
            decoded += 1;
        }
        assert_eq!(decoded, 88, "every packet of a real file decodes");
        // What a listener is handed, beside the timeline the reader stamped: one
        // block short of it, which is the block the decoder never got the packet for.
        assert_eq!(heard, 89_088);
        assert_eq!(reader.packets.len(), 88);
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn a_timestamp_lands_on_the_packet_at_or_before_it() {
        let mut reader = open(TONE).expect("fixture opens");
        assert_eq!(reader.seek_to(89_088), 89_088);
        assert_eq!(reader.seek_to(89_089), 89_088);
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(1 << 40), 89_088);
        assert!(reader.next_packet().expect("packet").is_some());
        assert!(reader.next_packet().expect("no more").is_none());
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
        assert_eq!(reader.stated, 2 * 1_323);
        assert_eq!(
            reader
                .packets
                .iter()
                .map(|at| (at.pts, at.duration))
                .collect::<Vec<_>>(),
            vec![
                (0, 128),
                (128, 1_024),
                (1_152, 1_024),
                (2_176, 128),
                (2_304, 1_024),
                (3_328, 1_024),
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
        assert!(error.to_string().contains("coded differently"), "{error}");
        // The two chains' comments do differ - each states its own title - and that
        // is what the sample is named for, so the pair opens with one decoder run.
        assert!(
            open(&bytes).is_ok(),
            "the same file with both headers agreeing"
        );
    }
}

//! Enhanced AC-3: the coding Annex E of ATSC A/52:2012 attaches to AC-3, which is
//! what Dolby Digital Plus delivers. A packet is one or more syncframes, each of
//! which carries up to six audio blocks of 256 samples per channel, so a frame of
//! six spans the same 32 ms an AC-3 frame does in a third of the frames.
//!
//! What is read here is the header: a syncframe's own geometry, which is all a
//! walker needs to list a stream's packets and all a container route needs to say
//! what a track sounds like before a single block is unpacked. Annex E packs every
//! field most significant bit first, unlike the AC-3 `bsi` this syntax lives
//! inside, and it writes the frame's length into the header rather than a code into
//! a table, so `(frmsiz + 1) * 2` is the byte count at which the next syncframe
//! starts.
//!
//! The metadata behind the geometry is stepped over field by field because none of
//! it changes how the audio unpacks - with one exception worth naming where it is,
//! since a decoder that folds a stream down will want it: the `mixmdate` group
//! carries this stream's own Lo/Ro and L/R/Ls/Rs coefficients, which is where an
//! E-AC-3 downmix finds its levels instead of the fixed ones AC-3 mixes by.
//!
//! Every refusal says what it refuses, because a file that needs syntax this module
//! does not have yet is a missing feature and not a broken file, and the caller
//! cannot tell them apart unless the decoder can:
//!
//! * a stream type other than independent - a dependent substream states nothing of
//!   its own and leans on the independent frame before it, and a converted one
//!   repeats AC-3's header, so both need a reader this one is not yet;
//! * a second substream of the same presentation, which is the other half of a pair
//!   the first point refuses;
//! * `bsid` other than 16, which is Annex E's own mark: below it the frame is AC-3,
//!   above it a revision not transcribed;
//! * a frame of one, two or three blocks, which is the shape that carries AC-3's
//!   conversion layer and with it the three-way block switching this decoder will
//!   not have to solve while six-block frames are what exists;
//! * the reduced rates `fscod` = 3 names, whose band edges and thresholds the AC-3
//!   tables this core reuses are keyed to the three full rates.
//!
//! The constants and field layouts are transcribed from the public ATSC A/52:2012
//! text, Annex E, sections 2.2.1 to 2.3.1.

use crate::codec::bits::BitReader;
use crate::{Result, invalid, unsupported};

/// The 16-bit mark a syncframe opens with, shared with AC-3 by Table E1.1.
const SYNCWORD: u32 = 0x0B77;
/// The `bsid` that says the syntax is Annex E's rather than AC-3's, section 2.1.
const EAC3_BSID: u32 = 16;
/// Rates the 2-bit `fscod` names, Table E2.2.
const SAMPLE_RATES: [u32; 3] = [48_000, 44_100, 32_000];
/// Full-bandwidth channels per `acmod`, the count Annex E borrows from AC-3.
const NFCHANS: [usize; 8] = [2, 1, 2, 3, 3, 4, 4, 5];
/// Audio blocks per syncframe for `numblkscod`, Table E2.4.
const BLOCK_COUNTS: [usize; 4] = [1, 2, 3, 6];

/// What a syncframe says about itself before any audio is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syncframe {
    /// The rate the frame's audio plays back at, from `fscod`.
    pub sample_rate: u32,
    /// The frame's own channel count in native order, which is what a decoder handed
    /// this frame produces when the container states no other layout.
    pub channels: u16,
    /// Bytes the frame occupies, from `frmsiz`, so the next one starts here.
    pub frame_bytes: usize,
    /// Audio blocks the frame carries, each of 256 samples per channel.
    pub blocks: usize,
}

/// Read the header of the syncframe that starts at `bytes`.
///
/// A bare `.ec3` file states its geometry in every frame rather than once up front,
/// so the walker that lists a stream's packets needs the same numbers the decoder
/// will. `None` for the same refusals [`header`] makes.
pub fn syncframe(bytes: &[u8]) -> Option<Syncframe> {
    header(&mut BitReader::new(bytes)).ok()
}

/// Read `syncinfo` and step over `bsi`, Tables E1.1 and E1.2, leaving the reader at
/// the audio frame.
fn header(bits: &mut BitReader<'_>) -> Result<Syncframe> {
    if bits.read(16)? != SYNCWORD {
        return Err(invalid("no E-AC-3 syncword at the start of the syncframe"));
    }
    // The two fields that say which substream this is come first, and both of the
    // kinds that are not this decoder's are named before anything else is read, so
    // that a stream needing them fails here rather than half a block later.
    let strmtyp = bits.read(2)?;
    if strmtyp != 0 {
        return Err(unsupported(&format!(
            "E-AC-3 stream type {strmtyp}, which is not an independent substream"
        )));
    }
    if bits.read(3)? != 0 {
        return Err(unsupported("a second E-AC-3 substream of the same presentation"));
    }
    // `frmsiz` counts 16-bit words in the syncframe, itself included.
    let frame_bytes = bits.read(11)? as usize * 2 + 2;
    let fscod = bits.read(2)? as usize;
    if fscod == 3 {
        return Err(unsupported("an E-AC-3 frame at a reduced sample rate"));
    }
    let rate = SAMPLE_RATES[fscod];
    let numblkscod = bits.read(2)? as usize;
    if numblkscod != 3 {
        return Err(unsupported(&format!(
            "an E-AC-3 frame of {} blocks, which carries AC-3's conversion layer",
            BLOCK_COUNTS[numblkscod]
        )));
    }
    let blocks = BLOCK_COUNTS[numblkscod];
    let acmod = bits.read(3)? as usize;
    let lfeon = bits.bit()?;
    let bsid = bits.read(5)?;
    if bsid != EAC3_BSID {
        return Err(unsupported(&format!(
            "an E-AC-3 stream marked with bsid {bsid}, which names {}",
            if bsid < EAC3_BSID { "AC-3's own syntax" } else { "a later revision" }
        )));
    }
    bsi(bits, acmod, lfeon, blocks)?;
    if bits.position() > frame_bytes * 8 {
        return Err(invalid("an E-AC-3 header longer than the frame that carries it"));
    }
    Ok(Syncframe {
        sample_rate: rate,
        channels: (NFCHANS[acmod] + usize::from(lfeon)) as u16,
        frame_bytes,
        blocks,
    })
}

/// Step over `bsi` past the fields `header` has already read: Table E1.2 from
/// `dialnorm` on. `strmtyp` is known independent and `numblkscod` known six blocks,
/// which is what retires the dependent-stream and conversion fields of the table.
fn bsi(bits: &mut BitReader<'_>, acmod: usize, lfeon: bool, blocks: usize) -> Result<()> {
    bits.skip(5)?; // dialnorm
    if bits.bit()? {
        bits.skip(8)?; // compr
    }
    if acmod == 0 {
        // 1+1 mode gives some of the metadata a second value of its own.
        bits.skip(5)?; // dialnorm2
        if bits.bit()? {
            bits.skip(8)?; // compr2
        }
    }
    if bits.bit()? {
        mixing(bits, acmod, lfeon, blocks)?;
    }
    if bits.bit()? {
        information(bits, acmod)?;
    }
    if bits.bit()? {
        let length = bits.read(6)? as usize;
        bits.skip((length + 1) * 8)?; // addbsi
    }
    Ok(())
}

/// Step over the mixing metadata of Table E1.2, which is where the downmix
/// coefficients an E-AC-3 fold reads live.
fn mixing(bits: &mut BitReader<'_>, acmod: usize, lfeon: bool, blocks: usize) -> Result<()> {
    if acmod > 2 {
        bits.skip(2)?; // dmixmod
    }
    if acmod > 2 && acmod & 1 != 0 {
        bits.skip(3)?; // ltrtcmixlev
        bits.skip(3)?; // lorocmixlev
    }
    if acmod & 4 != 0 {
        bits.skip(3)?; // ltrtsurmixlev
        bits.skip(3)?; // lorosurmixlev
    }
    if lfeon && bits.bit()? {
        bits.skip(5)?; // lfemixlevcod
    }
    if bits.bit()? {
        bits.skip(6)?; // pgmscl
    }
    if acmod == 0 && bits.bit()? {
        bits.skip(6)?; // pgmscl2
    }
    if bits.bit()? {
        bits.skip(6)?; // extpgmscl
    }
    match bits.read(2)? {
        // mixdef: how the mixing definition of an external program is spelled.
        0 => {}
        1 => {
            bits.skip(1)?; // premixcmpsel
            bits.skip(1)?; // drcsrc
            bits.skip(3)?; // premixcmpscl
        }
        2 => bits.skip(12)?, // mixdata
        _ => external(bits)?,
    }
    if acmod < 2 {
        if bits.bit()? {
            bits.skip(8)?; // panmean
            bits.skip(6)?; // paninfo, reserved
        }
        if acmod == 0 && bits.bit()? {
            bits.skip(8)?; // panmean2
            bits.skip(6)?; // paninfo2, reserved
        }
    }
    if bits.bit()? {
        // A frame of one block states its mixing configuration once; six-block
        // frames, which are all this reader accepts, state it per block.
        for _ in 0..blocks {
            if bits.bit()? {
                bits.skip(5)?; // blkmixcfginfo
            }
        }
    }
    Ok(())
}

/// Step over the most flexible mixing definition: `mixdeflen` says how many bytes
/// the group it opens spans, and the scaling flags and `mixdata` together fill
/// exactly that, with `mixdatafill` rounding the end up to a byte boundary. Reading
/// it as a total is what keeps the walk honest whatever the flags inside hold.
fn external(bits: &mut BitReader<'_>) -> Result<()> {
    let start = bits.position();
    let group = (bits.read(5)? as usize + 2) * 8; // mixdeflen counts the bytes of the group it opens
    if bits.bit()? {
        bits.skip(1)?; // premixcmpsel
        bits.skip(1)?; // drcsrc
        bits.skip(3)?; // premixcmpscl
        for _ in 0..6 {
            if bits.bit()? {
                bits.skip(4)?; // the left, centre, right, two surround and LFE scales
            }
        }
        if bits.bit()? {
            for _ in 0..2 {
                if bits.bit()? {
                    bits.skip(4)?; // the two auxiliary scales
                }
            }
        }
    }
    if bits.bit()? {
        bits.skip(5)?; // spchdat
        if bits.bit()? {
            bits.skip(5)?; // spchdat1
            bits.skip(2)?; // spchan1att
            if bits.bit()? {
                bits.skip(5)?; // spchdat2
                bits.skip(3)?; // spchan2att
            }
        }
    }
    let used = bits.position() - start;
    let rest = group
        .checked_sub(used)
        .ok_or_else(|| invalid("E-AC-3 mixing parameters longer than the group that holds them"))?;
    bits.skip(rest)?; // mixdata and the fill bits that round it up
    Ok(())
}

/// Step over the informational metadata, which is what a player shows rather than
/// what it mixes by.
fn information(bits: &mut BitReader<'_>, acmod: usize) -> Result<()> {
    bits.skip(3)?; // bsmod
    bits.skip(1)?; // copyrightb
    bits.skip(1)?; // origbs
    if acmod == 2 {
        bits.skip(2)?; // dsurmod
        bits.skip(2)?; // dheadphonmod
    }
    if acmod >= 6 {
        bits.skip(2)?; // dsurexmod
    }
    if bits.bit()? {
        bits.skip(5)?; // mixlevel
        bits.skip(2)?; // roomtyp
        bits.skip(1)?; // adconvtyp
    }
    if acmod == 0 && bits.bit()? {
        bits.skip(5)?; // mixlevel2
        bits.skip(2)?; // roomtyp2
        bits.skip(1)?; // adconvtyp2
    }
    // sourcefscod is there for every rate this reader takes, and the conversion
    // flag that follows it only for a frame of fewer than six blocks.
    bits.skip(1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2/0 stream at 48 kHz and 192 kbps whose channels each carry two tones, one
    /// frame per 32 ms.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)+0.1*sin(2*PI*1500*t)|0.2*sin(2*PI*220*t)+0.1*sin(2*PI*300*t):s=48000:d=0.25' \
    ///   -c:a eac3 -b:a 192k tests/fixtures/audio/eac3-stereo.ec3
    /// ```
    const STEREO: &[u8] = include_bytes!("../../tests/fixtures/audio/eac3-stereo.ec3");

    /// A 5.1 stream at 448 kbps with one tone per channel, so the channel count the
    /// header reports is the count the blocks will hold and the LFE is the difference
    /// between the two.
    ///
    /// ```text
    /// ffmpeg -hide_banner -loglevel error -y -f lavfi \
    ///   -i 'aevalsrc=0.3*sin(2*PI*440*t)|0.2*sin(2*PI*220*t)|0.15*sin(2*PI*1000*t)|0.1*sin(2*PI*3000*t)|0.05*sin(2*PI*5500*t)|0.4*sin(2*PI*60*t):s=48000:d=0.25' \
    ///   -ch_layout 5.1 -c:a eac3 -b:a 448k tests/fixtures/audio/eac3-51.ec3
    /// ```
    const SURROUND: &[u8] = include_bytes!("../../tests/fixtures/audio/eac3-51.ec3");

    /// AC-3's own 2/0 stream, which shares the syncword and nothing else: reading it
    /// as Annex E must say so rather than invent a geometry.
    const AC3: &[u8] = include_bytes!("../../tests/fixtures/audio/ac3-stereo.ac3");

    /// Walk a whole elementary stream frame by frame, the way a bare-file reader will.
    fn walk(bytes: &[u8]) -> Vec<Syncframe> {
        let mut frames = Vec::new();
        let mut rest = bytes;
        while let Some(frame) = syncframe(rest) {
            rest = &rest[frame.frame_bytes..];
            frames.push(frame);
        }
        assert!(rest.is_empty(), "the walk stopped {} bytes early", rest.len());
        frames
    }

    /// The bits planted behind the metadata of a crafted frame, which no field of
    /// Table E1.2 spells this way: a walk that counts one bit too many or too few
    /// finds something else there.
    const MARKER: u32 = 0x2DD3;

    /// A frame's own bits, written one field at a time, for the syntax the encoders
    /// on this machine do not produce.
    #[derive(Default)]
    struct Sink {
        bytes: Vec<u8>,
        spare: u8,
        width: usize,
    }

    impl Sink {
        fn put(&mut self, value: u32, count: usize) -> &mut Self {
            for step in (0..count).rev() {
                let bit = if step < usize::BITS as usize {
                    ((value >> step) & 1) as u8
                } else {
                    0
                };
                self.push(bit);
            }
            self
        }

        /// `count` bits of nothing, for the slack a walk has to run into before a
        /// test can see that it ran too far.
        fn zeros(&mut self, count: usize) -> &mut Self {
            for _ in 0..count {
                self.push(0);
            }
            self
        }

        fn push(&mut self, bit: u8) {
            self.spare = (self.spare << 1) | bit;
            self.width += 1;
            if self.width == 8 {
                self.bytes.push(self.spare);
                self.spare = 0;
                self.width = 0;
            }
        }

        /// Pad out to the next byte, so the frame is a whole number of them.
        fn finish(&mut self) -> &mut Self {
            if self.width != 0 {
                let rest = 8 - self.width;
                self.zeros(rest);
            }
            self
        }
    }

    /// A six-block frame at 48 kHz in `acmod`, with `compre`'s compression word when
    /// asked for, then whatever `metadata` writes from the `mixmdate` flag on, then
    /// the marker and enough slack behind it that a walk that runs on is seen to.
    fn frame(acmod: usize, lfeon: bool, compre: bool, metadata: impl FnOnce(&mut Sink)) -> Vec<u8> {
        let mut sink = Sink::default();
        sink.put(0x0B77, 16)
            .put(0, 2) // strmtyp: independent
            .put(0, 3) // substreamid: the only one a frame of this kind has
            .put(200, 11) // frmsiz: the frame claims 402 bytes
            .put(0, 2) // fscod: 48 kHz
            .put(3, 2) // numblkscod: six blocks
            .put(acmod as u32, 3)
            .put(u32::from(lfeon), 1)
            .put(EAC3_BSID, 5)
            .put(31, 5) // dialnorm
            .put(u32::from(compre), 1);
        if compre {
            sink.put(1, 8); // compr
        }
        if acmod == 0 {
            sink.put(31, 5).put(0, 1); // dialnorm2, compr2e
        }
        metadata(&mut sink);
        sink.put(MARKER, 16).zeros(64).finish();
        sink.bytes
    }

    /// Where the header of a real frame leaves its own reader: the number the crafted
    /// frames below are measured against.
    fn header_bits(bytes: &[u8]) -> usize {
        let mut bits = BitReader::new(bytes);
        header(&mut bits).unwrap();
        bits.position()
    }

    /// Read a crafted frame's header, insist the marker is the next thing in it, and
    /// say where the walk stopped.
    fn stopped_at(bytes: &[u8], channels: u16) -> usize {
        let mut bits = BitReader::new(bytes);
        let geometry = header(&mut bits).unwrap();
        assert_eq!(
            geometry,
            Syncframe {
                sample_rate: 48_000,
                channels,
                frame_bytes: 402,
                blocks: 6
            }
        );
        let position = bits.position();
        assert_eq!(
            bits.read(16).unwrap(),
            MARKER,
            "the walk left the reader somewhere inside the frame's own bits"
        );
        position
    }

    #[test]
    fn every_frame_of_a_stream_states_the_same_geometry() {
        let frames = walk(STEREO);
        assert_eq!(
            frames[0],
            Syncframe {
                sample_rate: 48_000,
                channels: 2,
                frame_bytes: 768,
                blocks: 6
            }
        );
        assert_eq!(frames.len(), STEREO.len() / 768);
        assert!(frames.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn the_low_frequency_channel_is_one_of_the_ones_a_frame_counts() {
        let frames = walk(SURROUND);
        assert_eq!(
            frames[0],
            Syncframe {
                sample_rate: 48_000,
                channels: 6,
                frame_bytes: 1792,
                blocks: 6
            }
        );
        assert_eq!(frames.len(), SURROUND.len() / 1792);
    }

    /// The two syntaxes share their first two bytes and disagree on every bit after
    /// them, and AC-3 packs them the other way round, so an AC-3 frame walked as
    /// Annex E falls over on the first field past the syncword. Either of the two
    /// refusals is a good outcome; what matters is that no geometry is invented.
    #[test]
    fn an_ac_three_frame_is_refused_before_it_is_misread() {
        let error = header(&mut BitReader::new(AC3)).unwrap_err();
        assert!(matches!(error, crate::Error::Unsupported(_)), "{error}");
        let said = error.to_string();
        assert!(
            said.contains("stream type") || said.contains("bsid"),
            "{said} names neither of the two marks that tell the syntaxes apart"
        );
    }

    #[test]
    fn a_stream_that_is_not_the_independent_one_is_named_not_guessed_at() {
        for (kind, words) in [(1u32, "type 1"), (2, "type 2")] {
            let mut bytes = STEREO.to_vec();
            // The stream type is the frame's first two bits past the syncword.
            bytes[2] = (bytes[2] & 0x3F) | u8::try_from(kind << 6).unwrap();
            let error = header(&mut BitReader::new(&bytes)).unwrap_err();
            assert!(error.to_string().contains(words), "{error}");
        }
        let mut bytes = STEREO.to_vec();
        bytes[2] |= 0x20; // the top bit of the three that name the substream
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("second"), "{error}");
    }

    #[test]
    fn the_shapes_this_reader_will_not_unpack_yet_are_refused_by_name() {
        // One, two or three blocks per frame is the shape that carries AC-3's
        // conversion layer; six is what the encoders here write.
        let mut bytes = STEREO.to_vec();
        bytes[4] &= 0xEF; // the low bit of the two that count the blocks
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("3 blocks"), "{error}");

        // The reduced rates are the fourth `fscod`, which moves the band edges the
        // reused AC-3 tables are keyed to.
        let mut bytes = STEREO.to_vec();
        bytes[4] |= 0xC0;
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(error.to_string().contains("reduced"), "{error}");
    }

    #[test]
    fn a_header_longer_than_the_frame_that_carries_it_is_not_read() {
        let mut bytes = STEREO.to_vec();
        // The eleven bits of `frmsiz` start at the frame's 22nd bit; one whole frame
        // is four bytes, which cannot hold a header of 54 bits.
        bytes[2] &= 0xF8;
        bytes[3] = 0x01;
        let error = header(&mut BitReader::new(&bytes)).unwrap_err();
        assert!(matches!(error, crate::Error::Invalid(_)), "{error}");
        assert!(error.to_string().contains("longer"), "{error}");
    }

    /// A frame that says it carries no metadata at all: 45 bits of geometry, then
    /// `dialnorm`, `compre`, and the three flags that end the bit stream information.
    /// Every syncframe of both fixtures stops there.
    #[test]
    fn a_frame_without_metadata_ends_where_the_real_ones_do() {
        let stopped = stopped_at(
            &frame(2, false, false, |sink| {
                sink.put(0, 1) // mixmdate
                    .put(0, 1) // infomdate
                    .put(0, 1); // addbsie
            }),
            2,
        );
        assert_eq!(stopped, 54);
        assert_eq!(stopped, header_bits(STEREO), "as an encoder measured it");
    }

    /// The 5.1 broadcast shape: a preferred downmix mode, both pairs of folding
    /// levels, a stated LFE level and a program scale, then the informational group
    /// with its own mix level and room type. Its header was measured to end 104 bits
    /// into a real broadcast stream, which is the number this asserts.
    #[test]
    fn the_mixing_metadata_of_a_broadcast_stream_leaves_the_audio_where_it_starts() {
        let stopped = stopped_at(
            &frame(7, true, true, |sink| {
                sink.put(1, 1) // mixmdate
                    .put(1, 2) // dmixmod
                    .put(1, 3) // ltrtcmixlev
                    .put(2, 3) // lorocmixlev
                    .put(3, 3) // ltrtsurmixlev
                    .put(0, 3) // lorosurmixlev
                    .put(1, 1) // lfemixlevcode
                    .put(7, 5) // lfemixlevcod
                    .put(0, 1) // pgmscle
                    .put(0, 1) // extpgmscle
                    .put(0, 2) // mixdef
                    .put(0, 1) // frmmixcfginfoe
                    .put(1, 1) // infomdate
                    .put(2, 3) // bsmod
                    .put(1, 1) // copyrightb
                    .put(1, 1) // origbs
                    .put(1, 2) // dsurexmod
                    .put(1, 1) // audprodie
                    .put(20, 5) // mixlevel
                    .put(1, 2) // roomtyp
                    .put(0, 1) // adconvtyp
                    .put(0, 1) // sourcefscod
                    .put(0, 1); // addbsie
            }),
            6,
        );
        assert_eq!(stopped, 104);
    }

    /// The most flexible mixing definition states its own length in bytes, and every
    /// scale factor and speech parameter in it lives inside that: a walk that reads
    /// the group as a total stays in step whatever the flags inside it say.
    #[test]
    fn an_external_mixing_group_fills_the_bytes_it_declares() {
        let stopped = stopped_at(
            &frame(2, false, false, |sink| {
                sink.put(1, 1) // mixmdate
                    .put(0, 1) // pgmscle
                    .put(0, 1) // extpgmscle
                    .put(3, 2) // mixdef: the flexible one
                    .put(5, 5) // mixdeflen: seven bytes from here
                    .put(1, 1) // mixdata2e
                    .put(1, 1) // premixcmpsel
                    .put(0, 1) // drcsrc
                    .put(3, 3) // premixcmpscl
                    .put(1, 1) // extpgmlscle
                    .put(6, 4) // extpgmlscl
                    .put(0, 1) // extpgmcscle
                    .put(1, 1) // extpgmrscle
                    .put(7, 4) // extpgmrscl
                    .put(0, 1) // extpgmlsscle
                    .put(0, 1) // extpgmrsscle
                    .put(0, 1) // extpgmlfescle
                    .put(0, 1) // dmixscle
                    .put(1, 1) // addche
                    .put(1, 1) // extpgmaux1scle
                    .put(4, 4) // extpgmaux1scl
                    .put(0, 1) // extpgmaux2scle
                    .put(1, 1) // mixdata3e
                    .put(3, 5) // spchdat
                    .put(1, 1) // addspchdate
                    .put(4, 5) // spchdat1
                    .put(2, 2) // spchan1att
                    .put(1, 1) // addspchdat1e
                    .put(5, 5) // spchdat2
                    .put(3, 3) // spchan2att
                    .put(0, 1) // frmmixcfginfoe
                    .put(0, 1) // infomdate
                    .put(0, 1); // addbsie
            }),
            2,
        );
        // The parameters filled the seven declared bytes to the last bit, and the
        // three flags behind them are the whole of what is left to read.
        assert_eq!(stopped, 115);
    }

    /// A six-block frame states its mixing configuration once per block, which is
    /// what the frame's block count - and not a fixed number - has to answer for.
    #[test]
    fn per_block_mixing_configurations_are_counted_per_block() {
        let stopped = stopped_at(
            &frame(2, false, false, |sink| {
                sink.put(1, 1) // mixmdate
                    .put(0, 1) // pgmscle
                    .put(0, 1) // extpgmscle
                    .put(0, 2) // mixdef
                    .put(1, 1) // frmmixcfginfoe
                    .put(1, 1) // blkmixcfginfoe of block 0
                    .put(3, 5) // blkmixcfginfo
                    .put(1, 1) // blkmixcfginfoe of block 1
                    .put(20, 5)
                    .put(0, 1) // the four blocks behind them state nothing
                    .put(0, 1)
                    .put(0, 1)
                    .put(0, 1)
                    .put(0, 1); // infomdate
                sink.put(0, 1); // addbsie
            }),
            2,
        );
        assert_eq!(stopped, 75);
    }
}


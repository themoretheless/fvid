//! Apple MACE: three bits of code to a sample, or to two, with nothing in the byte
//! saying which.
//!
//! MACE (Macintosh Audio Compression/Expansion) is the coding the Mac put on its sound
//! chips: a byte holds three codes of three, two and three bits, each code looks a step
//! size up in one of two tables, and the resulting delta is dropped through a one-tap
//! predictor whose level follows the signal. The 3-to-1 name counts against an 8-bit
//! sample, so one byte codes three samples; the 6-to-1 name codes six, because its
//! three codes each drive a two-tap interpolation that fills in a sample between the
//! predictor's own output and the next. Neither byte states which of the two it is -
//! only the container's fourcc does, and the two codings are separate descriptors
//! (`mace3`, `mace6`) with separate arithmetic, which is why this module holds both.
//!
//! What the coding states about itself, measured over this build's reference:
//!
//! - One block per channel is two bytes of 3-to-1 or one byte of 6-to-1, and both
//!   blocks code six samples - the container's `block_align` for the two codings, and
//!   the reason both run at the same 2.67 bits a sample.
//! - The output's two bytes always hold the same value. The reference's last step is
//!   `QT_8S_2_16S`, which takes a code's high byte and writes it into both halves of the
//!   sample, so a 16-bit MACE stream carries 8-bit resolution: measured over a whole
//!   take, all 257 028 samples have the two bytes equal, and 196 distinct byte values
//!   appear among them.
//! - The header's other numbers change nothing. Measured over that take: with
//!   `sampleLength` stating 1, 8 and 16 bits, and with `numSamplesFrames` stating 4,
//!   42 838 and 999 999, the reference returned the same 514 056 bytes each time - the
//!   run's length comes from the `SSND` payload, as it does for every coding this
//!   player reads. The channel count is the field that does have an effect, and two is
//!   the most the coding was built for: at three this build's reference refuses to open
//!   its decoder at all, and at two it re-reads the same bytes as two predictors, which
//!   is a different stream.
//!
//! Two traps are worth naming because both survive into a transcription unnoticed.
//! Codes 4 to 7 of a four-entry row are not out of range: the reference reads the row
//! backwards and returns `-1 - level`, so eight codes come out of four entries and a
//! port that clamps the code to the row loses the whole negative half of the signal -
//! 612 of the 3-to-1 codebook's 1 536 samples and 907 of the 6-to-1's are below zero.
//! And the clip on the way into the predictor is asymmetric by intent: the reference
//! calls it `mace_broken_clip_int16` and comments that it exists "to keep binary
//! identical output to the binary decoder", because its bottom branch answers -32 767
//! where a symmetric clamp would answer -32 768. Both branches are reached by the
//! byte-value runs this module is proved on (measured: 36 samples of one codebook and
//! 90 of the other come out at the clip's top, 31 and 162 at its bottom) and by nothing
//! of the Mac's own recorded material (measured: 0 of either take's samples reaches
//! either end, the loudest standing at 23 130 and -23 645), so the codebooks are where
//! the difference from a tidied clamp shows.
//!
//! The state a resumed stream needs is one predictor per channel - position in the level
//! table, the 6-to-1 coder's step adaptation, and the two past samples its interpolator
//! reads - and nothing about where a packet starts. That is what lets either container
//! cut the run into its own windows: measured, the whole take came back sample for sample
//! the same through the reference's 4 096-byte AIFF packets as through one decode of the
//! whole payload, the same again cut at any whole number of blocks, and the same a third
//! time as the one-block packets this player's `mov` index hands over - which is what
//! lets a `stsz` of 630 630 single-sample entries be read as 105 105 blocks without
//! either the decoder or the listener noticing.

use crate::audio::{AudioDecode, AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

include!("../../crates/fvid-media/src/owned_mace_impl.rs");

impl MaceDecoder {
    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate(),
            channels: self.channels(),
            format: SampleFormat::F32,
        }
    }
}

impl AudioDecode for MaceDecoder {
    /// Every block yields six samples per channel, so a packet comes back a packet and
    /// the timestamps stay the caller's.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        let planar = self.decode_planar(data)?;
        let frames = planar[0].len();
        let mut out = Vec::with_capacity(frames * usize::from(self.channels) * 4);
        for frame in 0..frames {
            for channel in 0..usize::from(self.channels) {
                let sample = f32::from(planar[channel][frame]) / 32_768.0;
                out.extend_from_slice(&sample.to_le_bytes());
            }
        }
        Ok(Some(AudioPacket {
            data: out,
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    /// Both predictors start again at the quiet end of the level table, as a decoder
    /// opened on the stream does.
    fn reset(&mut self) {
        MaceDecoder::reset(self);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Channel, LEVEL_NARROW, LEVEL_WIDE, MaceCoding, MaceDecoder, NARROW, SAMPLES_PER_BLOCK,
        STEP_NARROW, STEP_WIDE, WIDE, clip_int16, codes, quantize, slot_tables,
    };
    use crate::audio::AudioDecode;
    #[cfg(feature = "player")]
    use crate::audio::AudioStream;
    #[cfg(feature = "player")]
    use crate::container::mp4::Limits as Mp4Limits;
    #[cfg(feature = "player")]
    use crate::playback_mp4_audio::Mp4AudioReader;
    #[cfg(feature = "player")]
    use std::io::Cursor;

    /// The coded runs the decoder is proved against, and the samples this build's reference reads
    /// out of each. The 3-to-1 material is the first 512 blocks of the shipped Mac take
    /// `mac3audio.mov`, laid out as the coding wants them - as two separate runs for the mono
    /// file, and as one channel's bytes after the other's for the stereo one, which is the block
    /// layout that take has. The 6-to-1 material is the first 512 bytes of `mjpega.mov`, and its
    /// stereo file is that run again against the same run's bytes from 64 in, interleaved one
    /// channel's byte to a block. The two codebooks hold every byte value once, so every code of
    /// every table row is read: 3-to-1's pairs a byte with its complement to keep the three slots
    /// moving across the table. Each `.s16` beside them is what `ffmpeg -f s16le -i <file>` writes
    /// for that file, so no expectation here is computed the way the decoder computes it.
    const MAC3_MONO: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-mono.aiff");
    const MAC3_MONO_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-mono.s16");
    const MAC3_STEREO: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-stereo.aiff");
    const MAC3_STEREO_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-stereo.s16");
    const MAC6_MONO: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-mono.aiff");
    const MAC6_MONO_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-mono.s16");
    const MAC6_STEREO: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-stereo.aiff");
    const MAC6_STEREO_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-stereo.s16");
    const MAC3_CODEBOOK: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-codebook.aiff");
    const MAC3_CODEBOOK_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-codebook.s16");
    const MAC6_CODEBOOK: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-codebook.aiff");
    const MAC6_CODEBOOK_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mac6-codebook.s16");
    /// The 3-to-1 mono run with one byte standing over its last block: 512 whole blocks and a
    /// ninth of another, which is the shape a damaged file hands a decoder.
    const MAC3_TAIL: &[u8] = include_bytes!("../../tests/fixtures/mace/mac3-mono-tail.aiff");
    /// A Mac screen recording whose audio track is 6-to-1 at the one rate a Wave reader can also
    /// state: 2 976 blocks of one byte, 17 856 samples.
    const MJPEGA: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.mov");
    const MJPEGA_S16: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.s16");

    /// A fixture's coded run, found by walking its chunks to the `SSND` payload rather than by
    /// trusting an offset - except the trailing file, whose declared payload stops at the last
    /// whole byte of the last block plus the one that stands over.
    fn run(aiff: &[u8]) -> &[u8] {
        let mut at = 12;
        while at + 8 <= aiff.len() {
            let size = usize::try_from(u32::from_be_bytes(
                aiff[at + 4..at + 8].try_into().expect("four bytes"),
            ))
            .expect("chunk size");
            if &aiff[at..at + 4] == b"SSND" {
                return &aiff[at + 16..at + 16 + size - 8];
            }
            at += 8 + size + (size & 1);
        }
        panic!("the fixture carries no SSND chunk");
    }

    /// The reference's samples for a run, as the interleaved list the decoder returns.
    fn reference(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().expect("two bytes")))
            .collect()
    }

    /// Decode a whole run in one call, at the rate its own file states.
    fn decode(bytes: &[u8], coding: MaceCoding, channels: u16) -> Vec<i16> {
        let mut decoder = MaceDecoder::new(coding, 22_050, channels).expect("a coding at a rate");
        let packet = decoder
            .decode_encoded(bytes, 0, 0)
            .expect("decodes")
            .expect("a whole block always yields samples");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 22_050));
        assert_eq!(
            packet.data.len() / 4,
            bytes.len() / coding.block_bytes(channels) * SAMPLES_PER_BLOCK * usize::from(channels)
        );
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| {
                (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0).round()
                    as i16
            })
            .collect()
    }

    /// The same run decoded in windows of `blocks`, which is what a container that cuts the timeline
    /// its own way hands the decoder.
    fn decode_in_windows(
        bytes: &[u8],
        coding: MaceCoding,
        channels: u16,
        blocks: usize,
    ) -> Vec<i16> {
        let width = blocks * coding.block_bytes(channels);
        let mut decoder = MaceDecoder::new(coding, 22_050, channels).expect("mono");
        let mut heard = Vec::with_capacity(bytes.len() * SAMPLES_PER_BLOCK / 2);
        for window in bytes.chunks(width) {
            let packet = decoder
                .decode_encoded(window, 0, 0)
                .expect("decodes")
                .expect("samples");
            heard.extend(
                packet
                    .data
                    .chunks_exact(4)
                    .map(|chunk| {
                        (f32::from_le_bytes(chunk.try_into().expect("four bytes")) * 32_768.0)
                            .round() as i16
                    })
                    .collect::<Vec<i16>>(),
            );
        }
        heard
    }

    /// One channel of an interleaved list: every `channels`-th sample from `index` on.
    fn channel(samples: &[i16], index: usize, channels: usize) -> Vec<i16> {
        samples
            .iter()
            .skip(index)
            .step_by(channels)
            .copied()
            .collect()
    }

    #[test]
    fn three_to_one_reads_a_run_of_the_mac_s_own_bytes_bit_for_bit() {
        assert_eq!(
            decode(run(MAC3_MONO), MaceCoding::ThreeToOne, 1),
            reference(MAC3_MONO_S16),
            "512 two-byte blocks must give the reference's 3 072 samples"
        );
        assert_eq!(
            decode(run(MAC3_STEREO), MaceCoding::ThreeToOne, 2),
            reference(MAC3_STEREO_S16),
            "a stereo block is this channel's two bytes then that one's"
        );
        assert_eq!(
            decode(run(MAC3_CODEBOOK), MaceCoding::ThreeToOne, 1),
            reference(MAC3_CODEBOOK_S16)
        );
    }

    #[test]
    fn six_to_one_reads_the_same_bytes_as_twice_the_samples() {
        assert_eq!(
            decode(run(MAC6_MONO), MaceCoding::SixToOne, 1),
            reference(MAC6_MONO_S16),
            "one byte codes the six samples a 3-to-1 pair costs"
        );
        assert_eq!(
            decode(run(MAC6_STEREO), MaceCoding::SixToOne, 2),
            reference(MAC6_STEREO_S16)
        );
        assert_eq!(
            decode(run(MAC6_CODEBOOK), MaceCoding::SixToOne, 1),
            reference(MAC6_CODEBOOK_S16)
        );
        // Both codings count their samples the same way, which is what makes them the same
        // bit rate off different byte counts: 512 blocks of either width come out at 3 072.
        assert_eq!(
            decode(run(MAC3_MONO), MaceCoding::ThreeToOne, 1).len(),
            decode(run(MAC6_MONO), MaceCoding::SixToOne, 1).len()
        );
    }

    #[test]
    fn a_byte_s_three_slots_come_out_in_the_order_each_coding_reads_them() {
        // 3-to-1 takes the byte from its low end, 6-to-1 from its top, and the two-bit code sits
        // between them in both: the same eight bits, read backwards.
        for byte in [0u8, 1, 0b111_11_111, 0b101_10_010, 0x2a, 0xff] {
            let three = codes(byte, MaceCoding::ThreeToOne);
            assert_eq!(
                three,
                [
                    usize::from(byte & 7),
                    usize::from((byte >> 3) & 3),
                    usize::from(byte >> 5)
                ]
            );
            assert_eq!(
                codes(byte, MaceCoding::SixToOne),
                [three[2], three[1], three[0]],
                "{byte:#04x}"
            );
            // Which slot a code stands in decides the table, and so the number of codes it holds:
            // the middle slot is the two-bit one both codings agree on.
            assert_eq!(slot_tables(1).steps, &STEP_NARROW[..], "slot {byte:#04x}");
            assert_eq!(slot_tables(0).steps, &STEP_WIDE[..]);
            assert_eq!(slot_tables(2).steps, &STEP_WIDE[..]);
            assert_eq!((slot_tables(0).stride, slot_tables(1).stride), (4, 2));
        }
    }

    #[test]
    fn a_row_of_four_entries_answers_eight_codes_by_reading_itself_backwards() {
        // The mirror branch is the port's easiest loss: clamp a code to the row and the whole
        // negative half of the signal disappears, because entries 4 to 7 of a row are not stored
        // anywhere - they are the row's own first entries inverted and thrown away a unit.
        let mut cold = Channel::default();
        assert_eq!(cold.read_table(0, &WIDE), LEVEL_WIDE[0]);
        assert_eq!(cold.read_table(3, &WIDE), LEVEL_WIDE[3]);
        let mut mirrored = Channel::default();
        assert_eq!(mirrored.read_table(4, &WIDE), -1 - LEVEL_WIDE[3]);
        let mut quiet = Channel::default();
        assert_eq!(quiet.read_table(7, &WIDE), -1 - LEVEL_WIDE[0]);
        let mut narrow_code2 = Channel::default();
        assert_eq!(narrow_code2.read_table(2, &NARROW), -1 - LEVEL_NARROW[1]);
        let mut narrow_code3 = Channel::default();
        assert_eq!(narrow_code3.read_table(3, &NARROW), -1 - LEVEL_NARROW[0]);
        // Both tables' steps are symmetric about their rows' middles, which is what makes the
        // mirrored codes pull the position the same way their positive twins push it.
        assert_eq!(&STEP_WIDE, &[-13, 8, 76, 222, 222, 76, 8, -13]);
        assert_eq!(&STEP_NARROW, &[-18, 140, 140, -18]);
    }

    #[test]
    fn the_table_position_follows_the_step_and_loses_a_thirty_second_of_itself() {
        // The `>> 5` the reference subtracts on every read is what lets a run of quiet codes bring
        // a loud passage down without any code naming a level, so a port that keeps only the step
        // holds a loud signal forever. The arithmetic is checked against the hand-computed
        // sequence, not against the code: 0 + 222 - 0, then 222 + 222 - 6, then 438 - 13 - 13.
        let mut channel = Channel::default();
        assert_eq!(channel.position, 0);
        channel.read_table(3, &WIDE);
        assert_eq!(channel.position, 222);
        channel.read_table(3, &WIDE);
        assert_eq!(channel.position, 438);
        // Two rows down by then, and a step back towards the quiet end of the table.
        channel.read_table(0, &WIDE);
        assert_eq!(channel.position, 412);
        // A code of the row's last entry pulls harder back than the first, and a position below
        // one row's width clamps at zero rather than wrapping into the loud rows.
        let mut quiet = Channel::default();
        quiet.position = 8;
        quiet.read_table(7, &WIDE);
        assert_eq!(quiet.position, 0);
    }

    #[test]
    fn the_output_is_eight_bits_wide_in_a_sixteen_bit_sample() {
        // The reference's last step copies a value's high byte into both halves of the sample, so
        // a 16-bit MACE stream carries 8-bit resolution. Measured across both codings' takes: 184
        // distinct high bytes in the 3-to-1 run's 3 072 samples and 243 in the 6-to-1 codebook's
        // 1 536, with every low byte a copy of its own.
        for take in [
            decode(run(MAC3_MONO), MaceCoding::ThreeToOne, 1),
            decode(run(MAC6_MONO), MaceCoding::SixToOne, 1),
            decode(run(MAC3_CODEBOOK), MaceCoding::ThreeToOne, 1),
            decode(run(MAC6_CODEBOOK), MaceCoding::SixToOne, 1),
        ] {
            assert!(
                take.iter()
                    .all(|sample| i32::from(*sample) & 0xff == (i32::from(*sample) >> 8) & 0xff),
                "{} samples of a byte-replicated stream",
                take.len()
            );
        }
        assert_eq!(quantize(0x12_34), 0x12_12);
        assert_eq!(quantize(-1), -1);
        assert_eq!(quantize(0x7f_80), 0x7f_7f);
    }

    #[test]
    fn the_codebooks_drive_the_negative_half_of_both_tables() {
        // A run that never left silence would pass with the mirror branch cut, so the counts here
        // are of the reference's own samples: 612 of the 3-to-1 codebook's 1 536 and 907 of the
        // 6-to-1 codebook's 1 536 are below zero.
        let three = reference(MAC3_CODEBOOK_S16);
        assert_eq!(three.len(), 256 * SAMPLES_PER_BLOCK);
        assert_eq!(three.iter().filter(|s| **s < 0).count(), 612);
        let six = reference(MAC6_CODEBOOK_S16);
        assert_eq!(six.iter().filter(|s| **s < 0).count(), 907);
        assert_eq!(decode(run(MAC3_CODEBOOK), MaceCoding::ThreeToOne, 1), three);
        assert_eq!(decode(run(MAC6_CODEBOOK), MaceCoding::SixToOne, 1), six);
    }

    #[test]
    fn the_clip_into_the_predictor_keeps_the_mac_binary_s_uneven_bottom_end() {
        // The reference calls this `mace_broken_clip_int16` and comments that it exists "to keep
        // binary identical output to the binary decoder". Its two ends are not twins: a value
        // above full scale becomes 32 767, a value below the sample's own bottom becomes -32 767,
        // and the bottom value itself -32 768 is the one input the comparison lets through
        // untouched. A symmetric clamp puts a different sample on the wire at that one value.
        assert_eq!(clip_int16(40_000), 32_767);
        assert_eq!(clip_int16(32_768), 32_767);
        assert_eq!(clip_int16(32_767), 32_767);
        assert_eq!(clip_int16(-40_000), -32_767);
        assert_eq!(clip_int16(-32_769), -32_767);
        assert_eq!(clip_int16(-32_768), -32_768);
        assert_eq!(clip_int16(-32_767), -32_767);
        assert_eq!(clip_int16(0), 0);
    }

    #[test]
    fn the_codebooks_run_the_predictor_onto_both_ends_of_its_range_and_the_takes_do_not() {
        // The clip's two ends are visible in the output as the two byte-replicated values they
        // quantize down to: 32 639 is what a step that landed at or above full scale sounds like,
        // -32 640 what one at or below the sample's bottom sounds like. Measured over the
        // reference's own samples, the run of every byte value reaches both - 36 and 31 times in
        // the 3-to-1 codebook's 1 536 samples, 90 and 162 in the 6-to-1's - and none of the four
        // Mac takes or the screen recording reaches either, whose loudest samples stand at 23 130
        // and -23 645.
        let ends = |take: &[i16]| {
            (
                take.iter().filter(|s| **s == 32_639).count(),
                take.iter().filter(|s| **s == -32_640).count(),
            )
        };
        assert_eq!(ends(&reference(MAC3_CODEBOOK_S16)), (36, 31));
        assert_eq!(ends(&reference(MAC6_CODEBOOK_S16)), (90, 162));
        for oracle in [
            MAC3_MONO_S16,
            MAC3_STEREO_S16,
            MAC6_MONO_S16,
            MAC6_STEREO_S16,
            MJPEGA_S16,
        ] {
            assert_eq!(ends(&reference(oracle)), (0, 0));
        }
        // The bottom end is the one a tidy port loses: measured here, making the clip symmetric -
        // returning -32 768 where the reference returns -32 767 - moves the 3-to-1 codebook's
        // samples, because what the clip hands back is also what the predictor carries forward.
        assert_eq!(
            decode(run(MAC3_CODEBOOK), MaceCoding::ThreeToOne, 1),
            reference(MAC3_CODEBOOK_S16),
            "the uneven bottom must be the one the reference answers with"
        );
    }

    #[test]
    fn two_channels_hold_their_own_predictors_and_come_out_as_two_runs() {
        // The block interleaves bytes, the coding does not interleave state: 3-to-1's stereo file
        // holds one channel's run then the other's twice as wide, and both planes of it are the
        // mono take, so both of its channels must read as that take alone.
        let stereo = decode(run(MAC3_STEREO), MaceCoding::ThreeToOne, 2);
        let mono = decode(run(MAC3_MONO), MaceCoding::ThreeToOne, 1);
        assert_eq!(stereo.len(), mono.len() * 2);
        assert_eq!(channel(&stereo, 0, 2), mono);
        assert_eq!(channel(&stereo, 1, 2), mono);
        // The 6-to-1 pair is deliberately two different runs - the take's own bytes from its start
        // and from byte 64 - so a shared predictor shows up as a wrong sample rather than twice
        // the same right one.
        let six = decode(run(MAC6_STEREO), MaceCoding::SixToOne, 2);
        let first = &run(MAC6_MONO)[..192];
        let second = &run(MAC6_MONO)[64..256];
        assert_eq!(channel(&six, 0, 2), decode(first, MaceCoding::SixToOne, 1));
        assert_eq!(channel(&six, 1, 2), decode(second, MaceCoding::SixToOne, 1));
        assert_eq!(six, reference(MAC6_STEREO_S16));
    }

    #[test]
    fn where_a_packet_is_cut_changes_no_sample() {
        // Nothing in the coding says where a run's units begin, so a container may hand it over in
        // any whole number of blocks and the stream comes out the same - the reference does it in
        // windows of up to 1 020 samples, this player does it one block a packet, and the two
        // agree. 341 blocks is one of this player's own AIFF windows: 682 bytes for 2 046 frames.
        let bytes = run(MAC3_MONO);
        let whole = decode(bytes, MaceCoding::ThreeToOne, 1);
        for blocks in [1, 7, 341, 511] {
            assert_eq!(
                decode_in_windows(bytes, MaceCoding::ThreeToOne, 1, blocks),
                whole,
                "{blocks} blocks a window"
            );
        }
        let six = run(MAC6_MONO);
        let six_whole = decode(six, MaceCoding::SixToOne, 1);
        for blocks in [1, 682, 511] {
            assert_eq!(
                decode_in_windows(six, MaceCoding::SixToOne, 1, blocks),
                six_whole,
                "{blocks} blocks a window"
            );
        }
        // A stereo run cut across its blocks' channel planes is the same test again with the
        // other channel's bytes in the way.
        assert_eq!(
            decode_in_windows(run(MAC3_STEREO), MaceCoding::ThreeToOne, 2, 341),
            reference(MAC3_STEREO_S16)
        );
    }

    #[test]
    fn a_run_that_is_not_a_whole_number_of_blocks_is_refused() {
        // 3-to-1's block is two bytes a channel and 6-to-1's one, so only the stereo pair can show
        // a 6-to-1 remainder at all: the tail fixture's standing-over byte, and half a block.
        let tail = run(MAC3_TAIL);
        assert_eq!(tail.len(), 1_025);
        for (bytes, coding, channels) in [
            (tail, MaceCoding::ThreeToOne, 1),
            (&tail[..3], MaceCoding::ThreeToOne, 2),
            (&run(MAC6_STEREO)[..3], MaceCoding::SixToOne, 2),
        ] {
            let error = MaceDecoder::new(coding, 22_050, channels)
                .expect("a coding")
                .decode_encoded(bytes, 0, 0)
                .err()
                .unwrap_or_else(|| {
                    panic!(
                        "a {}-byte packet of {coding:?} blocks is refused",
                        bytes.len()
                    )
                });
            assert!(
                error.to_string().contains(&format!(
                    "not a whole number of {}-byte",
                    coding.block_bytes(channels)
                )),
                "{coding:?} {channels}: {error}"
            );
        }
        // A packet of no blocks at all is the other side of the same arithmetic: nothing left over
        // to refuse, and nothing to hear.
        let mut decoder = MaceDecoder::new(MaceCoding::ThreeToOne, 22_050, 1).expect("mono");
        assert_eq!(
            decoder
                .decode_encoded(&[], 0, 0)
                .expect("empty decodes")
                .map(|packet| packet.data.len()),
            Some(0)
        );
    }

    #[test]
    fn a_third_channel_and_a_track_with_no_rate_are_refused_at_the_construction() {
        // The coding was built for the Mac's sound chip: two predictors, and no field of a byte
        // says where a third would read. Measured, this build's reference does not open its
        // decoder at all on a three-channel track.
        for channels in [0u16, 3, 8] {
            let error = MaceDecoder::new(MaceCoding::ThreeToOne, 22_050, channels)
                .err()
                .unwrap_or_else(|| panic!("{channels} channels refused"));
            assert!(error.to_string().contains("one or two channels"), "{error}");
        }
        let error = MaceDecoder::new(MaceCoding::SixToOne, 0, 1)
            .err()
            .expect("a track with no sample rate is refused");
        assert!(error.to_string().contains("no sample rate"), "{error}");
    }

    #[test]
    fn a_seek_restarts_both_predictors_from_the_quiet_end_of_the_table() {
        let bytes = run(MAC6_MONO);
        let mut decoder = MaceDecoder::new(MaceCoding::SixToOne, 8_000, 2).expect("stereo");
        let first = decoder
            .decode_encoded(&bytes[..64], 0, 0)
            .expect("decodes")
            .expect("samples");
        decoder
            .decode_encoded(&bytes[64..], 0, 0)
            .expect("the rest decodes");
        decoder.reset();
        let again = decoder
            .decode_encoded(&bytes[..64], 0, 0)
            .expect("decodes")
            .expect("samples");
        assert_eq!(first.data, again.data, "reset must be a cold decoder");
        let mut cold = MaceDecoder::new(MaceCoding::SixToOne, 8_000, 2).expect("stereo");
        assert_eq!(
            cold.decode_encoded(&bytes[..64], 0, 0)
                .expect("decodes")
                .expect("samples")
                .data,
            again.data
        );
    }

    #[test]
    #[cfg(feature = "player")]
    fn a_mac_take_is_indexed_in_blocks_and_read_one_a_packet() {
        // `stsz` counts this track's samples, not its packets: the file states 17 856 of them at
        // one byte each where the coding packs six samples into each byte. So the reader's grain
        // is a block, the track's length is the same 17 856 samples, and the packets are the
        // blocks - which is what the reference reports for the same file.
        let mut reader = Mp4AudioReader::open(Cursor::new(MJPEGA), Mp4Limits::default())
            .expect("the take opens");
        assert_eq!(reader.codec(), "mace6");
        assert_eq!((reader.sample_rate(), reader.channels()), (8_000, 1));
        let mut heard = Vec::new();
        let mut packets = 0;
        while let Some(packet) = reader.next_packet().expect("packet") {
            assert_eq!(packet.data.len(), 1, "one block a packet");
            assert_eq!(packet.duration, SAMPLES_PER_BLOCK as i64);
            heard.extend_from_slice(&packet.data);
            packets += 1;
        }
        assert_eq!(packets, 2_976);
        assert_eq!(
            decode(&heard, MaceCoding::SixToOne, 1),
            reference(MJPEGA_S16),
            "2 976 single-byte packets must give the reference's 17 856 samples"
        );
    }
}

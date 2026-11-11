//! Indexed ISO BMFF demultiplexing implemented in FVid.
pub use fvid_media::owned_mp4::*;

#[cfg(test)]
mod tests {
    /// A Mac screen recording whose audio track is MACE 6-to-1 mono at 8 000 Hz: `stsz` counts
    /// 17 856 samples of one byte each in one `stts` run of one tick, `stsc` lays them out as
    /// two chunks of 12 252 and 5 604, and `stco` puts them at 633 536 and 886 874 inside an
    /// `mdat` that ends at 887 808. Every one of those numbers is the file's own.
    #[cfg(feature = "player")]
    const MJPEGA: &[u8] = include_bytes!("../../tests/fixtures/mace/mjpega.mov");
    /// Where the take states the four of those numbers a lying file can get wrong: the
    /// `stts` run and the `stsz` count that both name 17 856 samples, the `stsc` entry that
    /// names a chunk of 12 252 of them, and the `stco` offset the last chunk starts at.
    #[cfg(feature = "player")]
    const STTS_COUNT: usize = 889_166;
    #[cfg(feature = "player")]
    const STSZ_COUNT: usize = 889_230;
    #[cfg(feature = "player")]
    const STSC_PER_CHUNK: usize = 889_194;
    #[cfg(feature = "player")]
    const STCO_SECOND: usize = 889_254;

    /// The take with its big-endian table fields rewritten.
    #[cfg(feature = "player")]
    fn stating(patches: &[(usize, u32)]) -> Vec<u8> {
        let mut bytes = MJPEGA.to_vec();
        for (at, value) in patches {
            bytes[*at..*at + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }


    #[cfg(feature = "player")]
    fn open(
        bytes: &[u8],
    ) -> crate::Result<crate::playback_mp4_audio::Mp4AudioReader<std::io::Cursor<&[u8]>>> {
        crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(bytes),
            super::Limits::default(),
        )
    }


    #[cfg(feature = "player")]
    #[test]
    #[cfg(feature = "player")]
    fn a_mace_track_is_indexed_in_blocks_though_its_table_counts_samples() {
        use crate::audio::AudioStream as _;
        // The reader's grain is a block, so the packets are the 2 976 of them the 17 856 samples
        // hold - one byte and six ticks each, stamped end to end over the run `stsz` counts. The
        // reference reads the same tables into packets of up to 1 020 samples, and both come out
        // the same samples: the coding carries nothing about where a packet starts.
        let mut reader = open(MJPEGA).expect("the take opens");
        let mut packets = 0;
        let mut ticks = 0i64;
        while let Some(packet) = reader.next_packet().expect("packet") {
            assert_eq!(packet.data.len(), 1, "one block a packet");
            assert_eq!(packet.duration, 6, "six samples a block");
            assert_eq!(packet.pts, ticks, "the stamps follow the run");
            ticks += packet.duration;
            packets += 1;
        }
        assert_eq!(packets, 2_976);
        assert_eq!(ticks, 17_856, "as many samples as `stsz` counts");
    }

    #[cfg(feature = "player")]
    #[test]
    #[cfg(feature = "player")]
    fn a_mace_table_that_does_not_divide_into_blocks_is_refused() {
        // The three ways the geometry can lie, each caught before a packet is read: a sample
        // count that is not a whole number of six-sample blocks, a chunk that is not either,
        // and a chunk whose blocks would run past the `mdat` its offset names.
        // Both fields that count the run have to say 17 855, or the two tables disagree with
        // each other and that is what gets noticed first.
        let error = open(&stating(&[(STTS_COUNT, 17_855), (STSZ_COUNT, 17_855)]))
            .err()
            .expect("17 855 samples are not a whole number of blocks");
        assert!(
            error.to_string().contains("whole number of blocks"),
            "{error}"
        );
        let error = open(&stating(&[(STSC_PER_CHUNK, 12_253)]))
            .err()
            .expect("a chunk of 12 253 samples is not a whole number of blocks");
        assert!(
            error.to_string().contains("sample-to-chunk"),
            "{error}: {error}"
        );
        let error = open(&stating(&[(STCO_SECOND, 886_875)]))
            .err()
            .expect("the last chunk must stay inside the mdat");
        assert!(error.to_string().contains("outside mdat"), "{error}");
        // Sizing the run the way its `stsz` entries spell it - one byte each, 17 856 of them,
        // laid out as the table says - is exactly the same data, so the refusal is the block
        // reading having noticed something the per-sample reading could not.
        assert!(open(MJPEGA).is_ok());
    }


    /// The two fragmented takes this reader was written against, each exactly as
    /// the command above its own path in `tests/fixtures/fragmented` reproduces
    /// it. One is mono AAC at 48 kHz, indexed by a single `moof`; the other is
    /// H.264 with B-frames, indexed by five, each pointing into its own `mdat`.
    #[cfg(feature = "player")]
    const FRAG_AUDIO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/audio.mp4");
    const FRAG_VIDEO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/video.mp4");


    #[cfg(feature = "player")]
    #[test]
    fn the_sound_a_fragment_indexes_is_the_sound_the_player_hears() {
        // The route a file takes to the speaker, not just the index behind it:
        // the audio reader opens the fragmented take the same way it opens an
        // indexed one, the decoder the dispatch answers accepts its packets, and
        // each of the first twenty comes back as one raw block of mono sound.
        use crate::audio::AudioStream as _;
        let mut reader = open(FRAG_AUDIO).expect("the take opens for playback");
        assert_eq!(reader.codec(), "mp4a");
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::new(1, 221_333_333))
        );
        let mut decoder = crate::codec::make_audio_decoder(
            reader.codec(),
            reader.extra_data(),
            reader.sample_rate(),
            reader.channels(),
            reader.bits_per_sample(),
        )
        .expect("the dispatch answers this tag");
        let mut stamps = Vec::new();
        for _ in 0..20 {
            let packet = reader.next_packet().expect("packet").expect("sample");
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            assert_eq!(
                audio.data.len() / 4,
                1_024,
                "one raw block of interleaved f32 mono"
            );
            assert_eq!((audio.pts, audio.timebase_den), (packet.pts as u64, 48_000));
            stamps.push((packet.pts, packet.duration));
        }
        assert_eq!(stamps[0], (0, 1_024));
        assert_eq!(stamps[19], (19_456, 1_024), "the stamps are the rows' own");
    }

    #[test]
    fn the_picture_a_fragment_indexes_reaches_the_picture_route() {
        // The container is only half of a file playing: the reader the window
        // drives asks the same index for packets and reorders what it decodes by
        // the display times the rows carried. Five fragments of five rows each,
        // with no `stss` and no `ctts` anywhere in the take, is the case that
        // would quietly show B-frames in decode order if the shift were lost.
        let mut reader = crate::playback_mp4::Mp4VideoReader::open(
            std::io::Cursor::new(FRAG_VIDEO),
            super::Limits::default(),
            // What the reader splits between the references its decoder keeps and
            // the pictures it holds for reordering - which 320x240 with four
            // reference frames does not fit into at a megabyte.
            32 << 20,
        )
        .expect("the picture route opens the take");
        let mut shown = Vec::new();
        for _ in 0..5 {
            let frame = reader.read_frame().expect("frame").expect("picture");
            shown.push((
                frame.presentation_time.ticks,
                frame.duration.ticks,
                frame.sample_index,
                frame.picture.coded_width,
                frame.picture.coded_height,
                frame.picture.bit_depth,
            ));
        }
        assert_eq!(
            shown,
            vec![
                (1_024, 512, 0, 320, 240, 8),
                (1_536, 512, 3, 320, 240, 8),
                (2_048, 512, 2, 320, 240, 8),
                (2_560, 512, 4, 320, 240, 8),
                (3_072, 512, 1, 320, 240, 8),
            ],
            "display order, from a file whose rows said only decode order"
        );
    }

}

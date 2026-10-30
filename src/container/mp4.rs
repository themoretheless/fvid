//! Indexed ISO BMFF demultiplexing implemented in FVid, over both the one
//! complete sample table a non-fragmented file writes and the run of `moof`
//! fragments a stream-derived file writes in its place.
//! Timestamps are in the track media timeline; edit lists are returned separately.
use crate::color::hdr::{ColourDescription, HdrMetadata};
use crate::{container::FileTags, invalid, unsupported, Result};
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use crate::buffer;
use super::reduce_ratio;
use crate::codec::config::{aac_specific_config as mp4_aac_specific_config,AudioSpecificConfig as Mp4AudioSpecificConfig};
include!("../../crates/fvid-media/src/owned_mp4_impl.rs");

#[cfg(test)]
mod tests {
    use super::colr_description;
    use super::read_tags;
    use super::ColourDescription;
    use crate::container::FileTags;

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).unwrap().to_be_bytes();
        [&size[..], kind, payload].concat()
    }
    /// The iTunes `data` box, whose text stands behind a stated encoding and
    /// four bytes no writer explains.
    fn data(text: &[u8]) -> Vec<u8> {
        atom(b"data", &[&[0, 0, 0, 1, 0, 0, 0, 0][..], text].concat())
    }
    /// A Microsoft ADPCM entry: the fixed part every audio description carries,
    /// then the sixteen bytes of packet geometry a version 1 one adds, then the
    /// `wave` atom holding the `WAVEFORMATEX` the block length comes from.
    #[test]
    fn the_wave_record_sits_where_the_entry_version_puts_the_children() {
        use super::{is_adpcm, wave_record};
        let record: Vec<u8> = (0..18u8).collect();
        let wave = atom(b"wave", &atom(b"ms\x00\x02", &record));
        let mut entry = vec![0u8; 28];
        entry[8..10].copy_from_slice(&1u16.to_be_bytes());
        entry.extend_from_slice(&[255u8; 16]);
        entry.extend_from_slice(&wave);
        assert_eq!(
            wave_record(&entry, 44, *b"ms\x00\x02").as_deref(),
            Some(&record[..]),
            "a version 1 description states its own geometry before the atoms"
        );
        let mut plain = vec![0u8; 28];
        plain.extend_from_slice(&wave);
        assert_eq!(
            wave_record(&plain, 28, *b"ms\x00\x02").as_deref(),
            Some(&record[..]),
            "version 0 goes straight from the sample rate to them"
        );
        // Apple's entry carries a `chan` atom instead and needs no record; one
        // that is looked for in it comes back absent, which the decoder reads as
        // a track it has no geometry for rather than a length of its own.
        let mut ima4 = vec![0u8; 28];
        ima4[8..10].copy_from_slice(&1u16.to_be_bytes());
        ima4.extend_from_slice(&[255u8; 16]);
        ima4.extend_from_slice(&atom(b"chan", &[0u8; 16]));
        assert_eq!(wave_record(&ima4, 44, *b"ms\x00\x02"), None);
        assert!(is_adpcm(b"ima4") && is_adpcm(b"ms\x00\x11") && !is_adpcm(b"mp4a"));
    }

    #[test]
    fn colr_reads_both_writers_spellings_of_the_cicp_triple() {
        // BT.2020 primaries, PQ transfer, BT.2020 NCL matrix, full-range flag
        // in the top bit of the byte following the three 16-bit codes.
        let nclx = [
            b"nclx"[..].to_vec(),
            vec![0, 9, 0, 16, 0, 9, 0x80],
        ]
        .concat();
        assert_eq!(
            colr_description(&nclx),
            Some(ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            })
        );
        let narrow = {
            let mut v = nclx.clone();
            v[10] = 0x00;
            v
        };
        assert_eq!(colr_description(&narrow).map(|c| c.full_range), Some(false));
        // QuickTime's older spelling has no version header and no range at all.
        let mut nclc = b"nclc"[..].to_vec();
        nclc.extend_from_slice(&[0, 1, 0, 1, 0, 1]);
        assert_eq!(
            colr_description(&nclc),
            Some(ColourDescription {
                primaries: 1,
                transfer: 1,
                matrix: 1,
                full_range: false,
            })
        );
        let mut wide_code = nclx.clone();
        wide_code[4] = 1;
        assert_eq!(colr_description(&wide_code), None, "0x0109 must not alias BT.2020");
        // An ICC profile states colour another way and is not guessed at here.
        assert_eq!(colr_description(b"rICC\x00\x00\x00\x00"), None);
        // Truncated atoms of either spelling are refused, not read past.
        for len in 0..nclc.len() {
            assert_eq!(colr_description(&nclc[..len]), None, "{len} bytes");
        }
        for len in 0..nclx.len() {
            assert_eq!(colr_description(&nclx[..len]), None, "{len} bytes");
        }
    }
    /// Every fact the movie can be named by, in the spelling a QuickTime file
    /// states it — text standing in the tag itself — and in the one an
    /// iTunes-style file nests it in.
    #[test]
    fn the_movie_tags_name_it_in_either_writers_spelling() {
        let plain = [
            atom(b"\xa9nam", "Имя".as_bytes()),
            atom(b"\xa9ART", b"A"),
            atom(b"\xa9alb", b"B"),
            atom(b"\xa9gen", b"G"),
            atom(b"\xa9day", b"2026"),
            atom(b"\xa9cmt", b"C"),
        ];
        assert_eq!(
            read_tags(Some(&plain.concat())),
            FileTags {
                title: "Имя".to_owned(),
                artist: "A".to_owned(),
                album: "B".to_owned(),
                genre: "G".to_owned(),
                date: "2026".to_owned(),
                comment: "C".to_owned(),
                track: String::new(),
                album_artist: String::new(),
                disc: String::new(),
                publisher: String::new(),
                copyright: String::new(),
                description: String::new(),
                rating: String::new(),
            }
        );
        // A name the writer ended with a NUL byte is the name without it.
        assert_eq!(read_tags(Some(&atom(b"\xa9nam", b"Ok\0"))).title, "Ok");
        let nested = atom(
            b"meta",
            &[
                &[0, 0, 0, 0][..],
                &atom(b"ilst", &atom(b"\xa9nam", &data(b"Ok"))),
            ]
            .concat(),
        );
        assert_eq!(read_tags(Some(&nested)).title, "Ok");
        // Where a file states one fact both ways, the plain box answers.
        let both = [atom(b"\xa9nam", b"plain"), nested].concat();
        assert_eq!(read_tags(Some(&both)).title, "plain");
    }
    /// A file this reader finds no facts in states nothing: no `udta` at all, a
    /// tag of something else, a value in bytes no UTF-8 text spells, and a tag
    /// with nothing behind it all answer the same way.
    #[test]
    fn tags_this_reader_cannot_read_leave_the_file_unstated() {
        assert_eq!(read_tags(None), FileTags::default());
        // The writer's own name is no fact about the file it tags.
        assert_eq!(
            read_tags(Some(&atom(b"\xa9too", b"Lavf"))),
            FileTags::default()
        );
        assert_eq!(read_tags(Some(&atom(b"\xa9nam", &[]))), FileTags::default());
        assert_eq!(
            read_tags(Some(&atom(b"\xa9nam", &[0xff, 0xfe]))),
            FileTags::default()
        );
        let empty = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &atom(b"\xa9nam", &[]))].concat(),
        );
        assert_eq!(read_tags(Some(&empty)), FileTags::default());
    }

    /// Who holds the rights and what the file says of itself reach the panel
    /// beside the rest. `ffmpeg` files both under iTunes-style names with no
    /// `©` mark — `cprt` and `desc` — and this reader asks the one table for
    /// them in both of a file's writings: wrapped in a `data` box under
    /// `meta/ilst`, the way the muxer leaves them, and standing as a plain box
    /// under `udta`, the way an older writer might.
    #[test]
    fn the_rights_and_the_note_read_from_their_atoms_both_ways() {
        let nested = atom(
            b"meta",
            &[
                &[0, 0, 0, 0][..],
                &atom(
                    b"ilst",
                    &[
                        atom(b"cprt", &data(b"Right")),
                        atom(b"desc", &data(b"Note")),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        );
        let tags = read_tags(Some(&nested));
        assert_eq!(
            (tags.copyright.as_str(), tags.description.as_str()),
            ("Right", "Note")
        );
        let plain = [atom(b"cprt", b"R"), atom(b"desc", b"D")].concat();
        let tags = read_tags(Some(&plain));
        assert_eq!(
            (tags.copyright.as_str(), tags.description.as_str()),
            ("R", "D")
        );
    }

    /// A track number is the one tag whose value is not text: two sixteen-bit
    /// numbers behind two reserved bytes, which QuickTime writes as a bare box
    /// under `udta` and ffmpeg's iTunes layout wraps in a `data` box, and both
    /// ways land in the panel's `Track` line — the total only when the file
    /// knows it. A box whose number is zero states no place, however full the
    /// rest of it is, and one too short for the pair states nothing at all.
    #[test]
    fn a_track_number_reads_from_its_binary_box_both_writers_spell_it() {
        fn stated(number: u16, total: u16, short: bool) -> Vec<u8> {
            let mut bytes = vec![0, 0];
            bytes.extend(number.to_be_bytes());
            if !short {
                bytes.extend(total.to_be_bytes());
                bytes.extend([0; 6]);
            }
            bytes
        }
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(3, 12, false)))).track,
            "3/12"
        );
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(7, 0, false)))).track,
            "7"
        );
        let inside = atom(
            b"trkn",
            &atom(
                b"data",
                &[&[0; 8][..], stated(2, 9, false).as_slice()].concat(),
            ),
        );
        let list = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &inside)].concat(),
        );
        assert_eq!(read_tags(Some(&list)).track, "2/9");
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(0, 12, false)))),
            FileTags::default(),
            "a zero place is no place"
        );
        assert_eq!(
            read_tags(Some(&atom(b"trkn", &stated(3, 12, true)))),
            FileTags::default(),
            "a box cut short states nothing"
        );
    }

    /// The disc's place travels in the same binary pair as the track's, under
    /// its own `disk` key, and the album's artist is plain text under `aART` —
    /// ffmpeg writes both ways, the bare QuickTime box and the `data`-wrapped
    /// iTunes one, and both reach the panel's `Disc` and `Album artist` lines.
    #[test]
    fn a_disc_and_the_albums_own_artist_read_from_their_boxes() {
        fn stated(number: u16, total: u16) -> Vec<u8> {
            let mut bytes = vec![0, 0];
            bytes.extend(number.to_be_bytes());
            bytes.extend(total.to_be_bytes());
            bytes.extend([0; 6]);
            bytes
        }
        let file = [atom(b"disk", &stated(2, 10)), atom(b"aART", b"AA")].concat();
        let tags = read_tags(Some(&file));
        assert_eq!(tags.disc, "2/10");
        assert_eq!(tags.album_artist, "AA");
        let inside = atom(
            b"disk",
            &atom(b"data", &[&[0; 8][..], stated(4, 0).as_slice()].concat()),
        );
        let list = atom(
            b"meta",
            &[&[0, 0, 0, 0][..], &atom(b"ilst", &inside)].concat(),
        );
        assert_eq!(read_tags(Some(&list)).disc, "4");
    }

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

    use super::{is_mace, mace_block};

    #[cfg(feature = "player")]
    fn open(
        bytes: &[u8],
    ) -> crate::Result<crate::playback_mp4_audio::Mp4AudioReader<std::io::Cursor<&[u8]>>> {
        crate::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(bytes),
            super::Limits::default(),
        )
    }

    #[test]
    fn the_coding_s_block_is_two_or_one_bytes_a_channel_and_six_samples_whichever_it_is() {
        assert_eq!(mace_block(b"MAC3", 1), Some((6, 2)));
        assert_eq!(mace_block(b"MAC3", 2), Some((6, 4)));
        assert_eq!(mace_block(b"MAC6", 1), Some((6, 1)));
        assert_eq!(mace_block(b"MAC6", 2), Some((6, 2)));
        // Every other coding's `stsz` entry is the frame it names, blocks included: a track of
        // any of them goes on being indexed per sample as it was before this arm existed.
        for codec in [*b"twos", *b"MAC0", *b"lpcm", *b"sowt", *b"mp4a"] {
            assert_eq!(mace_block(&codec, 2), None, "{:?} is not MACE", codec);
        }
        assert!(is_mace(b"MAC3") && is_mace(b"MAC6") && !is_mace(b"MAC0"));
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
    const FRAG_AUDIO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/audio.mp4");
    const FRAG_VIDEO: &[u8] = include_bytes!("../../tests/fixtures/fragmented/video.mp4");

    /// Where the audio take states the four things a lying file can get wrong:
    /// the `tfhd` flags that say how its samples are described, the `trun` count
    /// that says how many rows follow, the first row's size, and the moment the
    /// one `moof` begins - which is also where the media it indexes does not.
    const AUDIO_TFHD_FLAGS: usize = 768;
    const AUDIO_TRUN_COUNT: usize = 820;
    const AUDIO_TRUN_ROW0_SIZE: usize = 832;
    /// Where the audio take's one `moof` begins, which is also where an init
    /// segment on its own stops.
    const AUDIO_MOOF: usize = 728;
    /// The second video fragment's `tfdt`: its box header, where the kind stands,
    /// and the `baseMediaDecodeTime` the take-away test compares against.
    const VIDEO_TFDT2_KIND: usize = 4_884;

    /// The take with big-endian fields rewritten at the byte offsets above.
    fn rewriting(take: &[u8], patches: &[(usize, u32)]) -> Vec<u8> {
        let mut bytes = take.to_vec();
        for (at, value) in patches {
            bytes[*at..*at + 4].copy_from_slice(&value.to_be_bytes());
        }
        bytes
    }
    /// A box renamed so that a reader walks past it as past any it does not know,
    /// which is how a file that simply never wrote it looks - with every other
    /// offset in the take left alone.
    fn hiding(take: &[u8], at: usize) -> Vec<u8> {
        let mut bytes = take.to_vec();
        bytes[at..at + 4].copy_from_slice(b"zzzz");
        bytes
    }

    fn demux(bytes: &[u8]) -> crate::Result<super::Mp4Reader<std::io::Cursor<&[u8]>>> {
        super::Mp4Reader::open(std::io::Cursor::new(bytes), super::Limits::default())
    }

    /// One sample's worth of an index, in the order the reader keeps it.
    fn entry(sample: &super::Sample) -> (u64, u32, u64, i64, u32, bool) {
        (
            sample.offset,
            sample.size,
            sample.dts,
            sample.pts,
            sample.duration,
            sample.sync,
        )
    }

    #[test]
    fn a_fragment_of_audio_is_indexed_as_its_own_run_states() {
        let file = demux(FRAG_AUDIO).expect("the take opens");
        assert_eq!(file.refused().len(), 0, "nothing here lacks a coding");
        let [track] = file.tracks() else {
            panic!("one audio track, as ffprobe says");
        };
        assert_eq!((track.handler, track.codec), (*b"soun", *b"mp4a"));
        assert_eq!(
            (track.timescale, track.sample_rate, track.channels),
            (48_000, 48_000, 1)
        );
        // The `mvex` promises samples by fragment and the `stbl` holds none, so
        // every number below came out of the `moof`: 58 rows of a duration and a
        // size, laid from the run's own offset inside the `mdat` that follows it.
        let samples = track.samples.expanded().expect("an expanded index");
        assert_eq!(samples.len(), 58, "as many rows as the run counts");
        assert_eq!(
            entry(&samples[0]),
            (1_300, 451, 0, 0, 1_024, true),
            "the first row, at the offset the run offsets from its `moof`"
        );
        assert_eq!(entry(&samples[1]), (1_751, 359, 1_024, 1_024, 1_024, true));
        assert_eq!(
            entry(&samples[57]),
            (20_013, 5, 58_368, 58_368, 256, true),
            "the fill row the encoder ends on, and the last byte of media the take holds"
        );
        assert!(
            samples
                .iter()
                .all(|s| s.pts == i64::try_from(s.dts).unwrap()),
            "sound is displayed as it is decoded"
        );
        assert!(
            samples.iter().all(|s| s.sync),
            "every row of sound restarts"
        );
        assert_eq!(
            samples.iter().map(|s| u64::from(s.duration)).sum::<u64>(),
            track.duration,
            "the track is as long as its samples, which is what the empty init segment left unsaid"
        );
        assert_eq!(track.duration, 58_624);
    }

    #[test]
    fn fragments_of_video_keep_the_decode_stamps_and_the_restart_points() {
        let file = demux(FRAG_VIDEO).expect("the take opens");
        let [track] = file.tracks() else {
            panic!("one video track, as ffprobe says");
        };
        assert_eq!((track.handler, track.codec), (*b"vide", *b"avc1"));
        assert_eq!(
            (track.width, track.height, track.timescale),
            (320, 240, 12_800)
        );
        // The init segment describes the coding in its `stsd`, exactly as a file
        // with a complete sample table does: a fragment adds nothing to it, and a
        // decoder that got no `avcC` here could not start at any of these rows.
        assert_eq!(
            track.configuration.get(..4),
            Some(&[1, 0x64, 0x00, 0x0d][..]),
            "an H.264 configuration record, version and profile and length size"
        );
        let samples = track.samples.expanded().expect("an expanded index");
        assert_eq!(samples.len(), 25, "five fragments of five rows each");
        // Each tuple is one row as the fragments state it: where its bytes start
        // and how many, then decode time, display time and length in ticks of
        // 12 800 Hz, then whether a player may start from it. The display times
        // are the decode ones plus the composition shift each row carries, and
        // only the first row of a fragment is a keyframe - which is the fragment
        // form of a `stss` list, and what a seek lands on.
        #[rustfmt::skip]
        let expected: [(u64, u32, u64, i64, u32, bool); 25] = [
            (   930,  3485,      0,   1024, 512, true),
            (  4415,   268,    512,   3072, 512, false),
            (  4683,    68,   1024,   2048, 512, false),
            (  4751,    36,   1536,   1536, 512, false),
            (  4787,    33,   2048,   2560, 512, false),
            (  4972,  3066,   2560,   3584, 512, true),
            (  8038,   258,   3072,   5632, 512, false),
            (  8296,    63,   3584,   4608, 512, false),
            (  8359,    25,   4096,   4096, 512, false),
            (  8384,    30,   4608,   5120, 512, false),
            (  8566,  3066,   5120,   6144, 512, true),
            ( 11632,   245,   5632,   8192, 512, false),
            ( 11877,    70,   6144,   7168, 512, false),
            ( 11947,    50,   6656,   6656, 512, false),
            ( 11997,    42,   7168,   7680, 512, false),
            ( 12191,  3087,   7680,   8704, 512, true),
            ( 15278,   232,   8192,  10240, 512, false),
            ( 15510,    50,   8704,   9216, 512, false),
            ( 15560,    28,   9216,   9728, 512, false),
            ( 15588,   138,   9728,  10752, 512, false),
            ( 15878,  3018,  10240,  11264, 512, true),
            ( 18896,   240,  10752,  12800, 512, false),
            ( 19136,    64,  11264,  11776, 512, false),
            ( 19200,    27,  11776,  12288, 512, false),
            ( 19227,   125,  12288,  13312, 512, false),        ];
        for (index, sample) in samples.iter().enumerate() {
            assert_eq!(entry(sample), expected[index], "sample {index}");
        }
        assert_eq!(
            track.duration, 12_800,
            "one second, as the last sample ends"
        );
    }

    #[test]
    fn every_fragmented_sample_reads_the_bytes_its_index_points_at() {
        // The index is only worth what the reader can fetch by it: each packet
        // has to come back the length the row said, and the whole walk has to
        // cover the media exactly once - which is what the audio take's one
        // `mdat` and the video take's five each of their own test together.
        for (take, samples, media) in [(FRAG_AUDIO, 58usize, 18_718usize), (FRAG_VIDEO, 25, 17_814)]
        {
            let mut file = demux(take).expect("the take opens");
            let track = &file.tracks()[0];
            let indexed: usize = track
                .samples
                .expanded()
                .expect("an expanded index")
                .iter()
                .map(|s| usize::try_from(s.size).unwrap())
                .sum();
            assert_eq!(indexed, media, "the rows add up to the media they name");
            let mut read = 0usize;
            for index in 0..samples {
                let mut packet = Vec::new();
                file.read_packet(0, index, &mut packet).expect("packet");
                assert_eq!(
                    packet.len(),
                    usize::try_from(file.tracks()[0].samples.get(index).unwrap().size).unwrap()
                );
                read += packet.len();
            }
            assert_eq!(read, media, "and nothing was read twice or left over");
        }
    }

    #[test]
    fn a_fragment_that_leaves_out_its_decode_time_continues_the_track() {
        // `tfdt` is optional, and a muxer appending to a live stream is what
        // leaves it out. The take says 2 560 for its second fragment either way,
        // so hiding that box only keeps the test honest if the stamps are still
        // the ones the first fragment's samples add up to.
        let take = hiding(FRAG_VIDEO, VIDEO_TFDT2_KIND);
        let file = demux(&take).expect("the take opens");
        let samples = file.tracks()[0]
            .samples
            .expanded()
            .expect("an expanded index");
        assert_eq!(samples.len(), 25);
        assert_eq!(
            samples[5].dts, 2_560,
            "where the last sample of the one before ended"
        );
        assert_eq!(samples[5].pts, 3_584, "and its own shift on top");
        assert_eq!(
            samples[10].dts, 5_120,
            "the next fragment still says its own"
        );
        assert_eq!(
            samples[9].offset, 8_384,
            "and the run still measures from a row's end"
        );
    }

    #[cfg(feature = "player")]
    #[test]
    #[cfg(feature = "player")]
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

    #[test]
    fn fragment_validation_preserves_refusals_and_resolves_an_implicit_base() {
        // Flags this reader has no rule for, a run whose rows do not fill it, a
        // sample reaching past the media: each
        // is said in the bytes of the take above, and each has to stop the file
        // rather than index a wrong sample list a player would play quietly.
        let error = demux(&rewriting(FRAG_AUDIO, &[(AUDIO_TFHD_FLAGS, 0x0002_0078)]))
            .err()
            .expect("a reserved `tfhd` bit is no reading this one has");
        assert!(
            error.to_string().contains("track-fragment header flags"),
            "{error}"
        );
        let error = demux(&rewriting(FRAG_AUDIO, &[(AUDIO_TRUN_COUNT, 57)]))
            .err()
            .expect("57 rows are not 476 bytes of them");
        assert!(error.to_string().contains("track-run size"), "{error}");
        let error = demux(&rewriting(
            FRAG_AUDIO,
            &[(AUDIO_TRUN_ROW0_SIZE, 0x0fff_ffff)],
        ))
        .err()
        .expect("no sample is 268 MB of a 20 KB take");
        assert!(error.to_string().contains("outside mdat"), "{error}");
        // The first fragment has a defined implicit moof base even without
        // default-base-is-moof. Its complete index and payload must match.
        let implicit = rewriting(FRAG_AUDIO, &[(AUDIO_TFHD_FLAGS, 0x0000_0038)]);
        let mut actual = demux(&implicit).expect("the first fragment has an implicit base");
        let mut expected = demux(FRAG_AUDIO).unwrap();
        assert_eq!(actual.tracks().len(), expected.tracks().len());
        for track in 0..actual.tracks().len() {
            assert_eq!(actual.tracks()[track].duration, expected.tracks()[track].duration);
            assert_eq!(actual.tracks()[track].timescale, expected.tracks()[track].timescale);
            assert_eq!(actual.tracks()[track].samples.len(), expected.tracks()[track].samples.len());
            for sample in 0..actual.tracks()[track].samples.len() {
                assert_eq!(entry(&actual.tracks()[track].samples.get(sample).unwrap()), entry(&expected.tracks()[track].samples.get(sample).unwrap()));
                let (mut a,mut b)=(Vec::new(),Vec::new());
                actual.read_packet(track,sample,&mut a).unwrap();
                expected.read_packet(track,sample,&mut b).unwrap();
                assert_eq!(a,b);
            }
        }
        // An init segment on its own names no media at all: `mvex` says the file
        // indexes by fragment and no fragment was ever written, which is what a
        // stream that stopped before it began leaves behind.
        let error = demux(&FRAG_AUDIO[..AUDIO_MOOF])
            .err()
            .expect("an init segment is not a playable file");
        assert!(
            error.to_string().contains("init segment without media"),
            "{error}"
        );
        // And the same take whole is what all four of those bytes came from.
        assert!(demux(FRAG_AUDIO).is_ok());
    }
}

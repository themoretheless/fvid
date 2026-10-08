//! Bounded seekable WebM/Matroska indexing, without an external demultiplexer.
use crate::color::hdr::{ColourDescription, HdrMetadata, MasteringDisplay};
use crate::color::tonemap::ContentLight;
use crate::{Result, container::FileTags, invalid, unsupported};
use std::io::{Read, Seek, SeekFrom};
pub use fvid_media::owned_matroska::Chapter;
use fvid_media::owned_opus_packet::{duration_ns as opus_lace_duration, header_channels as opus_lace_channels};
use fvid_media::owned_aac::config::AudioSpecificConfig as WebmAacConfig;
include!("../../crates/fvid-media/src/owned_webm_reader_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn atom(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 127);
        [id, &[0x80 | payload.len() as u8], payload].concat()
    }
    #[test]
    fn rectangular_projection_roll_has_clockwise_display_orientation() {
        let parse = |kind: u8, yaw: f64, pitch: f64, roll: f64| {
            let data = [atom(&[0x76, 0x71], &[kind]),
                atom(&[0x76, 0x73], &yaw.to_be_bytes()), atom(&[0x76, 0x74], &pitch.to_be_bytes()),
                atom(&[0x76, 0x75], &roll.to_be_bytes())].concat();
            let parent = Element { id: 0x7670, data: 0, end: Some(data.len() as u64) };
            read_rotation(&mut Cursor::new(data), parent, &mut 0, 100)
        };
        for (roll, angle) in [(0.0, 0), (-90.0, 90), (90.0, 270), (180.0, 180), (-180.0, 180)] {
            assert_eq!(parse(0, 0.0, 0.0, roll).unwrap(), angle);
        }
        assert_eq!(parse(1, 0.0, 0.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 180.0, 0.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 0.0, 90.0, 90.0).unwrap(), 0);
        assert_eq!(parse(0, 0.0, 0.0, 45.0).unwrap(), 0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(parse(0, 0.0, 0.0, bad).is_err());
            assert!(parse(0, bad, 0.0, 0.0).is_err());
            assert!(parse(0, 0.0, bad, 0.0).is_err());
        }
    }
    #[test]
    fn block_duration_scales_once_across_lazy_clusters_and_rejects_overflow() {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"matroska"));
        let info = atom(&[0x15, 0x49, 0xa9, 0x66], &atom(&[0x2a, 0xd7, 0xb1], &2_000_000u32.to_be_bytes()));
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &atom(&[0xae], &[
            atom(&[0xd7], &[1]), atom(&[0x83], &[2]), atom(&[0x86], b"A_AAC"),
        ].concat()));
        let cluster = |ticks: u64, duplicate: bool| {
            let mut group = atom(&[0xa1], &[0x81, 0, 0, 0, 1]);
            group.extend(atom(&[0x9b], &ticks.to_be_bytes()));
            if duplicate { group.extend(atom(&[0x9b], &[1])); }
            atom(&[0x1f, 0x43, 0xb6, 0x75], &[atom(&[0xe7], &[0]), atom(&[0xa0], &group)].concat())
        };
        let prefix = [header, vec![0x18, 0x53, 0x80, 0x67, 0xff], info, tracks].concat();
        let data = [prefix.clone(), cluster(5, false), cluster(7, false)].concat();
        let mut reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
        reader.scan_all().unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.iter().map(|p| p.duration_ns).collect::<Vec<_>>(), [Some(10_000_000), Some(14_000_000)]);
        for invalid in [cluster(u64::MAX, false), cluster(5, true)] {
            let result = WebmReader::open(Cursor::new([prefix.clone(), invalid].concat()), Limits::default())
                .and_then(|mut r| r.scan_all());
            assert!(result.is_err());
        }
    }
    #[test]
    fn delay_and_signed_discard_padding_keep_nanosecond_units() {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"matroska"));
        let info = atom(&[0x15, 0x49, 0xa9, 0x66], &atom(&[0x2a, 0xd7, 0xb1], &2_000_000u32.to_be_bytes()));
        let track = atom(&[0xae], &[
            atom(&[0xd7], &[1]), atom(&[0x83], &[2]), atom(&[0x86], b"A_AAC"),
            atom(&[0x56, 0xaa], &21_333_333u32.to_be_bytes()),
        ].concat());
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        for value in [0i64, 1, -1, 128, -129, 1_000_000, -1_000_000, i64::MIN, i64::MAX] {
            // Include both compact negative/positive signed encodings and 8-byte extremes.
            let encoded = value.to_be_bytes();
            let width = if (-128..=127).contains(&value) { 1 } else { 8 };
            for padding_first in [false, true] {
                let block = atom(&[0xa1], &[0x81, 0, 3, 0, 0xe0]);
                let padding = atom(&[0x75, 0xa2], &encoded[8-width..]);
                let children = if padding_first { [padding, block] } else { [block, padding] };
                let group = atom(&[0xa0], &children.concat());
                let cluster = atom(&[0x1f, 0x43, 0xb6, 0x75], &[atom(&[0xe7], &[1]), group].concat());
                let bytes = [header.clone(), vec![0x18, 0x53, 0x80, 0x67, 0xff], info.clone(), tracks.clone(), cluster].concat();
                let mut reader = WebmReader::open(Cursor::new(bytes), Limits::default()).unwrap();
                reader.scan_all().unwrap();
                assert_eq!(reader.tracks[0].codec_delay_ns, 21_333_333);
                assert_eq!(reader.packets[0].discard_padding_ns, value);
                assert_eq!(reader.packets[0].pts_ns, 8_000_000);
                assert_eq!(reader.read_packet(0).unwrap(), [0xe0]);
            }
        }
    }

    fn fixture(lace: bool) -> Vec<u8> {
        let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
        let track = atom(
            &[0xae],
            &[
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[1]),
                atom(&[0x86], b"V_VP9"),
                atom(
                    &[0xe0],
                    &[atom(&[0xb0], &[16]), atom(&[0xba], &[16])].concat(),
                ),
            ]
            .concat(),
        );
        let tracks = atom(&[0x16, 0x54, 0xae, 0x6b], &track);
        let cluster = |time: u8| {
            [
                vec![0x1f, 0x43, 0xb6, 0x75, 0xff],
                atom(&[0xe7], &[time]),
                atom(
                    &[0xa3],
                    &[0x81, 0xff, 0xff, if lace { 0x82 } else { 0x80 }, 0x82, 0x49],
                ),
            ]
            .concat()
        };
        [
            header,
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            tracks,
            cluster(2),
            cluster(4),
        ]
        .concat()
    }
    #[test]
    fn unknown_segment_and_cluster_sizes_signed_timestamps_and_packet_reads() {
        let mut r = WebmReader::open(Cursor::new(fixture(false)), Limits::default()).unwrap();
        assert_eq!(r.tracks[0].codec, "V_VP9");
        assert_eq!(r.tracks[0].width, 16);
        assert_eq!(r.packets.len(), 2);
        assert_eq!(r.packets[0].pts_ns, 1_000_000);
        assert_eq!(r.packets[1].pts_ns, 3_000_000);
        assert!(r.packets.iter().all(|p| p.keyframe));
        assert_eq!(r.read_packet(1).unwrap(), [0x82, 0x49]);
        assert!(r.read_packet(2).is_err());
    }
    /// A track's display size says what shape its pixels have only against the
    /// size it stores them at. Pixel dimensions and explicit display ratios
    /// both specify shape; physical display units remain unsupported.
    #[test]
    fn a_video_track_display_size_becomes_the_shape_of_its_pixels() {
        fn video(extra: &[Vec<u8>]) -> Vec<u8> {
            let mut block = vec![atom(&[0xb0], &[16]), atom(&[0xba], &[8])];
            block.extend_from_slice(extra);
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    atom(&[0xe0], &block.concat()),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        let track = |extra: &[Vec<u8>]| {
            let data = video(extra);
            let reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
            reader.tracks[0].clone()
        };
        // 16x8 stored, drawn 32x8: every pixel twice as wide as it is tall.
        let wide = track(&[atom(&[0x54, 0xb0], &[32]), atom(&[0x54, 0xba], &[8])]);
        assert_eq!((wide.width, wide.height, wide.display), (16, 8, (32, 8)));
        assert_eq!(wide.pixel_aspect(), (2, 1));
        // Drawn at the size it is stored: square, stated rather than silent.
        assert_eq!(
            track(&[atom(&[0x54, 0xb0], &[16]), atom(&[0x54, 0xba], &[8])]).pixel_aspect(),
            (1, 1)
        );
        assert_eq!(track(&[]).pixel_aspect(), (1, 1));
        let ratio = track(&[atom(&[0x54, 0xb0], &[16]), atom(&[0x54, 0xba], &[9]), atom(&[0x54, 0xb2], &[3])]);
        assert_eq!(ratio.pixel_aspect(), (8, 9));
        // Measured in centimetres rather than pixels, the pair is a size on a
        // screen whose dots this reader does not know.
        let counted = track(&[
            atom(&[0x54, 0xb0], &[100]),
            atom(&[0x54, 0xba], &[50]),
            atom(&[0x54, 0xb2], &[2]),
        ]);
        assert_eq!((counted.display, counted.pixel_aspect()), ((0, 0), (1, 1)));
    }
    /// The four `PixelCrop*` elements state a border of coded pixels to keep
    /// off screen. They arrive in the order the specification writes them —
    /// bottom, top, left, right — and are kept in the order a viewer cuts them;
    /// a crop that leaves nothing to show is a statement the file cannot keep,
    /// so the whole picture stands and the pixels stay square.
    #[test]
    fn a_video_track_keeps_its_crop_borders_and_shows_only_what_survives_them() {
        fn video(extra: &[Vec<u8>]) -> Vec<u8> {
            let mut block = vec![atom(&[0xb0], &[16]), atom(&[0xba], &[16])];
            block.extend_from_slice(extra);
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    atom(&[0xe0], &block.concat()),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        let track = |extra: &[Vec<u8>]| {
            let reader = WebmReader::open(Cursor::new(video(extra)), Limits::default()).unwrap();
            reader.tracks[0].clone()
        };
        let cropped = track(&[
            atom(&[0x54, 0xaa], &[2]),
            atom(&[0x54, 0xbb], &[3]),
            atom(&[0x54, 0xcc], &[4]),
            atom(&[0x54, 0xdd], &[2]),
        ]);
        assert_eq!(cropped.crop, [4, 3, 2, 2]);
        assert_eq!(cropped.visible(), (10, 11));
        // Nothing cropped, nothing to keep off screen.
        assert_eq!(track(&[]).visible(), (16, 16));
        // A border as wide as the picture leaves no picture to show.
        let greedy = track(&[atom(&[0x54, 0xcc], &[8]), atom(&[0x54, 0xdd], &[8])]);
        assert_eq!(greedy.crop, [0; 4]);
        assert_eq!(greedy.visible(), (16, 16));
        // The display size is a statement about the visible picture, so the
        // crop divides into the coded size before the shape of a pixel does:
        // 16 stored with a 4-pixel border leaves 12x16 to draw, and a file
        // that draws that into 16x16 means pixels four parts wide to three
        // parts tall.
        let stretched = track(&[
            atom(&[0x54, 0xcc], &[4]),
            atom(&[0x54, 0xb0], &[16]),
            atom(&[0x54, 0xba], &[16]),
        ]);
        assert_eq!(stretched.visible(), (12, 16));
        assert_eq!(stretched.pixel_aspect(), (4, 3));
    }
    /// An audio track's rate is a float element, not an integer, and
    /// `CodecPrivate` is where Vorbis keeps the setup headers a decoder cannot
    /// start without.
    #[test]
    fn audio_track_reads_its_float_rate_and_codec_private() {
        fn with_rate(rate: Vec<u8>) -> Vec<u8> {
            let header = atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm"));
            let track = atom(
                &[0xae],
                &[
                    atom(&[0xd7], &[2]),
                    atom(&[0x83], &[2]),
                    atom(&[0x86], b"A_VORBIS"),
                    atom(&[0x63, 0xa2], b"\x01vorbisidentification"),
                    atom(
                        &[0xe1],
                        &[atom(&[0xb5], &rate), atom(&[0x9f], &[2])].concat(),
                    ),
                ]
                .concat(),
            );
            [
                header,
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &track),
            ]
            .concat()
        }
        for rate in [
            44100.0f32.to_be_bytes().to_vec(),
            44100.0f64.to_be_bytes().to_vec(),
        ] {
            let reader = WebmReader::open(Cursor::new(with_rate(rate)), Limits::default()).unwrap();
            let track = &reader.tracks[0];
            assert_eq!(track.kind, 2);
            assert_eq!(track.sample_rate, 44_100);
            assert_eq!(track.channels, 2);
            assert_eq!(track.codec_private, b"\x01vorbisidentification");
        }
        for rate in [
            0.0f64.to_be_bytes().to_vec(),
            f64::NAN.to_be_bytes().to_vec(),
        ] {
            assert!(WebmReader::open(Cursor::new(with_rate(rate)), Limits::default()).is_err());
        }
    }
    #[test]
    fn declared_duration_uses_final_scale_and_validates_float() {
        for value in [
            12.5f32.to_be_bytes().to_vec(),
            12.5f64.to_be_bytes().to_vec(),
        ] {
            let mut data = fixture(false);
            data.extend(atom(
                &[0x15, 0x49, 0xa9, 0x66],
                &[
                    atom(&[0x44, 0x89], &value),
                    atom(&[0x2a, 0xd7, 0xb1], &[0x03, 0xe8]),
                ]
                .concat(),
            ));
            let reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
            assert_eq!(reader.duration_ns, Some(12_500));
        }
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.0, f64::MAX] {
            let mut data = fixture(false);
            data.extend(atom(
                &[0x15, 0x49, 0xa9, 0x66],
                &atom(&[0x44, 0x89], &value.to_be_bytes()),
            ));
            assert!(WebmReader::open(Cursor::new(data), Limits::default()).is_err());
        }
        assert_eq!(
            WebmReader::open(Cursor::new(fixture(false)), Limits::default())
                .unwrap()
                .duration_ns,
            None
        );
    }
    #[test]
    fn malformed_vints_lacing_and_limits_fail() {
        assert!(vint(&mut Cursor::new([0]), false).is_err());
        assert!(vint(&mut Cursor::new([8, 0, 0, 0, 0]), true).is_err());
        assert!(WebmReader::open(Cursor::new(fixture(true)), Limits::default()).is_err());
        for limits in [
            Limits {
                packets: 1,
                ..Limits::default()
            },
            Limits {
                packet_bytes: 1,
                ..Limits::default()
            },
            Limits {
                elements: 2,
                ..Limits::default()
            },
        ] {
            assert!(WebmReader::open(Cursor::new(fixture(false)), limits).is_err());
        }
        let bytes = fixture(false);
        for len in 0..bytes.len() {
            let _ = WebmReader::open(Cursor::new(&bytes[..len]), Limits::default());
        }
    }
    /// `tests/fixtures/chapters/chapters.mkv`, made with:
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=4 -i chapters.txt \
    ///   -map 0:v -map_metadata 1 -c:v libvpx-vp9 -crf 63 -b:v 0 \
    ///   -pix_fmt yuv420p tests/fixtures/chapters/chapters.mkv
    /// where `chapters.txt` is an FFmetadata file naming the three chapters at
    /// 0-1 s, 1-3 s and 3-4 s.
    const CHAPTERS: &[u8] = include_bytes!("../../tests/fixtures/chapters/chapters.mkv");
    #[test]
    fn chapter_atoms_come_out_in_the_files_own_clock() {
        let reader = WebmReader::open(Cursor::new(CHAPTERS), Limits::default())
            .expect("fixture has chapters");
        let shown: Vec<(u64, &str)> = reader
            .chapters
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (0, "Opening"),
                // A title written in another alphabet reaches the player as is.
                (1_000_000_000, "Глава 2"),
                (3_000_000_000, "End"),
            ]
        );
    }
    fn chapter(atoms: &[u8]) -> Vec<u8> {
        atom(&[0x45, 0xb9], &atom(&[0xb6], atoms))
    }
    #[test]
    fn chapter_times_are_nanoseconds_independent_of_segment_scale() {
        let mut file = fixture(false);
        // Even with default 1 ms Segment ticks and no declared duration,
        // chapter values 100 and 250 remain nanoseconds, never milliseconds.
        file.extend(atom(
            &[0x10, 0x43, 0xa7, 0x70],
            &[
                chapter(&[0x91, 0x81, 100]),
                chapter(
                    &[
                        &[0x91, 0x81, 250][..],
                        &[0x92, 0x82, 0x01, 0x90][..],
                        &atom(&[0x80], &atom(&[0x85], b"Ok\0")),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        ));
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        let shown: Vec<(u64, &str)> = reader
            .chapters
            .iter()
            .map(|c| (c.start_ns, c.title.as_str()))
            .collect();
        assert_eq!(shown, [(100, ""), (250, "Ok")]);
        assert_eq!(reader.chapters[0].end_ns, None);
        assert_eq!(reader.chapters[1].end_ns, Some(400));
    }
    #[test]
    fn a_chapter_list_that_cannot_be_walked_leaves_the_file_playable() {
        let mut file = fixture(false);
        // An edition written with an unknown size has no end for the walker to
        // stop at, so its chapters are dropped rather than the whole file,
        // which plays fine without them.
        file.extend(atom(
            &[0x10, 0x43, 0xa7, 0x70],
            &[
                &[0x45, 0xb9, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff][..],
                &[0x91, 0x81, 1],
            ]
            .concat(),
        ));
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert!(reader.chapters.is_empty());
        assert!(!reader.packets.is_empty());
    }
    #[test]
    fn overflowing_crop_borders_keep_the_whole_picture() {
        for crop in [[u64::MAX,0,1,0],[0,u64::MAX,0,1],[u64::MAX,0,u64::MAX,0],[16,0,0,0],[2,2,2,2]] {
            let video=[atom(&[0xb0],&[16]),atom(&[0xba],&[16]),
                atom(&[0x54,0xcc],&crop[0].to_be_bytes()),atom(&[0x54,0xbb],&crop[1].to_be_bytes()),
                atom(&[0x54,0xdd],&crop[2].to_be_bytes()),atom(&[0x54,0xaa],&crop[3].to_be_bytes())].concat();
            let body=[atom(&[0xd7],&[1]),atom(&[0x83],&[1]),atom(&[0x86],b"V_VP9"),atom(&[0xe0],&video)].concat();
            let file=[atom(&[0x1a,0x45,0xdf,0xa3],&atom(&[0x42,0x82],b"webm")),
                vec![0x18,0x53,0x80,0x67,0xff],atom(&[0x16,0x54,0xae,0x6b],&atom(&[0xae],&body))].concat();
            let reader=WebmReader::open(Cursor::new(file),Limits::default()).unwrap();
            assert_eq!(reader.tracks[0].crop,if crop==[2,2,2,2] {crop}else{[0;4]});
        }
    }
    /// What a track is called and which language it speaks are each written in
    /// two spellings: the element in use now and, in files muxed before it, the
    /// older one in the same place. The newer statement wins in either writing
    /// order, the `und` a writer uses for "nothing was said" reads as a track
    /// that states no language, and a name the muxer ended with a NUL byte is
    /// the name without it.
    #[test]
    fn a_track_keeps_the_name_and_the_language_either_spelling_gives_it() {
        fn track(extra: &[Vec<u8>]) -> Track {
            let mut body = vec![
                atom(&[0xd7], &[1]),
                atom(&[0x83], &[2]),
                atom(&[0x86], b"A_VORBIS"),
            ];
            body.extend_from_slice(extra);
            let file = [
                atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
                vec![0x18, 0x53, 0x80, 0x67, 0xff],
                atom(&[0x16, 0x54, 0xae, 0x6b], &atom(&[0xae], &body.concat())),
            ]
            .concat();
            WebmReader::open(Cursor::new(file), Limits::default())
                .expect("built file opens")
                .tracks[0]
                .clone()
        }
        let said = |name: &[u8], current: &[u8], older: &[u8]| {
            let track = track(&[
                atom(&[0x53, 0x6e], name),
                atom(&[0x22, 0xb5, 0x9c], current),
                atom(&[0x44, 0x7a], older),
            ]);
            (track.name, track.language)
        };
        assert_eq!(
            said(b"Commentary", b"ru", b"rus"),
            ("Commentary".into(), "ru".into())
        );
        // Only the older spelling, which is all a file muxed years ago states.
        assert_eq!(said(b"", b"", b"por"), ("".into(), "por".into()));
        // The `und` of either is the writer saying nothing, not a language.
        assert_eq!(said(b"", b"und", b"und"), ("".into(), "".into()));
        assert_eq!(
            said(b"Tail\0", b"eng\0", b""),
            ("Tail".into(), "eng".into())
        );
    }
    /// The same fields as a real muxer writes them: a titled audio track, a
    /// second one that states only its language, a subtitle track with both and
    /// a picture that states neither. `tests/fixtures/tracks/named.mkv`, made
    /// with:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=64x64:rate=10:duration=0.4 \
    ///   -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.4 \
    ///   -f lavfi -i sine=frequency=880:sample_rate=32000:duration=0.4 \
    ///   -i one.srt -i two.srt \
    ///   -map 0:v -map 1:a -map 2:a -map 3:0 -map 4:0 \
    ///   -c:v libvpx-vp9 -pix_fmt yuv420p -c:a flac -ac 1 -c:s srt \
    ///   -metadata:s:a:0 title=Первая -metadata:s:a:0 language=rus \
    ///   -metadata:s:a:1 language=fre \
    ///   -metadata:s:s:0 title=Титры -metadata:s:s:0 language=rus \
    ///   tests/fixtures/tracks/named.mkv
    /// ```
    #[test]
    fn a_real_file_lets_the_reader_name_its_tracks() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tracks/named.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        let shown: Vec<(u64, &str, &str)> = reader
            .tracks
            .iter()
            .map(|track| (track.number, track.name.as_str(), track.language.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (1, "", ""),
                (2, "Первая", "rus"),
                (3, "", "fre"),
                (4, "Титры", "rus"),
                (5, "", ""),
            ]
        );
    }
    /// What a writer that follows the specification calls the file: a `Tag` whose
    /// `Targets` single out nothing, holding a `SimpleTag` named `TITLE`. A tag
    /// aimed at one track is that track's business, and a tag written after a
    /// byte of padding the muxer left behind is still read.
    fn file_tag(name: &[u8], value: &[u8]) -> Vec<u8> {
        atom(
            &[0x73, 0x73],
            &[
                atom(&[0x63, 0xc0], &[]),
                atom(
                    &[0x67, 0xc8],
                    &[atom(&[0x45, 0xa3], name), atom(&[0x44, 0x87], value)].concat(),
                ),
            ]
            .concat(),
        )
    }
    fn track_tag(name: &[u8], value: &[u8]) -> Vec<u8> {
        atom(
            &[0x73, 0x73],
            &[
                atom(
                    &[0x63, 0xc0],
                    &atom(&[0x63, 0xc5], &[0, 0, 0, 0, 0, 0, 7, 9]),
                ),
                atom(
                    &[0x67, 0xc8],
                    &[atom(&[0x45, 0xa3], name), atom(&[0x44, 0x87], value)].concat(),
                ),
            ]
            .concat(),
        )
    }
    /// A list of tags is longer than one byte of length states, so the master
    /// carrying it writes its size as a two-byte variable integer.
    fn masters(body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 16_383);
        let size = (0x4000 | body.len() as u16).to_be_bytes();
        [&[0x12, 0x54, 0xc3, 0x67][..], &size, body].concat()
    }
    #[test]
    fn a_tag_of_the_file_names_it_and_a_tag_of_one_track_does_not() {
        for (built, name) in [
            (file_tag(b"TITLE", "Имя".as_bytes()), "Имя"),
            // The name is matched whatever case the writer chose for it.
            (file_tag(b"title", b"Ok"), "Ok"),
            // A file-level tag still names the file after one aimed at a track.
            (
                [track_tag(b"TITLE", b"No"), file_tag(b"TITLE", b"Ok")].concat(),
                "Ok",
            ),
            (
                [file_tag(b"TITLE", b"Ok"), track_tag(b"TITLE", b"No")].concat(),
                "Ok",
            ),
            // The last byte is what a muxer rewrites the list over: a tag this
            // reader cannot reach is left alone, and the ones before it stand.
            ([file_tag(b"TITLE", b"Ok"), vec![0]].concat(), "Ok"),
        ] {
            let file = [fixture(false), masters(&built)].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.title, name);
        }
        // A list holding nothing a file is named by leaves the file unnamed.
        let file = [
            fixture(false),
            masters(&track_tag(b"AUTHOR", "Не имя".as_bytes())),
        ]
        .concat();
        let reader = WebmReader::open(Cursor::new(file), Limits::default()).expect("opens");
        assert_eq!(reader.tags, FileTags::default());
    }

    /// The rest of what a file states about itself sits in the same list as its
    /// name, one `SimpleTag` per fact. A fact this player has no line for is
    /// dropped on the way, and a fact aimed at one track does not become a fact
    /// about the file.
    #[test]
    fn a_file_keeps_every_tag_that_names_it_rather_than_one_track() {
        let built = [
            file_tag(b"TITLE", b"T"),
            file_tag(b"ARTIST", "Артист".as_bytes()),
            file_tag(b"album", b"B"),
            file_tag(b"GENRE", b"G"),
            file_tag(b"DATE", b"2026"),
            file_tag(b"COMMENT", b"C"),
            file_tag(b"ENCODER", b"Lavf"),
            track_tag(b"ARTIST", b"Not the file's"),
            // A file that states one fact twice is not asked to mean it twice.
            file_tag(b"TITLE", b"later"),
        ]
        .concat();
        let file = [fixture(false), masters(&built)].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(
            reader.tags,
            FileTags {
                title: "T".to_owned(),
                artist: "Артист".to_owned(),
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
    }

    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -metadata title="Имя из контейнера" -c:v libvpx-vp9 -crf 63 -b:v 0 \
    ///   -pix_fmt yuv420p -an tests/fixtures/tags/title.mkv
    /// ```
    /// The muxer here puts the title in the file's own information block rather
    /// than in its tags, and leaves the encoder's name in the tags — which is
    /// what keeps a block's title and a tag of a track from being read as one.
    #[test]
    fn a_real_file_names_itself_in_its_information_block() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tags/title.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        assert_eq!(reader.tags.title, "Имя из контейнера");
    }

    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B \
    ///   -metadata genre=G -metadata date=2026 -metadata comment=C \
    ///   -c:v libvpx-vp9 -crf 63 -b:v 0 -pix_fmt yuv420p -an \
    ///   tests/fixtures/tags/tags.mkv
    /// ```
    /// What that muxer leaves behind: every fact in its own tag of the whole
    /// file, the title among the tags rather than in the information block, and
    /// the encoder's name standing between them for nothing the player asks.
    #[test]
    fn a_real_file_names_itself_and_its_author_in_its_tags() {
        const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/tags/tags.mkv");
        let reader =
            WebmReader::open(Cursor::new(FIXTURE), Limits::default()).expect("fixture opens");
        assert_eq!(
            reader.tags,
            FileTags {
                title: "T".to_owned(),
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
    }

    /// A track number travels to the player under any of the three names its
    /// writers choose: the plain word, the Vorbis field the Matroska tag docs
    /// list, and the spelling ffmpeg's Matroska muxer writes for it. The value
    /// is text here rather than a number's bytes, so a place within an album
    /// keeps its second half: `3/12` arrives as it was written.
    #[test]
    fn a_track_number_arrives_under_whichever_name_its_writer_chose() {
        for name in [b"TRACK".as_slice(), b"TRACKNUMBER", b"PART_NUMBER"] {
            let file = [fixture(false), masters(&file_tag(name, b"3/12"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.track, "3/12", "under {name:?}");
        }
        // The first spelling a file states wins, as with every other fact.
        let built = [
            file_tag(b"TRACKNUMBER", b"2"),
            file_tag(b"PART_NUMBER", b"9"),
        ]
        .concat();
        let file = [fixture(false), masters(&built)].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(reader.tags.track, "2");
    }

    /// The album's own artist, its disc and its publisher arrive under either
    /// spelling their writers choose — the Vorbis field, and the name ffmpeg's
    /// Matroska muxer keeps when it has no mapping of its own — and the disc
    /// keeps a total the file states, text here rather than a number's bytes.
    /// The rights and the file's own note of itself arrive the same way, under
    /// the plain names both muxers give them, and so does the rating that only
    /// this container carries.
    #[test]
    fn the_album_artist_the_disc_and_the_publisher_keep_their_spellings() {
        for name in [b"ALBUMARTIST".as_slice(), b"ALBUM_ARTIST"] {
            let file = [fixture(false), masters(&file_tag(name, b"AA"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.album_artist, "AA", "under {name:?}");
        }
        for name in [b"DISC".as_slice(), b"DISCNUMBER"] {
            let file = [fixture(false), masters(&file_tag(name, b"2/10"))].concat();
            let reader =
                WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
            assert_eq!(reader.tags.disc, "2/10", "under {name:?}");
        }
        let file = [fixture(false), masters(&file_tag(b"PUBLISHER", b"PB"))].concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(reader.tags.publisher, "PB");
        let file = [
            fixture(false),
            masters(
                &[
                    file_tag(b"COPYRIGHT", b"2026 The Holder"),
                    file_tag(b"DESCRIPTION", b"A note"),
                    file_tag(b"RATING", b"5"),
                ]
                .concat(),
            ),
        ]
        .concat();
        let reader =
            WebmReader::open(Cursor::new(file), Limits::default()).expect("built file opens");
        assert_eq!(
            (
                reader.tags.copyright.as_str(),
                reader.tags.description.as_str(),
                reader.tags.rating.as_str(),
            ),
            ("2026 The Holder", "A note", "5")
        );
    }
    /// An element whose size is too long for `atom`'s single length byte.
    ///
    /// Matroska allows a float element of either four or eight bytes and muxers
    /// use both, so the fixtures here do too.
    fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 0x4000);
        [
            id,
            &[0x40 | (payload.len() >> 8) as u8, payload.len() as u8],
            payload,
        ]
        .concat()
    }
    /// A video track whose `Video` element carries exactly the given `Colour`
    /// payload and nothing else.
    fn coloured(colour: &[u8]) -> Track {
        let data = [
            atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            element(
                &[0x16, 0x54, 0xae, 0x6b],
                &element(
                    &[0xae],
                    &[
                        atom(&[0xd7], &[1]),
                        atom(&[0x83], &[1]),
                        atom(&[0x86], b"V_MPEGH/ISO/HEVC"),
                        element(
                            &[0xe0],
                            &[
                                atom(&[0xb0], &[16]),
                                atom(&[0xba], &[8]),
                                element(&[0x55, 0xb0], colour),
                            ]
                            .concat(),
                        ),
                    ]
                    .concat(),
                ),
            ),
        ]
        .concat();
        WebmReader::open(Cursor::new(data), Limits::default())
            .unwrap()
            .tracks[0]
            .clone()
    }
    /// `Colour` states the same H.273 indices an MP4 `colr` atom does, but
    /// numbers its range the other way round: 1 is the studio range a flag
    /// would call 0, and 2 the full range it would call 1. A muxer that wrote
    /// the flag's numbers here would have every file it writes stretched wrong.
    #[test]
    fn a_colour_element_states_its_range_in_matroskas_own_numbers() {
        // The eight bytes ffmpeg's muxer writes for a bt2020nc studio-range
        // track, taken from the file it produced.
        let t = coloured(&[atom(&[0x55, 0xb1], &[9]), atom(&[0x55, 0xb9], &[1])].concat());
        assert_eq!(t.colour.matrix, 9);
        assert!(!t.colour.full_range);
        assert!(t.colour.primary_set().is_none());
        assert!(t.hdr.is_empty());
        let t = coloured(&[atom(&[0x55, 0xb9], &[2])].concat());
        assert!(t.colour.full_range);
        // Nothing stated at all reads the same way, and an element this reader
        // has no meaning for — `BitsPerChannel` — is passed over.
        let t = coloured(&[atom(&[0x55, 0xb2], &[10])].concat());
        assert_eq!(t.colour, ColourDescription::default());
    }
    /// The triple, the light level and the mastering volume in one element,
    /// which is what an HDR10 track muxed by a tool that knows the elements
    /// states.
    #[test]
    fn a_colour_element_carries_the_triple_and_the_mastering_volume() {
        use crate::color::primaries::Primaries;
        let t = coloured(
            &[
                atom(&[0x55, 0xbb], &[9]),
                atom(&[0x55, 0xba], &[16]),
                atom(&[0x55, 0xb1], &[9]),
                atom(&[0x55, 0xb9], &[2]),
            ]
            .concat(),
        );
        assert_eq!(
            t.colour,
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            }
        );
        assert!(t.colour.is_hdr());
        assert_eq!(t.colour.primary_set(), Some(Primaries::BT2020));

        // One element per coordinate, in the named red, green, blue order the
        // byte payload never uses, and already in the units they mean.
        let ids: [[u8; 2]; 10] = [
            [0x55, 0xd1],
            [0x55, 0xd2],
            [0x55, 0xd3],
            [0x55, 0xd4],
            [0x55, 0xd5],
            [0x55, 0xd6],
            [0x55, 0xd7],
            [0x55, 0xd8],
            [0x55, 0xd9],
            [0x55, 0xda],
        ];
        let corners: [f64; 10] = [
            0.708, 0.292, 0.17, 0.797, 0.131, 0.046, 0.3127, 0.329, 1000.0, 0.0001,
        ];
        let volume = |drop: Option<usize>, wide: bool| {
            let mut out = Vec::new();
            for (i, (id, value)) in ids.iter().zip(corners).enumerate() {
                if drop == Some(i) {
                    continue;
                }
                let bytes: Vec<u8> = if wide {
                    value.to_be_bytes().to_vec()
                } else {
                    (value as f32).to_be_bytes().to_vec()
                };
                out.extend_from_slice(&element(id, &bytes));
            }
            out
        };
        let light = || {
            [
                atom(&[0x55, 0xbc], &[0x04, 0xd2]),
                atom(&[0x55, 0xbd], &[0x02, 0x37]),
            ]
            .concat()
        };
        for wide in [true, false] {
            let mut payload = element(&[0x55, 0xd0], &volume(None, wide));
            payload.extend_from_slice(&light());
            let t = coloured(&payload);
            let display = t.hdr.mastering.expect("a complete volume");
            assert!(display.is_hdr10(), "{display:?}");
            assert_eq!(display.max_luminance, 1_000.0);
            assert_eq!(display.min_luminance, 0.0001);
            assert_eq!(t.hdr.light.max_cll, 1_234.0);
            assert_eq!(t.hdr.light.max_fall, 567.0);
            // The volume is also the tone map's destination panel.
            assert_eq!(display.display_target().peak_nits, 1_000.0);
        }
        // A corner short is not a display: the light level the same element
        // states still reaches the player, and no volume is claimed.
        let mut payload = element(&[0x55, 0xd0], &volume(Some(3), true));
        payload.extend_from_slice(&light());
        let t = coloured(&payload);
        assert!(t.hdr.mastering.is_none());
        assert_eq!(t.hdr.light.max_cll, 1_234.0);
    }
    /// A source that reports the furthest byte it has actually handed over, so
    /// a test can say what an opening walk read rather than what it might have.
    struct Reach {
        inner: Cursor<Vec<u8>>,
        high: u64,
    }
    impl Read for Reach {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let at = self.inner.stream_position()?;
            let n = self.inner.read(buf)?;
            self.high = self.high.max(at + n as u64);
            Ok(n)
        }
    }
    impl Seek for Reach {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(from)
        }
    }
    /// A file that states its own length, with `clusters` blocks of `payload`
    /// bytes each, and the offset at which every cluster ends.
    fn timed(clusters: usize, payload: usize) -> (Vec<u8>, Vec<u64>) {
        let info = element(
            &[0x15, 0x49, 0xa9, 0x66],
            &[
                atom(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
                element(&[0x44, 0x89], &5000.0f32.to_be_bytes()),
            ]
            .concat(),
        );
        let tracks = element(
            &[0x16, 0x54, 0xae, 0x6b],
            &element(
                &[0xae],
                &[
                    atom(&[0xd7], &[1]),
                    atom(&[0x83], &[1]),
                    atom(&[0x86], b"V_VP9"),
                    element(
                        &[0xe0],
                        &[atom(&[0xb0], &[16]), atom(&[0xba], &[16])].concat(),
                    ),
                ]
                .concat(),
            ),
        );
        let cluster = |time: u8| {
            let mut block = vec![0x81, 0x00, 0x00, 0x80];
            block.extend(std::iter::repeat_n(0x27, payload));
            element(
                &[0x1f, 0x43, 0xb6, 0x75],
                &[
                    atom(&[0xe7], &[time]),
                    element(&[0xa3], &block),
                ]
                .concat(),
            )
        };
        let mut file = [
            atom(&[0x1a, 0x45, 0xdf, 0xa3], &atom(&[0x42, 0x82], b"webm")),
            vec![0x18, 0x53, 0x80, 0x67, 0xff],
            info,
            tracks,
        ]
        .concat();
        let mut ends = Vec::new();
        for time in 0..clusters {
            file.extend_from_slice(&cluster(time as u8));
            ends.push(file.len() as u64);
        }
        (file, ends)
    }
    /// The hang this indexing shape exists to avoid: an item on a slow mount is
    /// opened by reading it end to end, because blocks can only be listed by
    /// walking the clusters that hold them. With a stated length the walk stops
    /// at the first cluster whose blocks are known, and everything behind it is
    /// read only when a reader asks for the picture that lives there.
    #[test]
    fn opening_a_timed_file_stops_at_the_first_cluster() {
        let (file, ends) = timed(3, 4096);
        let mut r = WebmReader::open(Reach {
            inner: Cursor::new(file),
            high: 0,
        }, Limits::default())
        .expect("built file opens");
        assert_eq!(r.duration_ns, Some(5_000_000_000), "the length the file stated");
        assert_eq!(r.packets.len(), 1);
        assert!(!r.fully_indexed());
        assert!(
            r.reader.high < ends[0],
            "the second cluster begins at {} and open read to {}",
            ends[0],
            r.reader.high
        );
        assert_eq!(r.packets[0].size, 4096);
        assert_eq!(r.read_packet(0).unwrap(), vec![0x27; 4096]);
        // The rest arrives a cluster at a time, and the last cluster is reported
        // as the growth it is rather than as the end of the file.
        assert!(r.scan_more().unwrap());
        assert_eq!(r.packets.len(), 2);
        assert!(r.reader.high > ends[0]);
        assert!(r.scan_more().unwrap(), "the last cluster still indexes");
        assert_eq!(r.packets.len(), 3);
        assert!(!r.scan_more().unwrap(), "nothing is left behind it");
        assert!(r.fully_indexed());
        assert_eq!(r.duration_ns, Some(5_000_000_000), "a settled index keeps it");
    }
    /// A jump costs the distance it travels, not the length of the item: the
    /// walk stops at the first block past the point asked for, and the clusters
    /// behind it stay unread until something needs them. The fixture times one
    /// millisecond per cluster, so a target is reached by the cluster of that
    /// number.
    #[test]
    fn scanning_to_a_point_leaves_the_file_behind_it_unread() {
        let (file, ends) = timed(6, 4096);
        let mut r = WebmReader::open(
            Reach {
                inner: Cursor::new(file),
                high: 0,
            },
            Limits::default(),
        )
        .expect("built file opens");
        r.scan_until(2_000_000).unwrap();
        assert_eq!(r.packets.len(), 3, "the walk stopped at the target");
        assert!(!r.fully_indexed());
        assert!(
            r.reader.high < ends[3],
            "the fourth cluster begins at {} and the walk read to {}",
            ends[3],
            r.reader.high
        );
        // Asking past the last block is asking for the end, and gets it.
        r.scan_until(i64::MAX).unwrap();
        assert_eq!(r.packets.len(), 6);
        assert!(r.fully_indexed());
        assert!(
            r.reader.high > ends[4],
            "the walk reached the last cluster, having read to {}",
            r.reader.high
        );
    }
    /// A file that states no length has its tail as the only statement of it, so
    /// the opening walk still measures the whole item and a consumer that reads
    /// the duration off its own blocks gets the answer it always got.
    #[test]
    fn a_file_with_no_stated_length_is_indexed_whole_at_open() {
        let r = WebmReader::open(Cursor::new(fixture(false)), Limits::default())
            .expect("built file opens");
        assert_eq!(r.packets.len(), 2);
        assert!(r.fully_indexed());
    }
}

//! Owned high-throughput 16-bit intra profile at 14/16-bit sample precision.
use std::{io::Cursor, path::Path};
#[test]
fn deep_precision_decoding_playback_and_export_match_hm() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (depth, suffix) in [
        (14u8, ""),
        (14, "-parallel"),
        (16, ""),
        (16, "-parallel"),
        (14, "-inter"),
        (14, "-inter-parallel"),
    ] {
        let stem = format!("hevc-deep-rext{depth}{suffix}");
        let source = root.join(format!("{stem}.mp4"));
        let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
        assert!(
            oracle
                .chunks_exact(2)
                .any(|b| u16::from_le_bytes([b[0], b[1]]) & ((1u16 << (depth - 12)) - 1) != 0),
            "fixture must retain precision beyond 12 bits"
        );
        let bytes = std::fs::read(&source).unwrap();
        let mut input =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut decoder = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
            &input.tracks()[0].configuration,
            16 << 20,
        )
        .unwrap();
        let [width, height] = decoder.parameters().0.dimensions.map(|v| v as usize);
        assert_eq!(decoder.parameters().0.depth, [depth; 2]);
        assert_eq!(decoder.parameters().0.chroma_format, 3);
        assert!(decoder.parameters().0.extended_precision);
        assert!(decoder.parameters().0.cabac_bypass_alignment);
        assert_eq!(decoder.parameters().1.entropy_sync, !suffix.is_empty());
        for _ in 0..2 {
            let mut actual = Vec::new();
            for i in 0..input.tracks()[0].samples.len() {
                let mut packet = Vec::new();
                input.read_packet(0, i, &mut packet).unwrap();
                let headers = decoder.slice_headers(&packet).unwrap();
                let kind = if suffix.contains("inter") && i > 0 {
                    fvid::codec::hevc_cabac::SliceType::B
                } else {
                    fvid::codec::hevc_cabac::SliceType::I
                };
                assert!(
                    headers
                        .iter()
                        .filter(|h| !h.dependent)
                        .all(|h| h.slice_type == kind)
                );
                let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                for plane in &frame.picture.planes {
                    for v in plane.samples() {
                        actual.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            assert_eq!(actual, oracle, "{stem}: decoder differs");
            decoder.reset();
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-deep-export-{}-{stem}.mkv",
            std::process::id()
        ));
        for negate in [false, true] {
            let filtered: Vec<u8> = oracle
                .chunks_exact(2)
                .flat_map(|b| {
                    let v = u16::from_le_bytes([b[0], b[1]]);
                    (if negate {
                        (((1u32 << depth) - 1) as u16) - v
                    } else {
                        v
                    })
                    .to_le_bytes()
                })
                .collect();
            for export in [
                fvid_media::transcode_lossless,
                fvid::media::transcode_lossless,
            ] {
                export(
                    &source,
                    &output,
                    fvid_media::LosslessTransform {
                        negate: negate.then(String::new),
                        ..Default::default()
                    },
                    &Default::default(),
                )
                .unwrap();
                for path in [&source, &output] {
                    let mut reader = fvid::playback_native::NativeReader::software(
                        Cursor::new(std::fs::read(path).unwrap()),
                        16 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut actual = Vec::new();
                        let mut count = 0;
                        while let Some(frame) = reader.read_frame_raw().unwrap() {
                            let fvid::playback_native::RawFrame::Planar(p) = frame else {
                                panic!("precise planes required")
                            };
                            assert_eq!(p.depth, depth);
                            assert_eq!(p.frame.subsampling, Some([1, 1]));
                            assert_eq!([p.frame.width, p.frame.height], [width, height]);
                            actual.extend_from_slice(&p.frame.data);
                            count += 1;
                        }
                        assert_eq!(count, 3);
                        assert_eq!(
                            actual.as_slice(),
                            if path == &source {
                                oracle.as_slice()
                            } else {
                                filtered.as_slice()
                            },
                            "{stem}: transport differs"
                        );
                        reader.rewind().unwrap();
                    }
                }
                std::fs::remove_file(&output).unwrap();
            }
        }
    }
}

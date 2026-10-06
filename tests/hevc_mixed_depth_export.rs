//! Mixed HEVC component precision survives owned FFV1 export without narrowing.
use std::{io::Cursor, path::Path};
#[test]
fn mixed_component_depths_export_every_hm_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (y, c) in [(8u8, 10u8), (10, 8), (12, 10)] {
        for suffix in ["", "-wpp", "-mixed-tiles", "-parallel", "-cross"] {
            let stem = format!("hevc-mixed-depth-y{y}-c{c}{suffix}");
            let source = root.join(format!("{stem}.mp4"));
            let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
            let [width, height] = if suffix == "-parallel" {
                [128, 96]
            } else {
                [64, 64]
            };
            let depth = y.max(c);
            let expected: Vec<u8> = oracle
                .chunks_exact(2)
                .enumerate()
                .flat_map(|(i, b)| {
                    let component = (i % (width * height * 3)) / (width * height);
                    (u16::from_le_bytes([b[0], b[1]])
                        << (depth - if component == 0 { y } else { c }))
                    .to_le_bytes()
                })
                .collect();
            let output = std::env::temp_dir().join(format!(
                "fvid-mixed-depth-export-{}-{stem}.mkv",
                std::process::id()
            ));
            for negate in [false, true] {
                let expected = if negate {
                    expected
                        .chunks_exact(2)
                        .flat_map(|b| {
                            (((1u16 << depth) - 1) - u16::from_le_bytes([b[0], b[1]])).to_le_bytes()
                        })
                        .collect::<Vec<_>>()
                } else {
                    expected.clone()
                };
                for root_export in [false, true] {
                    let export = if root_export {
                        fvid::media::transcode_lossless
                    } else {
                        fvid_media::transcode_lossless
                    };
                    let transform = fvid_media::LosslessTransform {
                        negate: negate.then(String::new),
                        ..Default::default()
                    };
                    let stats = export(&source, &output, transform, &Default::default())
                        .unwrap_or_else(|e| panic!("{stem}: {e}"));
                    assert_eq!(stats.encoder, "ffv1");
                    let mut reader = fvid::playback_native::NativeReader::software(
                        Cursor::new(std::fs::read(&output).unwrap()),
                        16 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut actual = Vec::new();
                        let mut frames = 0;
                        while let Some(frame) = reader.read_frame_raw().unwrap() {
                            let fvid::playback_native::RawFrame::Planar(p) = frame else {
                                panic!("expected precise packed planes")
                            };
                            assert_eq!(p.depth, depth);
                            assert_eq!(p.frame.subsampling, Some([1, 1]));
                            assert_eq!([p.frame.width, p.frame.height], [width, height]);
                            actual.extend_from_slice(&p.frame.data);
                            frames += 1;
                        }
                        assert_eq!(frames, 3);
                        assert_eq!(actual, expected, "{stem}: exported samples differ from HM");
                        reader.rewind().unwrap();
                    }
                    std::fs::remove_file(&output).unwrap();
                }
            }
        }
    }
}

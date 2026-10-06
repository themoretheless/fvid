//! SCC profile headers with additional SCC coding tools disabled.
use std::{io::Cursor, path::Path};
#[test]
fn scc_base_streams_decode_play_and_export_every_reference_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8u8, 10] {
        for suffix in ["", "-parallel", "-empty-initializers"] {
            let stem = format!("hevc-scc-base{suffix}-rext{depth}");
            let source = root.join(format!("{stem}.mp4"));
            let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
            let bytes = std::fs::read(&source).unwrap();
            let mut input =
                fvid::container::mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default())
                    .unwrap();
            let mut decoder = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            assert_eq!(decoder.parameters().0.profile.profile.unwrap().idc, 9);
            assert!(decoder.parameters().0.scc_extension);
            assert!(decoder.parameters().1.scc_extension);
            assert_eq!(decoder.parameters().0.chroma_format, 3);
            assert_eq!(decoder.parameters().0.depth, [depth; 2]);
            assert_eq!(decoder.parameters().1.entropy_sync, suffix == "-parallel");
            for _ in 0..2 {
                let mut actual = Vec::new();
                for i in 0..3 {
                    let mut packet = Vec::new();
                    input.read_packet(0, i, &mut packet).unwrap();
                    let f = decoder.decode_packet(&packet).unwrap().unwrap();
                    for p in &f.picture.planes {
                        for &v in p.samples() {
                            if depth == 8 {
                                actual.push(v as u8);
                            } else {
                                actual.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(actual, oracle);
                decoder.reset();
            }
            let verify = |path: &Path| {
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(path).unwrap()),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut actual = Vec::new();
                    let mut count = 0;
                    while let Some(frame) = reader.read_frame_raw().unwrap() {
                        match frame {
                            fvid::playback_native::RawFrame::Planar(p) => {
                                assert_eq!(p.depth, depth);
                                assert_eq!(p.frame.subsampling, Some([1, 1]));
                                actual.extend_from_slice(&p.frame.data);
                            }
                            fvid::playback_native::RawFrame::Planar8(p) => {
                                assert_eq!(depth, 8);
                                assert_eq!([p.chroma_width, p.chroma_height], [p.width, p.height]);
                                for plane in [&p.y, &p.cb, &p.cr] {
                                    actual.extend_from_slice(plane);
                                }
                            }
                            _ => panic!("expected planar SCC output"),
                        }
                        count += 1;
                    }
                    assert_eq!(count, 3);
                    assert_eq!(actual, oracle, "{stem}: sample mismatch");
                    reader.rewind().unwrap();
                }
            };
            verify(&source);
            let output = std::env::temp_dir()
                .join(format!("fvid-scc-base-{}-{stem}.mkv", std::process::id()));
            for export in [
                fvid::media::transcode_lossless,
                fvid_media::transcode_lossless,
            ] {
                export(&source, &output, Default::default(), &Default::default()).unwrap();
                verify(&output);
                std::fs::remove_file(&output).unwrap();
            }
        }
    }
}

#[test]
fn scc_reserved_motion_resolution_is_invalid_not_silently_ignored() {
    let source = include_bytes!("fixtures/playback-errors/hevc-scc-reserved-motion.mp4");
    let input =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
    let result = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
        &input.tracks()[0].configuration,
        16 << 20,
    );
    assert!(
        matches!(result,Err(fvid::Error::Invalid(ref s)) if s.contains("reserved HEVC SCC motion resolution"))
    );
}

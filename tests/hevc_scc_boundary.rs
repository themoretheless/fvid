//! SCC intra boundary filtering disable with a controlled independent HM oracle.
use std::{io::Cursor, path::Path};
#[test]
fn scc_boundary_disabled_streams_decode_play_and_export_every_reference_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8u8, 10] {
        for suffix in ["", "-parallel"] {
            let stem = format!("hevc-scc-boundary-disabled{suffix}-rext{depth}");
            let source = root.join(format!("{stem}.mp4"));
            let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
            let dims = if suffix == "-parallel" {
                [128, 96]
            } else {
                [64, 64]
            };
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
            assert!(decoder.parameters().0.intra_boundary_filtering_disabled);
            // A decoder that merely skips the flag must fail the pixel oracle.
            let mut first_packet = Vec::new();
            input.read_packet(0, 0, &mut first_packet).unwrap();
            let headers = decoder.slice_headers(&first_packet).unwrap();
            let mut ignored = decoder.parameters().0.clone();
            ignored.intra_boundary_filtering_disabled = false;
            let lists = vec![[Vec::new(), Vec::new()]; headers.len()];
            let wrong = fvid::codec::hevc_picture::decode_slices(
                &ignored,
                decoder.parameters().1,
                &headers,
                0,
                &lists,
                16 << 20,
            )
            .unwrap();
            let mut wrong_pixels = Vec::new();
            for p in &wrong.planes {
                for &v in p.samples() {
                    if depth == 8 {
                        wrong_pixels.push(v as u8);
                    } else {
                        wrong_pixels.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            assert_ne!(
                wrong_pixels.as_slice(),
                &oracle[..wrong_pixels.len()],
                "fixture must expose ignored boundary-disable flag"
            );

            assert!(decoder.parameters().1.scc_extension);
            assert_eq!(decoder.parameters().0.dimensions.map(|v| v as usize), dims);
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
                                assert_eq!([p.frame.width, p.frame.height], dims);
                                assert_eq!(p.depth, depth);
                                assert_eq!(p.frame.subsampling, Some([1, 1]));
                                actual.extend_from_slice(&p.frame.data);
                            }
                            fvid::playback_native::RawFrame::Planar8(p) => {
                                assert_eq!([p.width, p.height], dims);
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

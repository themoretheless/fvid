//! Owned current-picture prediction acceptance against SCM samples.
use std::{io::Cursor, path::Path};
#[test]
fn scc_ibc_streams_decode_play_and_export_every_scm_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8u8, 10] {
        for name in [
            "intra",
            "parallel",
            "weighted",
            "weighted-parallel",
            "420-odd",
            "420-odd-parallel",
            "inter",
            "inter-parallel",
            "bidir",
            "bidir-parallel",
            "tiles",
            "dependent",
        ] {
            let stem = format!("hevc-scc-ibc-{name}-rext{depth}");
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

            let chroma = if name.starts_with("420") { 1 } else { 3 };
            assert_eq!(decoder.parameters().0.chroma_format, chroma);
            assert_eq!(decoder.parameters().0.depth, [depth; 2]);
            assert_eq!(
                decoder.parameters().1.weighted_prediction,
                name.starts_with("weighted")
            );
            assert_eq!(
                decoder.parameters().1.entropy_sync,
                name.ends_with("parallel")
            );
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
                                assert_eq!(
                                    p.frame.subsampling,
                                    if chroma == 1 {
                                        Some([2, 2])
                                    } else {
                                        Some([1, 1])
                                    }
                                );
                                actual.extend_from_slice(&p.frame.data);
                            }
                            fvid::playback_native::RawFrame::Planar8(p) => {
                                assert_eq!(depth, 8);
                                assert_eq!(
                                    [p.chroma_width, p.chroma_height],
                                    if chroma == 1 {
                                        [p.width / 2, p.height / 2]
                                    } else {
                                        [p.width, p.height]
                                    }
                                );
                                for plane in [&p.y, &p.cb, &p.cr] {
                                    actual.extend_from_slice(plane);
                                }
                            }
                            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                                assert_eq!(chroma, 1);
                                assert_eq!(picture.bit_depth, depth);
                                picture.write_planar(&mut actual).unwrap();
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
fn current_picture_parameters_and_headers_parse_with_sps_dependency() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10] {
        for name in [
            "intra",
            "parallel",
            "weighted",
            "weighted-parallel",
            "420-odd",
            "420-odd-parallel",
            "inter",
            "inter-parallel",
            "bidir",
            "bidir-parallel",
            "tiles",
            "dependent",
        ] {
            let source = root.join(format!("hevc-scc-ibc-{name}-rext{depth}.mp4"));
            let mut input = fvid::container::mp4::Mp4Reader::open(
                Cursor::new(std::fs::read(source).unwrap()),
                Default::default(),
            )
            .unwrap();
            let configuration = input.tracks()[0].configuration.clone();
            let config = fvid::codec::config::HevcConfig::parse(&configuration).unwrap();
            let sps = config
                .arrays
                .iter()
                .find(|a| a.nal_type == 33)
                .unwrap()
                .units[0];
            let sps = fvid::codec::hevc_sps::Sps::parse(sps, 16 << 20).unwrap();
            assert!(sps.current_picture_reference);
            let pps = fvid::codec::hevc_pps::Pps::parse(
                config
                    .arrays
                    .iter()
                    .find(|a| a.nal_type == 34)
                    .unwrap()
                    .units[0],
                &sps,
                16 << 20,
            )
            .unwrap();
            assert!(pps.current_picture_reference);
            assert_eq!(pps.tiles.is_some(),name=="tiles" || name=="dependent");
            assert_eq!(pps.dependent_slices,name=="dependent");
            let mut disabled = sps.clone();
            disabled.current_picture_reference = false;
            assert!(
                fvid::codec::hevc_pps::Pps::parse(
                    config
                        .arrays
                        .iter()
                        .find(|a| a.nal_type == 34)
                        .unwrap()
                        .units[0],
                    &disabled,
                    16 << 20
                )
                .unwrap_err()
                .to_string()
                .contains("requires SPS capability")
            );
            let mut dependent_segments=0;
            for frame in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, frame, &mut packet).unwrap();
                let mut previous=None;
                for nal in fvid::codec::config::NalUnits::new(&packet, config.length_size).unwrap()
                {
                    let nal = nal.unwrap();
                    if !fvid::codec::hevc_nal::NalHeader::parse(nal)
                        .unwrap()
                        .is_vcl()
                    {
                        continue;
                    }
                    let header =
                        fvid::codec::hevc_slice::SliceHeader::parse_with_previous(nal, &sps, &pps, 16 << 20, previous.as_ref())
                            .unwrap();
                    assert_eq!(header.slice_type, if name.starts_with("bidir") && frame != 0 { fvid::codec::hevc_cabac::SliceType::B } else { fvid::codec::hevc_cabac::SliceType::P });
                    dependent_segments+=usize::from(header.dependent);
                    previous=Some(header.clone());
                    assert!(header.current_picture_reference);
                    if !(name.starts_with("inter") || name.starts_with("bidir")) || frame == 0 {
                        assert_eq!(header.references, [1, 0]);
                    } else {
                        assert!(header.references[0] > 1);
                        assert!(header.short_term.iter().any(|r| r.used));
                        if name.starts_with("bidir") {assert!(header.references[1]>0);}
                    }
                    if frame == 0 {
                        assert!(header.short_term.iter().all(|r| !r.used));
                    }
                    assert!(header.list_modification[0].is_none());
                    assert!(header.long_term.iter().all(|r| !r.used));
                }
            }
            assert_eq!(dependent_segments>0,name=="dependent");
            assert!(
                fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
                    &configuration,
                    16 << 20
                )
                .is_ok()
            );
        }
    }
}

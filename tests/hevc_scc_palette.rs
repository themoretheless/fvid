//! Owned palette parameter and pixel acceptance against SCM.
use std::{io::Cursor, path::Path};

#[test]
fn palette_parameters_parse_limits_and_initializers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10, 12, 14] {
        for name in [
            "intra",
            "parallel",
            "initializers",
            "initializers-parallel",
            "pps-initializers",
            "pps-initializers-parallel",
            "tiles",
            "dependent",
            "420",
            "420-parallel",
            "422",
            "422-parallel",
            "mono",
            "mono-parallel",
            "escape",
            "escape-parallel",
            "escape-bypass",
            "escape-bypass-parallel",
            "420-escape",
            "420-escape-parallel",
            "420-escape-bypass",
            "420-escape-bypass-parallel",
            "422-escape",
            "422-escape-parallel",
            "422-escape-bypass",
            "422-escape-bypass-parallel",
            "mono-escape",
            "mono-escape-parallel",
            "mono-escape-bypass",
            "mono-escape-bypass-parallel",
            "precision-escape",
            "precision-escape-bypass",
            "mono-precision-escape",
            "mono-precision-escape-bypass",
            "420-precision-escape",
            "420-precision-escape-bypass",
            "422-precision-escape",
            "422-precision-escape-bypass",
        ] {
            if name.contains("precision") && depth < 12 {
                continue;
            }
            if depth >= 12 && matches!(name, "tiles" | "dependent") {
                continue;
            }
            let bytes =
                std::fs::read(root.join(format!("hevc-scc-palette-{name}-rext{depth}.mp4")))
                    .unwrap();
            let mut input =
                fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes), Default::default())
                    .unwrap();
            let configuration = input.tracks()[0].configuration.clone();
            let config = fvid::codec::config::HevcConfig::parse(&configuration).unwrap();
            let nal = config
                .arrays
                .iter()
                .find(|a| a.nal_type == 33)
                .unwrap()
                .units[0];
            let sps = fvid::codec::hevc_sps::Sps::parse(nal, 16 << 20).unwrap();
            let palette = sps.palette.as_ref().unwrap();
            assert_eq!(
                (palette.maximum, palette.predictor_maximum),
                (if name.contains("escape") { 4 } else { 63 }, 128)
            );
            assert_eq!(
                !palette.initial.is_empty(),
                name.starts_with("initializers")
            );
            assert_eq!(
                sps.depth,
                if name.starts_with("mono") {
                    [depth, 8]
                } else {
                    [depth; 2]
                }
            );
            assert_eq!(
                sps.chroma_format,
                if name.starts_with("mono") {
                    0
                } else if name.starts_with("420") {
                    1
                } else if name.starts_with("422") {
                    2
                } else {
                    3
                }
            );
            let pps_nal = config
                .arrays
                .iter()
                .find(|a| a.nal_type == 34)
                .unwrap()
                .units[0];
            let pps = fvid::codec::hevc_pps::Pps::parse(pps_nal, &sps, 16 << 20).unwrap();
            assert_eq!(
                pps.palette_initial.as_ref().is_some_and(|e| !e.is_empty()),
                name.starts_with("pps-initializers")
            );
            assert_eq!(pps.tiles.is_some(), name == "tiles" || name == "dependent");
            assert_eq!(pps.dependent_slices, name == "dependent");
            let mut dependent = 0;
            for frame in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, frame, &mut packet).unwrap();
                let mut previous = None;
                for nal in fvid::codec::config::NalUnits::new(&packet, config.length_size).unwrap()
                {
                    let nal = nal.unwrap();
                    if !fvid::codec::hevc_nal::NalHeader::parse(nal)
                        .unwrap()
                        .is_vcl()
                    {
                        continue;
                    }
                    let header = fvid::codec::hevc_slice::SliceHeader::parse_with_previous(
                        nal,
                        &sps,
                        &pps,
                        16 << 20,
                        previous.as_ref(),
                    )
                    .unwrap();
                    dependent += usize::from(header.dependent);
                    previous = Some(header);
                }
            }
            assert_eq!(dependent > 0, name == "dependent");
            if name.starts_with("pps-initializers") {
                let mut no_palette = sps.clone();
                no_palette.palette = None;
                assert!(
                    fvid::codec::hevc_pps::Pps::parse(pps_nal, &no_palette, 16 << 20)
                        .unwrap_err()
                        .to_string()
                        .contains("require SPS capability")
                );
                let mut wrong_depth = sps.clone();
                wrong_depth.depth[0] = if depth == 8 { 10 } else { 8 };
                assert!(
                    fvid::codec::hevc_pps::Pps::parse(pps_nal, &wrong_depth, 16 << 20)
                        .unwrap_err()
                        .to_string()
                        .contains("depth disagrees")
                );
            }
        }
    }
}

#[test]
fn palette_streams_match_every_scm_sample_after_reset() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [8, 10, 12, 14] {
        for name in [
            "intra",
            "parallel",
            "initializers",
            "initializers-parallel",
            "pps-initializers",
            "pps-initializers-parallel",
            "tiles",
            "dependent",
            "420",
            "420-parallel",
            "422",
            "422-parallel",
            "mono",
            "mono-parallel",
            "escape",
            "escape-parallel",
            "escape-bypass",
            "escape-bypass-parallel",
            "420-escape",
            "420-escape-parallel",
            "420-escape-bypass",
            "420-escape-bypass-parallel",
            "422-escape",
            "422-escape-parallel",
            "422-escape-bypass",
            "422-escape-bypass-parallel",
            "mono-escape",
            "mono-escape-parallel",
            "mono-escape-bypass",
            "mono-escape-bypass-parallel",
            "precision-escape",
            "precision-escape-bypass",
            "mono-precision-escape",
            "mono-precision-escape-bypass",
            "420-precision-escape",
            "420-precision-escape-bypass",
            "422-precision-escape",
            "422-precision-escape-bypass",
        ] {
            if name.contains("precision") && depth < 12 {
                continue;
            }
            if depth >= 12 && matches!(name, "tiles" | "dependent") {
                continue;
            }
            let stem = format!("hevc-scc-palette-{name}-rext{depth}");
            let source = root.join(format!("{stem}.mp4"));
            let bytes = std::fs::read(root.join(format!("{stem}.mp4"))).unwrap();
            let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
            let mut input =
                fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes), Default::default())
                    .unwrap();
            let mut decoder = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
                &input.tracks()[0].configuration,
                16 << 20,
            )
            .unwrap();
            for _ in 0..2 {
                let mut actual = Vec::new();
                for frame in 0..3 {
                    let mut packet = Vec::new();
                    input.read_packet(0, frame, &mut packet).unwrap();
                    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                    for plane in &decoded.picture.planes {
                        for &sample in plane.samples() {
                            if depth == 8 {
                                actual.push(sample as u8);
                            } else {
                                actual.extend_from_slice(&sample.to_le_bytes());
                            }
                        }
                    }
                }
                assert!(
                    actual == oracle,
                    "{stem}: first sample mismatch {:?}",
                    actual.iter().zip(&oracle).position(|(a, b)| a != b)
                );
                decoder.reset();
            }
            let verify = |path: &Path, mono_divisor: u32| {
                let playback_oracle = if name.starts_with("mono") {
                    let [w, h] = decoder.parameters().0.dimensions;
                    let storage = if depth == 8 { 1 } else { 2 };
                    let neutral = 1u16 << (depth - 1);
                    let mut expected = Vec::new();
                    for frame in oracle.chunks_exact(w as usize * h as usize * storage) {
                        expected.extend_from_slice(frame);
                        let divisor = mono_divisor;
                        for _ in 0..(w / divisor * h / divisor) * 2 {
                            if depth == 8 {
                                expected.push(neutral as u8);
                            } else {
                                expected.extend_from_slice(&neutral.to_le_bytes());
                            }
                        }
                    }
                    expected
                } else {
                    oracle.clone()
                };
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(path).unwrap()),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut output = Vec::new();
                    let mut frames = 0;
                    while let Some(frame) = reader.read_frame_raw().unwrap() {
                        match frame {
                            fvid::playback_native::RawFrame::Planar(p) => {
                                assert_eq!(p.depth, depth);
                                output.extend_from_slice(&p.frame.data);
                            }
                            fvid::playback_native::RawFrame::Planar8(p) => {
                                assert_eq!(depth, 8);
                                for plane in [&p.y, &p.cb, &p.cr] {
                                    output.extend_from_slice(plane);
                                }
                            }
                            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                                assert!(name.starts_with("420"));
                                assert_eq!(picture.bit_depth, depth);
                                picture.write_planar(&mut output).unwrap();
                            }
                            _ => panic!("expected planar palette output"),
                        }
                        frames += 1;
                    }
                    assert_eq!(frames, 3);
                    assert!(
                        output == playback_oracle,
                        "{stem}: playback mismatch path={} lengths={}/{} first={:?}",
                        path.display(),
                        output.len(),
                        playback_oracle.len(),
                        output
                            .iter()
                            .zip(&playback_oracle)
                            .position(|(a, b)| a != b)
                    );
                    reader.rewind().unwrap();
                }
            };
            verify(&source, 2);
            let output = std::env::temp_dir()
                .join(format!("fvid-palette-{}-{stem}.mkv", std::process::id()));
            for (index, export) in [
                fvid::media::transcode_lossless,
                fvid_media::transcode_lossless,
            ]
            .into_iter()
            .enumerate()
            {
                export(&source, &output, Default::default(), &Default::default()).unwrap();
                verify(&output, if index == 0 { 2 } else { 1 });
                std::fs::remove_file(&output).unwrap();
            }
        }
    }
}

#[test]
fn escape_fixture_provenance_requires_actual_transposed_reads() {
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/hevc-scc-palette-oracle.json"
    ))
    .unwrap();
    let mut escape_streams = 0;
    for fixture in report["fixtures"].as_array().unwrap() {
        let stem = fixture["stem"].as_str().unwrap();
        if !stem.contains("escape") {
            continue;
        }
        escape_streams += 1;
        if stem.contains("precision") {
            let pixels = std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors")
                    .join(format!("{stem}.yuv")),
            )
            .unwrap();
            assert!(
                pixels
                    .chunks_exact(2)
                    .any(|p| u16::from_le_bytes([p[0], p[1]]) & 15 != 0),
                "{stem}: missing full-precision low bits"
            );
        }
        let kind = if stem.contains("bypass") {
            "escape_bypass_samples"
        } else {
            "escape_lossy_samples"
        };
        assert!(
            fixture[kind].as_u64().unwrap() > 0,
            "{stem}: missing actual escape reads"
        );
        assert!(
            fixture["transpose_escape_samples"].as_u64().unwrap() > 0,
            "{stem}: missing actual transposed escape reads"
        );
    }
    assert_eq!(escape_streams, 80);
}

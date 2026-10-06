//! ACT preserves mixed component precision and consumes non-zero slice offsets.
use std::{io::Cursor, path::Path};

#[test]
fn mixed_deep_and_slice_act_streams_decode_play_export_and_rewind() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (kind, pairs) in [
        (
            "depth",
            vec![[8u8, 10], [10, 8], [14, 14], [14, 10], [10, 14]],
        ),
        ("slice", vec![[8u8, 8], [10, 10], [8, 10], [10, 8]]),
    ] {
        for depths in pairs {
            for parallel in [false, true] {
                let stem = format!(
                    "hevc-scc-act-{kind}-y{}-c{}{}",
                    depths[0],
                    depths[1],
                    if parallel { "-parallel" } else { "" }
                );
                let source = root.join(format!("{stem}.mp4"));
                let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
                let data = std::fs::read(&source).unwrap();
                let mut input =
                    fvid::container::mp4::Mp4Reader::open(Cursor::new(data), Default::default())
                        .unwrap();
                let mut decoder = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
                    &input.tracks()[0].configuration,
                    16 << 20,
                )
                .unwrap();
                let (sps, pps) = decoder.parameters();
                let depth = depths[0].max(depths[1]);
                assert_eq!(sps.depth, depths);
                assert_eq!(sps.chroma_format, 3);
                assert_eq!(
                    sps.profile.profile.unwrap().idc,
                    if depth > 10 { 11 } else { 9 }
                );
                assert!(!sps.extended_precision);
                assert!(!sps.cabac_bypass_alignment);
                assert!(pps.adaptive_colour_transform);
                assert_eq!(pps.slice_act_qp_offsets, kind == "slice");
                assert_eq!(pps.entropy_sync, parallel || depth > 10);
                let [width, height] = sps.dimensions.map(|v| v as usize);
                for _ in 0..2 {
                    let mut actual = Vec::new();
                    for frame in 0..3 {
                        let mut packet = Vec::new();
                        input.read_packet(0, frame, &mut packet).unwrap();
                        for header in decoder.slice_headers(&packet).unwrap() {
                            assert_eq!(
                                header.act_qp_offsets,
                                if kind == "slice" {
                                    [-2, -7, 1]
                                } else {
                                    [-5, -5, -3]
                                }
                            );
                        }
                        let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                        for plane in &decoded.picture.planes {
                            for &v in plane.samples() {
                                if depth == 8 {
                                    actual.push(v as u8);
                                } else {
                                    actual.extend_from_slice(&v.to_le_bytes());
                                }
                            }
                        }
                    }
                    assert_eq!(actual, oracle, "{stem}: decode");
                    decoder.reset();
                }
                let expected = if depth == 8 {
                    oracle.clone()
                } else {
                    oracle
                        .chunks_exact(2)
                        .enumerate()
                        .flat_map(|(i, b)| {
                            let c = (i % (width * height * 3)) / (width * height);
                            (u16::from_le_bytes([b[0], b[1]])
                                << (depth - depths[usize::from(c != 0)]))
                            .to_le_bytes()
                        })
                        .collect()
                };
                let verify = |path: &Path| {
                    let mut player = fvid::playback_native::NativeReader::software(
                        Cursor::new(std::fs::read(path).unwrap()),
                        16 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut actual = Vec::new();
                        let mut frames = 0;
                        while let Some(frame) = player.read_frame_raw().unwrap() {
                            match frame {
                                fvid::playback_native::RawFrame::Planar(p) => {
                                    assert_eq!(p.depth, depth);
                                    assert_eq!(p.frame.subsampling, Some([1, 1]));
                                    actual.extend_from_slice(&p.frame.data);
                                }
                                fvid::playback_native::RawFrame::Planar8(p) => {
                                    assert_eq!(depth, 8);
                                    for plane in [&p.y, &p.cb, &p.cr] {
                                        actual.extend_from_slice(plane);
                                    }
                                }
                                _ => panic!("expected planar ACT output"),
                            }
                            frames += 1;
                        }
                        assert_eq!(frames, 3);
                        assert_eq!(actual, expected, "{stem}: playback/export");
                        player.rewind().unwrap();
                    }
                };
                verify(&source);
                let output = std::env::temp_dir()
                    .join(format!("fvid-act-depth-{}-{stem}.mkv", std::process::id()));
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
}

//! Independent component-depth PCM, storage and presentation acceptance.
use fvid::codec::avc_decoder::AvcDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("avc-component-depths.json")).unwrap())
        .unwrap()
}
fn playback_manifest() -> serde_json::Value {
    let mut m = manifest();
    for file in [
        "avc-component-motion.json",
        "avc-component-fractional.json",
        "avc-component-weights.json",
        "avc-component-bipred.json",
    ] {
        let motion: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root().join(file)).unwrap()).unwrap();
        m["cases"]
            .as_array_mut()
            .unwrap()
            .extend(motion["cases"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), 2450);
    m
}
fn bytes(f: &serde_json::Value) -> Vec<u8> {
    std::fs::read(root().join(f.as_str().unwrap())).unwrap()
}
fn words(f: &serde_json::Value) -> Vec<u16> {
    bytes(f)
        .chunks_exact(2)
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect()
}
fn acceptance(case: &serde_json::Value) {
    let mut decoder = AvcDecoder::new(&bytes(&case["configuration"]), 32 << 20).unwrap();
    for _ in 0..2 {
        let frames = case["frames"].as_array().unwrap();
        let order = case["decode_order"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| (0..frames.len()).collect());
        for index in order {
            let frame = &frames[index];
            let packet = bytes(&frame["packet"]);
            let picture = if case["decode_order"].is_array() {
                decoder.decode_order(&packet)
            } else {
                decoder.decode(&packet)
            }
            .unwrap_or_else(|e| panic!("{}: {e}", case["file"]))
            .unwrap();
            assert_eq!(
                picture.component_depths(),
                [
                    case["luma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8,
                    case["chroma_depth"].as_u64().unwrap() as u8
                ]
            );
            assert_eq!(
                picture.y,
                words(&frame["y"]),
                "{} frame {index} Y",
                case["file"]
            );
            assert_eq!(
                picture.cb,
                words(&frame["cb"]),
                "{} frame {index} Cb",
                case["file"]
            );
            assert_eq!(
                picture.cr,
                words(&frame["cr"]),
                "{} frame {index} Cr",
                case["file"]
            );
        }
        decoder.reset();
    }
}
#[test]
fn all_equal_component_depth_pcm_controls_decode_exact_samples() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 49);
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["luma_depth"] == c["chroma_depth"])
    {
        acceptance(c);
    }
}
#[test]
fn all_mixed_component_depth_pcm_frames_decode_exact_samples() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["luma_depth"] != c["chroma_depth"])
    {
        acceptance(c);
    }
}

#[test]
fn picture_storage_and_planar_writer_keep_each_component_depth() {
    for case in manifest()["cases"].as_array().unwrap() {
        let y = case["luma_depth"].as_u64().unwrap() as u8;
        let c = case["chroma_depth"].as_u64().unwrap() as u8;
        for frame in case["frames"].as_array().unwrap() {
            let picture = fvid::codec::avc_picture::IntraPicture {
                coded_width: 16,
                coded_height: 16,
                crop: [0; 4],
                bit_depth: y,
                chroma_bit_depth: c,
                y: words(&frame["y"]),
                cb: words(&frame["cb"]),
                cr: words(&frame["cr"]),
            };
            assert_eq!(picture.component_depths(), [y, c, c]);
            let mut output = Vec::new();
            picture.write_planar(&mut output).unwrap();
            let mut expected = Vec::new();
            for (plane, depth) in [(&picture.y, y), (&picture.cb, c), (&picture.cr, c)] {
                for &sample in plane {
                    if depth == 8 {
                        expected.push(sample as u8);
                    } else {
                        expected.extend_from_slice(&sample.to_le_bytes());
                    }
                }
            }
            assert_eq!(output, expected, "{}", case["file"]);
            assert_eq!(
                output.len(),
                256 * if y == 8 { 1 } else { 2 } + 128 * if c == 8 { 1 } else { 2 }
            );
        }
    }
}

#[test]
fn mixed_depth_presentation_uses_each_plane_precision() {
    for case in manifest()["cases"].as_array().unwrap() {
        let y = case["luma_depth"].as_u64().unwrap() as u8;
        let c = case["chroma_depth"].as_u64().unwrap() as u8;
        let frame = &case["frames"][0];
        let picture = fvid::codec::avc_picture::IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: y,
            chroma_bit_depth: c,
            y: words(&frame["y"]),
            cb: words(&frame["cb"]),
            cr: words(&frame["cr"]),
        };
        for full in [false, true] {
            let colour = fvid::playback_native::AvcColour {
                kr: 0.299,
                kb: 0.114,
                full,
            };
            let planes = fvid::playback_native::avc_to_planar8(&picture, colour);
            assert_eq!(
                planes.y,
                picture
                    .y
                    .iter()
                    .map(|v| (v >> (y - 8)) as u8)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                planes.cb,
                picture
                    .cb
                    .iter()
                    .map(|v| (v >> (c - 8)) as u8)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                planes.cr,
                picture
                    .cr
                    .iter()
                    .map(|v| (v >> (c - 8)) as u8)
                    .collect::<Vec<_>>()
            );
            let mut rgb = Vec::new();
            fvid::playback_native::avc_to_rgb(&picture, colour, &mut rgb, 1 << 20).unwrap();
            for py in 0..16 {
                for px in 0..16 {
                    let ci = (py / 2) * 8 + px / 2;
                    let ys = f64::from(1u32 << (y - 8));
                    let cs = f64::from(1u32 << (c - 8));
                    let yr = if full {
                        f64::from((1u32 << y) - 1)
                    } else {
                        219. * ys
                    };
                    let crange = if full {
                        f64::from((1u32 << c) - 1)
                    } else {
                        224. * cs
                    };
                    let l = (f64::from(picture.y[py * 16 + px]) - if full { 0. } else { 16. * ys })
                        / yr;
                    let cb = (f64::from(picture.cb[ci]) - 128. * cs) / crange;
                    let cr = (f64::from(picture.cr[ci]) - 128. * cs) / crange;
                    let expected = [
                        l + 1.402 * cr,
                        l - 0.299 * 1.402 / 0.587 * cr - 0.114 * 1.772 / 0.587 * cb,
                        l + 1.772 * cb,
                    ];
                    for channel in 0..3 {
                        let value = (expected[channel] * 255.).round().clamp(0., 255.) as u8;
                        assert!(
                            rgb[(py * 16 + px) * 3 + channel].abs_diff(value) <= 1,
                            "{} pixel {px},{py}, channel {channel}",
                            case["file"]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_geometry_normalizes_mixed_precision_without_losing_samples() {
    for c in manifest()["cases"].as_array().unwrap() {
        let y = c["luma_depth"].as_u64().unwrap() as u8;
        let chroma = c["chroma_depth"].as_u64().unwrap() as u8;
        let depth = y.max(chroma);
        let f = &c["frames"][0];
        let p = fvid::codec::avc_picture::IntraPicture {
            coded_width: 16,
            coded_height: 16,
            crop: [0; 4],
            bit_depth: y,
            chroma_bit_depth: chroma,
            y: words(&f["y"]),
            cb: words(&f["cb"]),
            cr: words(&f["cr"]),
        };
        let expected = [(&p.y, y), (&p.cb, chroma), (&p.cr, chroma)]
            .into_iter()
            .flat_map(|(v, d)| v.iter().map(move |v| v << (depth - d)))
            .collect::<Vec<_>>();
        let raw = fvid::playback_native::RawFrame::Avc {
            picture: std::sync::Arc::new(p),
            colour: fvid::playback_native::AvcColour {
                kr: 0.299,
                kb: 0.114,
                full: true,
            },
        };
        let out = fvid::native_geometry::VideoGeometry::default()
            .apply(&raw, 16, 16)
            .unwrap();
        let actual = if depth == 8 {
            out.data.iter().map(|v| u16::from(*v)).collect::<Vec<_>>()
        } else {
            out.data
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect()
        };
        assert_eq!(actual, expected, "{}", c["file"]);
    }
}

#[test]
fn mp4_playback_seek_and_rewind_keep_separate_component_precision() {
    let m = playback_manifest();
    verify_playback_cases(m["cases"].as_array().unwrap());
}
fn verify_playback_cases(cases: &[serde_json::Value]) {
    use std::io::Cursor;
    for case in cases {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(bytes(&case["file"])),
            32 << 20,
        )
        .unwrap();
        let depths = [
            case["luma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8,
        ];
        let check = |raw: &fvid::playback_native::RawFrame, frame: &serde_json::Value| match raw {
            fvid::playback_native::RawFrame::Avc { picture, .. } => {
                assert_eq!(
                    picture.component_depths(),
                    [depths[0], depths[1], depths[1]]
                );
                assert_eq!(picture.y, words(&frame["y"]));
                assert_eq!(picture.cb, words(&frame["cb"]));
                assert_eq!(picture.cr, words(&frame["cr"]));
            }
            fvid::playback_native::RawFrame::Planar8(p) => {
                assert_eq!(depths, [8, 8]);
                assert_eq!(
                    p.y,
                    words(&frame["y"])
                        .into_iter()
                        .map(|v| v as u8)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    p.cb,
                    words(&frame["cb"])
                        .into_iter()
                        .map(|v| v as u8)
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    p.cr,
                    words(&frame["cr"])
                        .into_iter()
                        .map(|v| v as u8)
                        .collect::<Vec<_>>()
                );
            }
            _ => panic!("unexpected raw precision for {}", case["file"]),
        };
        let mut intervals = Vec::new();
        for pass in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let raw = reader
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["file"]))
                    .unwrap();
                check(&raw, frame);
                let interval = reader.frame_interval().unwrap();
                if pass == 0 {
                    intervals.push(interval);
                } else {
                    assert_eq!(interval, intervals[i]);
                }
                assert_eq!(raw.into_rgb(32 << 20).unwrap().len(), 16 * 16 * 3);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for i in [case["frames"].as_array().unwrap().len() as u64 - 1, 1, 0] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(i * 20))
                .unwrap()
                .unwrap();
            check(&raw, &case["frames"][i as usize]);
            assert_eq!(reader.frame_interval().unwrap(), intervals[i as usize]);
        }
    }
}

#[test]
fn mixed_depth_mp4_export_preserves_every_component_bit_and_timing() {
    let m = playback_manifest();
    verify_export_cases(m["cases"].as_array().unwrap(), "main");
}
fn verify_export_cases(cases: &[serde_json::Value], label: &str) {
    use std::io::Cursor;
    for (index, case) in cases.iter().enumerate() {
        let y = case["luma_depth"].as_u64().unwrap() as u8;
        let c = case["chroma_depth"].as_u64().unwrap() as u8;
        let depth = y.max(c);
        let output = std::env::temp_dir().join(format!(
            "fvid-avc-components-{}-{label}-{index}.mkv",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        fvid_media::transcode_lossless(
            &root().join(case["file"].as_str().unwrap()),
            &output,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(std::fs::read(&output).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        let track = &reader.tracks[0];
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
            track.width as usize,
            track.height as usize,
            &track.codec_private,
            32 << 20,
        )
        .unwrap();
        assert_eq!(
            reader.packets.len(),
            case["frames"].as_array().unwrap().len()
        );
        for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            assert_eq!(decoded.depth, depth, "{}", case["file"]);
            assert_eq!(reader.packets[i].pts_ns, i as i64 * 20_000_000);
            assert_eq!(reader.packets[i].duration_ns, Some(20_000_000));
            let actual = if depth == 8 {
                decoded
                    .frame
                    .data
                    .iter()
                    .map(|v| u16::from(*v))
                    .collect::<Vec<_>>()
            } else {
                decoded
                    .frame
                    .data
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect()
            };
            let mut expected = Vec::new();
            for (plane, width, d) in [("y", 16, y), ("cb", 8, c), ("cr", 8, c)] {
                for row in words(&frame[plane]).chunks_exact(width) {
                    expected.extend(row.iter().rev().map(|v| v << (depth - d)));
                }
            }
            assert_eq!(actual, expected, "{} frame {i}", case["file"]);
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn mixed_depth_p_skip_preserves_reference_components() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-motion.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 49);
    for c in m["cases"].as_array().unwrap() {
        acceptance(c);
    }
}

#[test]
fn mixed_depth_prediction_weights_and_residuals_clip_each_plane_independently() {
    use fvid::codec::avc_compensation::{ComponentWeight, InterLumaResidual, Reference420};
    for case in manifest()["cases"].as_array().unwrap() {
        let depths = [
            case["luma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8,
        ];
        let f = &case["frames"][0];
        let source = [words(&f["y"]), words(&f["cb"]), words(&f["cr"])];
        let r = Reference420::new_components(
            source.each_ref().map(Vec::as_slice),
            16,
            16,
            [16, 8, 8],
            depths,
        )
        .unwrap();
        let check = |p: fvid::codec::avc_compensation::Prediction420, add: [u16; 3]| {
            for (i, actual) in [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()]
                .into_iter()
                .enumerate()
            {
                let depth = depths[usize::from(i != 0)];
                let maximum = (1u16 << depth) - 1;
                let expected = source[i]
                    .iter()
                    .map(|v| v.saturating_add(add[i]).min(maximum))
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected, "{} component {i}", case["file"]);
            }
        };
        let weights = [ComponentWeight {
            weight: 1,
            offset: 1,
            denominator: 0,
        }; 3];
        let delta = [
            1u16 << (depths[0] - 8),
            1u16 << (depths[1] - 8),
            1u16 << (depths[1] - 8),
        ];
        check(
            r.predict([0, 0], [0, 0], [16, 16])
                .unwrap()
                .weight(weights)
                .unwrap(),
            delta,
        );
        check(
            r.predict([0, 0], [0, 0], [16, 16])
                .unwrap()
                .blend(r.predict([0, 0], [0, 0], [16, 16]).unwrap(), [weights; 2])
                .unwrap(),
            delta,
        );
        check(
            r.predict([0, 0], [0, 0], [16, 16])
                .unwrap()
                .reconstruct_inter_bypass(
                    InterLumaResidual::Blocks4(&[[1; 16]; 16]),
                    &[[1; 4]; 2],
                    &[[[1; 16]; 4]; 2],
                )
                .unwrap(),
            [1; 3],
        );
        check(
            r.predict([0, 0], [0, 0], [16, 16])
                .unwrap()
                .reconstruct_inter(
                    InterLumaResidual::Blocks4(&[[0; 16]; 16]),
                    &[[0; 4]; 2],
                    &[[[0; 16]; 4]; 2],
                    [
                        26 + 6 * (depths[0] - 8),
                        26 + 6 * (depths[1] - 8),
                        26 + 6 * (depths[1] - 8),
                    ],
                    &[[16; 16]; 3],
                    &[16; 64],
                )
                .unwrap(),
            [0; 3],
        );
    }
}

#[test]
fn mixed_depth_axial_motion_matches_independent_fractional_oracles() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("avc-component-fractional.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 784);
    for c in m["cases"].as_array().unwrap() {
        acceptance(c);
    }
}

#[test]
fn mixed_depth_weighted_p_bitstreams_match_independent_oracles() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-weights.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 882);
    for c in m["cases"].as_array().unwrap() {
        acceptance(c);
    }
}

#[test]
fn mixed_depth_b_bipred_bitstreams_preserve_precision_and_decode_order() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-bipred.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 686);
    for c in m["cases"].as_array().unwrap() {
        acceptance(c);
    }
}

#[test]
fn mixed_depth_signed_dc_residual_bitstreams_match_independent_oracles() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-residual.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 2352);
    for c in m["cases"].as_array().unwrap() {
        assert!(
            c["residual_values"].as_array().unwrap().iter().any(|p| p
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_i64().unwrap() != 0)),
            "fixture must contain a nonzero decoded residual"
        );
        acceptance(c);
    }
}

#[test]
fn mixed_depth_residual_reference_history_survives_mp4_seek_and_rewind() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-residual.json")).unwrap())
            .unwrap();
    verify_playback_cases(m["cases"].as_array().unwrap());
}

#[test]
fn mixed_depth_residual_export_keeps_reconstructed_reference_samples() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-residual.json")).unwrap())
            .unwrap();
    verify_export_cases(m["cases"].as_array().unwrap(), "residual");
}

#[test]
fn mixed_depth_ac_basis_bitstreams_match_independent_matrix_oracles() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-ac.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 6174);
    for c in m["cases"].as_array().unwrap() {
        assert!(
            c["residual_values"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p.as_array().unwrap().len() > 1),
            "AC fixture must have a nonconstant residual"
        );
        acceptance(c);
    }
}

#[test]
fn mixed_depth_ac_pipeline_reference_history_survives_mp4_seek_and_rewind() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-ac.json")).unwrap())
            .unwrap();
    verify_playback_cases(m["cases"].as_array().unwrap());
}

#[test]
fn mixed_depth_ac_pipeline_export_keeps_reconstructed_reference_samples() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("avc-component-ac.json")).unwrap())
            .unwrap();
    verify_export_cases(m["cases"].as_array().unwrap(), "ac");
}

#[test]
fn mixed_depth_transform8_basis_bitstreams_match_independent_matrix_oracles() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("avc-component-transform8.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 6272);
    for c in m["cases"].as_array().unwrap() {
        acceptance(c);
    }
}

#[test]
fn mixed_depth_transform8_pipeline_reference_history_survives_mp4_seek_and_rewind() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("avc-component-transform8.json")).unwrap(),
    )
    .unwrap();
    verify_playback_cases(m["cases"].as_array().unwrap());
}

#[test]
fn mixed_depth_transform8_pipeline_export_keeps_reconstructed_reference_samples() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("avc-component-transform8.json")).unwrap(),
    )
    .unwrap();
    verify_export_cases(m["cases"].as_array().unwrap(), "transform8");
}

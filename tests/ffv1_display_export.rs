//! Stored display metadata acceptance on short independently authored FFV1 videos.
use serde_json::Value;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-display-geometry.json")).unwrap())
        .unwrap()
}
fn words(data: &[u8], depth: u8) -> Vec<u16> {
    if depth == 8 {
        data.iter().map(|&v| u16::from(v)).collect()
    } else {
        data.chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect()
    }
}
fn source_size(source: &Value) -> (usize, usize) {
    source["size"]
        .as_array()
        .map(|a| {
            (
                a[0].as_u64().unwrap() as usize,
                a[1].as_u64().unwrap() as usize,
            )
        })
        .unwrap_or((4, 2))
}
fn source_sub(source: &Value) -> Option<[usize; 2]> {
    match source["layout"].as_str().unwrap() {
        "rgb" => None,
        "420" => Some([2, 2]),
        "custom" => Some([
            source["subsampling"][0].as_u64().unwrap() as usize,
            source["subsampling"][1].as_u64().unwrap() as usize,
        ]),
        _ => Some([1, 1]),
    }
}
fn turn(mut rows: Vec<Vec<u16>>, rotation: u16, flip: bool) -> Vec<Vec<u16>> {
    rows = match rotation {
        0 => rows,
        180 => {
            rows.reverse();
            for r in &mut rows {
                r.reverse();
            }
            rows
        }
        90 | 270 => {
            let (w, h) = (rows[0].len(), rows.len());
            (0..w)
                .map(|y| {
                    (0..h)
                        .map(|x| {
                            if rotation == 90 {
                                rows[h - 1 - x][y]
                            } else {
                                rows[x][w - 1 - y]
                            }
                        })
                        .collect()
                })
                .collect()
        }
        _ => unreachable!(),
    };
    if flip {
        for r in &mut rows {
            r.reverse();
        }
    }
    rows
}
fn crop_turn(
    values: &[u16],
    width: usize,
    crop: [usize; 4],
    rotation: u16,
    flip: bool,
) -> Vec<u16> {
    let [x, y, w, h] = crop;
    let rows = values
        .chunks_exact(width)
        .skip(y)
        .take(h)
        .map(|r| r[x..x + w].to_vec())
        .collect();
    turn(rows, rotation, flip).into_iter().flatten().collect()
}
fn reference(
    case: &Value,
    frame: &Value,
) -> (Vec<u16>, Option<Vec<u16>>, Option<[usize; 2]>, [usize; 2]) {
    let source = &case["source"];
    let (w, h) = source_size(source);
    let depth = source["depth"].as_u64().unwrap() as u8;
    let crop = case["crop"].as_array().unwrap();
    let (left, top, right, bottom) = (
        crop[0].as_u64().unwrap() as usize,
        crop[1].as_u64().unwrap() as usize,
        crop[2].as_u64().unwrap() as usize,
        crop[3].as_u64().unwrap() as usize,
    );
    let (cw, ch) = (w - left - right, h - top - bottom);
    let rotation = case["rotation"].as_u64().unwrap() as u16;
    let size = if matches!(rotation, 90 | 270) {
        [ch, cw]
    } else {
        [cw, ch]
    };
    let rect = [left, top, cw, ch];
    let color = words(
        &std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap(),
        depth,
    );
    let sub = source_sub(source);
    let (data, out_sub) = if let Some([sx, sy]) = sub {
        let out = if matches!(rotation, 90 | 270) {
            [sy, sx]
        } else {
            [sx, sy]
        };
        let promote = left % sx != 0
            || top % sy != 0
            || !matches!(out, [1, 1] | [2, 1] | [2, 2] | [1, 2] | [4, 1] | [4, 4]);
        let mut data = Vec::new();
        let mut offset = 0;
        for component in 0..3 {
            let (dx, dy) = if component == 0 { (1, 1) } else { (sx, sy) };
            let (pw, ph) = (w.div_ceil(dx), h.div_ceil(dy));
            let plane = &color[offset..offset + pw * ph];
            offset += pw * ph;
            if promote {
                let expanded = (0..h)
                    .flat_map(|y| (0..w).map(move |x| plane[(y / dy) * pw + x / dx]))
                    .collect::<Vec<_>>();
                data.extend(crop_turn(&expanded, w, rect, rotation, true));
            } else {
                data.extend(crop_turn(
                    plane,
                    pw,
                    [left / dx, top / dy, cw.div_ceil(dx), ch.div_ceil(dy)],
                    rotation,
                    true,
                ));
            }
        }
        (data, Some(if promote { [1, 1] } else { out }))
    } else {
        let mut channels = Vec::new();
        for c in 0..3 {
            let p = color.chunks_exact(3).map(|p| p[c]).collect::<Vec<_>>();
            channels.push(crop_turn(&p, w, rect, rotation, true));
        }
        let data = (0..channels[0].len())
            .flat_map(|i| [channels[0][i], channels[1][i], channels[2][i]])
            .collect();
        (data, None)
    };
    let alpha = frame["alpha"].as_str().map(|file| {
        crop_turn(
            &words(&std::fs::read(root().join(file)).unwrap(), depth),
            w,
            rect,
            rotation,
            true,
        )
    });
    (data, alpha, out_sub, size)
}
fn verify_export(
    case: &Value,
    transform: fvid::media_info::LosslessTransform,
    order: &[usize],
    label: &str,
) {
    // Qualify fixture metadata and raw samples before exercising the exporter.
    let mut input = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
        Default::default(),
    )
    .unwrap();
    input.scan_all().unwrap();
    let track = &input.tracks[0];
    assert_eq!(
        track.rotation,
        case["rotation"].as_u64().unwrap() as u16,
        "{label} input rotation"
    );
    assert_eq!(
        track.crop,
        std::array::from_fn(|i| case["crop"][i].as_u64().unwrap()),
        "{label} input crop"
    );
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        track.width as usize,
        track.height as usize,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    for (i, frame) in case["source"]["frames"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let decoded = decoder.decode(&input.read_packet(i).unwrap()).unwrap();
        assert_eq!(
            decoded.frame.data,
            std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap(),
            "{label} input color"
        );
        assert_eq!(
            decoded.alpha,
            frame["alpha"]
                .as_str()
                .map(|f| std::fs::read(root().join(f)).unwrap()),
            "{label} input alpha"
        );
    }
    let path = std::env::temp_dir().join(format!(
        "fvid-display-export-{}-{label}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    fvid_media::transcode_lossless(
        &root().join(case["file"].as_str().unwrap()),
        &path,
        transform,
        &Default::default(),
    )
    .unwrap();
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&path).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let track = &reader.tracks[0];
    assert_eq!(track.rotation, 0);
    assert_eq!(track.crop, [0; 4]);
    assert_eq!(track.pixel_aspect(), (1, 1));
    let depth = case["source"]["depth"].as_u64().unwrap() as u8;
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        track.width as usize,
        track.height as usize,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    assert_eq!(reader.packets.len(), order.len());
    for (i, &source_index) in order.iter().enumerate() {
        assert_eq!(reader.packets[i].pts_ns, i as i64 * 20_000_000);
        assert_eq!(reader.packets[i].duration_ns, Some(20_000_000));
        let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
        let (color, alpha, sub, size) = reference(case, &case["source"]["frames"][source_index]);
        assert_eq!(
            [decoded.frame.width, decoded.frame.height],
            size,
            "{label} size"
        );
        assert_eq!(decoded.depth, depth);
        assert_eq!(decoded.frame.subsampling, sub, "{label} sub");
        assert_eq!(
            words(&decoded.frame.data, depth),
            color,
            "{label} frame {i} color"
        );
        assert_eq!(
            decoded.alpha.map(|a| words(&a, depth)),
            alpha,
            "{label} frame {i} alpha"
        );
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn stored_crop_rotation_and_valid_chroma_profiles_export_exact_color_alpha_and_timing() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 1328);
    for (i, case) in m["cases"].as_array().unwrap().iter().enumerate() {
        let order = (0..case["source"]["frames"].as_array().unwrap().len()).collect::<Vec<_>>();
        verify_export(
            case,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &order,
            &i.to_string(),
        );
    }
}
#[test]
fn normalization_precedes_reverse_shuffle_and_user_geometry() {
    let m = manifest();
    for (i, case) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c["source"]["version"] == 3
                && c["source"]["coder"] == 1
                && c["source"]["depth"] == 16
                && c["rotation"] == 90
                && c["crop"][0] == 1
        })
    {
        verify_export(
            case,
            fvid::media_info::LosslessTransform {
                horizontal_flip: true,
                reverse: Some(String::new()),
                shuffleframes: Some("1 0".into()),
                ..Default::default()
            },
            &[2, 3, 0, 1],
            &format!("temporal-{i}"),
        );
    }
}
#[test]
fn quarter_turn_inverts_pixel_aspect_before_user_resize() {
    let m = manifest();
    let case = &m["aspect"];
    let path = std::env::temp_dir().join(format!("fvid-display-aspect-{}.mkv", std::process::id()));
    let _ = std::fs::remove_file(&path);
    fvid_media::transcode_lossless(
        &root().join(case["file"].as_str().unwrap()),
        &path,
        fvid::media_info::LosslessTransform {
            horizontal_flip: true,
            scale: Some(fvid::media_info::ScaleSize {
                width: 4,
                height: 9,
            }),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&path).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let t = &reader.tracks[0];
    assert_eq!((t.width, t.height), (4, 9));
    assert_eq!(t.pixel_aspect(), (3, 4));
    assert_eq!(t.crop, [0; 4]);
    assert_eq!(t.rotation, 0);
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        4,
        9,
        &t.codec_private,
        32 << 20,
    )
    .unwrap();
    for (index, frame) in case["source"]["frames"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let (color, alpha, sub, size) = reference(case, frame);
        assert_eq!(sub, None);
        assert_eq!(size, [2, 3]);
        // Independent integer enlargement: two columns and three rows per pixel.
        let enlarge = |samples: &[u16], channels: usize| -> Vec<u16> {
            let mut out = Vec::new();
            for row in samples.chunks_exact(2 * channels) {
                for _ in 0..3 {
                    for pixel in row.chunks_exact(channels) {
                        out.extend_from_slice(pixel);
                        out.extend_from_slice(pixel);
                    }
                }
            }
            out
        };
        let decoded = decoder.decode(&reader.read_packet(index).unwrap()).unwrap();
        assert_eq!(words(&decoded.frame.data, 16), enlarge(&color, 3));
        assert_eq!(
            decoded.alpha.map(|a| words(&a, 16)),
            alpha.map(|a| enlarge(&a, 1))
        );
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn yuv_color_and_opacity_use_the_same_nearest_scale_phase() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-alpha.json")).unwrap()).unwrap();
    let repro = &m["scale_phase"];
    let source = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["file"] == repro["source"])
        .unwrap();
    let path =
        std::env::temp_dir().join(format!("fvid-alpha-scale-phase-{}.mkv", std::process::id()));
    let _ = std::fs::remove_file(&path);
    fvid_media::transcode_lossless(
        &root().join(repro["file"].as_str().unwrap()),
        &path,
        fvid::media_info::LosslessTransform {
            scale: Some(fvid::media_info::ScaleSize {
                width: 2,
                height: 2,
            }),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&path).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let t = &reader.tracks[0];
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        2,
        2,
        &t.codec_private,
        32 << 20,
    )
    .unwrap();
    for (i, frame) in source["frames"].as_array().unwrap().iter().enumerate() {
        let color = words(
            &std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap(),
            16,
        );
        let alpha = words(
            &std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap(),
            16,
        );
        let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
        assert_eq!(
            &words(&decoded.frame.data, 16)[..4],
            &[color[1], color[3], color[5], color[7]]
        );
        let expected = [alpha[1], alpha[3], alpha[5], alpha[7]];
        let wrong = [alpha[0], alpha[2], alpha[4], alpha[6]];
        assert_ne!(expected, wrong, "fixture must expose the old alpha phase");
        assert_eq!(words(&decoded.alpha.unwrap(), 16), expected);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn native_packed_display_accepts_the_same_crops_and_chroma_as_file_export() {
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let rotation = case["rotation"].as_u64().unwrap() as u16;
        let [l, t, r, b] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as u32);
        let insets = match rotation {
            90 => [b, l, t, r],
            180 => [r, b, l, t],
            270 => [t, r, b, l],
            _ => [l, t, r, b],
        };
        for frame in case["source"]["frames"].as_array().unwrap() {
            let raw = reader.read_frame_raw().unwrap().unwrap();
            let fvid::playback_native::RawFrame::Planar(p) = raw else {
                panic!("packed FFV1")
            };
            for media in [false, true] {
                let geometry = fvid::native_geometry::VideoGeometry {
                    horizontal_flip: true,
                    ..Default::default()
                };
                let out = if media {
                    geometry.apply_packed_display_media(&p, rotation, insets)
                } else {
                    geometry.apply_packed_display(&p, rotation, insets)
                }
                .unwrap_or_else(|e| panic!("case {index}, media {media}: {e}"));
                let (color, alpha, sub, size) = reference(case, frame);
                assert_eq!([out.frame.width, out.frame.height], size, "case {index}");
                assert_eq!(out.frame.subsampling, sub, "case {index}");
                assert_eq!(words(&out.frame.data, out.depth), color, "case {index}");
                assert_eq!(
                    out.alpha.map(|a| words(&a, out.depth)),
                    alpha,
                    "case {index}"
                );
            }
        }
    }
}

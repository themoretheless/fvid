use fvid::codec::{
    vp9::HeaderState,
    vp9_decoder::Decoder,
    vp9_probs::{CompressedHeader, Probabilities},
};
fn packet(chroma: &str, depth: u8) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}-synthetic.vp9",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn owned_neutral_420_controls_decode_every_sample() {
    for depth in [8, 10, 12] {
        let p = packet("420", depth);
        let frame = Decoder::new(32 << 20).decode(&p).unwrap();
        assert_eq!(frame.picture.size, [64, 64]);
        assert_eq!(frame.picture.depth, depth);
        for (i, plane) in frame.picture.planes.iter().enumerate() {
            assert_eq!(
                [plane.width, plane.height],
                if i == 0 { [64, 64] } else { [32, 32] }
            );
            assert!(plane.samples.iter().all(|&v| v == 1 << (depth - 1)));
        }
    }
}
#[test]
fn full_chroma_neutral_pictures_decode_every_sample_and_reset() {
    for depth in [8, 10, 12] {
        for chroma in ["422", "444"] {
            let p = packet(chroma, depth);
            let h = HeaderState::default().parse(&p).unwrap();
            let subsampling = if chroma == "422" {
                [true, false]
            } else {
                [false, false]
            };
            assert_eq!(h.picture.format.subsampling, subsampling);
            assert!(h.lossless());
            CompressedHeader::parse(&p, &h, &Probabilities::default()).unwrap();
            let mut d = Decoder::new(32 << 20);
            for _ in 0..2 {
                d.reset();
                let frame = d.decode(&p).unwrap();
                assert_eq!(frame.picture.subsampling, subsampling);
                for (i, plane) in frame.picture.planes.iter().enumerate() {
                    let [sx, sy] = subsampling.map(|s| usize::from(i > 0 && s));
                    assert_eq!([plane.width, plane.height], [64 >> sx, 64 >> sy]);
                    assert!(
                        plane.samples.iter().all(|&v| v == 1 << (depth - 1)),
                        "{chroma} depth {depth} plane {i}"
                    );
                }
            }
        }
    }
}
#[test]
fn coloured_dc_residuals_match_independent_analytic_planes() {
    for depth in [8, 10, 12] {
        for chroma in ["420", "422", "444", "440"] {
            let base = format!(
                "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}-coloured",
                env!("CARGO_MANIFEST_DIR")
            );
            let p = std::fs::read(format!("{base}.vp9")).unwrap();
            let oracle = std::fs::read(format!("{base}.yuv16")).unwrap();
            let mut d = Decoder::new(32 << 20);
            for _ in 0..2 {
                d.reset();
                let frame = d
                    .decode(&p)
                    .unwrap_or_else(|e| panic!("{chroma} {depth}: {e}"));
                let samples: Vec<_> = frame
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                assert_eq!(samples, oracle, "{chroma} depth {depth}");
            }
        }
    }
}
#[test]
fn webm_chroma_geometry_pixels_resize_seek_and_rewind() {
    use fvid::{playback_native::RawFrame, playback_webm::WebmVideoReader};
    use std::io::Cursor;
    for depth in [8, 10, 12] {
        for (chroma, factors) in [
            ("420", [2, 2]),
            ("422", [2, 1]),
            ("444", [1, 1]),
            ("440", [1, 2]),
        ] {
            let base = format!(
                "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}",
                env!("CARGO_MANIFEST_DIR")
            );
            let source = std::fs::read(format!("{base}-synthetic.webm")).unwrap();
            let oracle = std::fs::read(format!("{base}-coloured.yuv16")).unwrap();
            let mut player =
                WebmVideoReader::open_software(Cursor::new(source.clone()), 32 << 20).unwrap();
            for _ in 0..2 {
                player.rewind();
                for (index, [w, h]) in [[64usize, 64], [17, 25], [72, 65]].into_iter().enumerate() {
                    let p = player.read_frame_planes().unwrap().unwrap();
                    assert_eq!([p.width, p.height], [w, h]);
                    let cw = w.div_ceil(factors[0]);
                    let ch = h.div_ceil(factors[1]);
                    assert_eq!([p.chroma_width, p.chroma_height], [cw, ch]);
                    let actual: Vec<_> = [&p.y, &p.cb, &p.cr]
                        .into_iter()
                        .flat_map(|p| p.iter().copied())
                        .collect();
                    if index == 0 {
                        let expected: Vec<_> = oracle
                            .chunks_exact(2)
                            .map(|b| (u16::from_le_bytes([b[0], b[1]]) >> (depth - 8)) as u8)
                            .collect();
                        assert_eq!(actual, expected, "{chroma} {depth}");
                    } else {
                        assert!(actual.iter().all(|&v| v == 128));
                    }
                    assert_eq!(actual.len(), w * h + 2 * cw * ch);
                }
                assert!(player.read_frame_planes().unwrap().is_none());
            }
            assert_eq!(player.seek_to_sync(500_000_000).unwrap(), 333_000_000);
            assert_eq!(player.read_frame_planes().unwrap().unwrap().width, 17);
            let mut raw = WebmVideoReader::open_software(Cursor::new(source), 32 << 20).unwrap();
            let frame = raw.read_frame_raw().unwrap().unwrap();
            if depth > 8 {
                let RawFrame::Planar(p) = frame else {
                    panic!("lost original depth")
                };
                assert_eq!(p.frame.subsampling, Some(factors));
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.data, oracle);
            } else {
                assert!(matches!(frame, RawFrame::Planar8(_)));
            }
        }
    }
}
#[test]
fn mp4_packed_output_preserves_every_chroma_sample_and_depth() {
    use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
    use std::io::Cursor;
    for depth in [8, 10, 12] {
        // vpcC has no 4:4:0 code; that geometry is qualified through WebM.
        for (chroma, factors) in [("420", [2, 2]), ("422", [2, 1]), ("444", [1, 1])] {
            let base = format!(
                "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}",
                env!("CARGO_MANIFEST_DIR")
            );
            let source = std::fs::read(format!("{base}-synthetic.mp4")).unwrap();
            let oracle = std::fs::read(format!("{base}-coloured.yuv16")).unwrap();
            let mut reader =
                Mp4VideoReader::open_software(Cursor::new(source), Limits::default(), 32 << 20)
                    .unwrap();
            let f = reader.read_frame().unwrap().unwrap();
            let p = f.packed.unwrap();
            assert_eq!(p.depth, depth);
            assert_eq!((p.colour.kr, p.colour.kb), (0.299, 0.114));
            assert_eq!(p.frame.subsampling, Some(factors));
            assert_eq!(
                p.frame.data,
                if depth == 8 {
                    oracle.chunks_exact(2).map(|p| p[0]).collect::<Vec<_>>()
                } else {
                    oracle
                }
            );
            let planar = f.planes8.unwrap();
            assert_eq!(
                [planar.chroma_width, planar.chroma_height],
                [64 / factors[0], 64 / factors[1]]
            );
            assert!(reader.read_frame().unwrap().is_none());
        }
    }
}
#[test]
fn webm_rgb_uses_full_chroma_geometry_at_every_sample_depth() {
    use fvid::playback_webm::WebmVideoReader;
    use std::io::Cursor;
    for depth in [8, 10, 12] {
        for (chroma, [sx, sy]) in [
            ("420", [2, 2]),
            ("422", [2, 1]),
            ("444", [1, 1]),
            ("440", [1, 2]),
        ] {
            let base = format!(
                "{}/tests/fixtures/playback-errors/vp9-chroma-{chroma}-{depth}",
                env!("CARGO_MANIFEST_DIR")
            );
            let source = std::fs::read(format!("{base}-synthetic.webm")).unwrap();
            let oracle = std::fs::read(format!("{base}-coloured.yuv16")).unwrap();
            let values: Vec<_> = oracle
                .chunks_exact(2)
                .map(|p| f64::from(u16::from_le_bytes([p[0], p[1]])))
                .collect();
            let cw = 64 / sx;
            let ch = 64 / sy;
            let scale = f64::from(1u32 << (depth - 8));
            let mut expected = Vec::new();
            for y in 0..64 {
                for x in 0..64 {
                    let luma = (values[y * 64 + x] - 16.0 * scale) / (219.0 * scale);
                    let index = (y / sy) * cw + x / sx;
                    let u = (values[4096 + index] - 128.0 * scale) / (224.0 * scale);
                    let v = (values[4096 + cw * ch + index] - 128.0 * scale) / (224.0 * scale);
                    let r = luma + 1.402 * v;
                    let b = luma + 1.772 * u;
                    let g = (luma - 0.299 * r - 0.114 * b) / 0.587;
                    expected.extend([r, g, b].map(|s| (s * 255.0).round().clamp(0.0, 255.0) as u8));
                }
            }
            let mut player = WebmVideoReader::open_software(Cursor::new(source), 32 << 20).unwrap();
            assert!(player.read_frame().unwrap());
            assert_eq!(player.rgb(), expected, "{chroma} {depth}");
        }
    }
}

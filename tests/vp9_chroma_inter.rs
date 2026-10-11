use fvid::{
    codec::vp9_decoder::Decoder,
    container::webm::{Limits, WebmReader},
};
use std::io::Cursor;
fn cases() -> Vec<serde_json::Value> {
    let cases = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(format!(
            "{}/tests/fixtures/playback-errors/vp9-chroma-inter.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(cases.len(), 96);
    cases
}
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn full_chroma_motion_matches_every_independently_interpolated_sample() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let mut source = WebmReader::open(Cursor::new(data(name)), Limits::default()).unwrap();
        source.scan_all().unwrap();
        assert_eq!(source.packets.len(), 2);
        let key = source.read_packet(0).unwrap();
        let inter = source.read_packet(1).unwrap();
        let expected = data(c["oracle"].as_str().unwrap());
        let mut decoder = Decoder::new(32 << 20);
        for _ in 0..2 {
            decoder.reset();
            let initial = decoder.decode(&key).unwrap();
            let key_size = if c["key_size"].is_array() {
                [16, 16]
            } else {
                [64, 64]
            };
            assert_eq!(initial.picture.size, key_size, "{name}");
            let d = decoder
                .decode(&inter)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(d.picture.depth, c["depth"].as_u64().unwrap() as u8);
            assert_eq!(
                d.picture.size,
                if c["size"].is_array() {
                    [8, 8]
                } else {
                    [64, 64]
                }
            );
            assert!(!d.header.is_intra());
            assert!(!d.header.keyframe);
            assert_eq!(
                d.header.interpolation_filter,
                Some(c["raw_filter"].as_u64().unwrap_or(3) as u8)
            );
            assert_eq!(d.header.base_q, 0);
            let actual: Vec<_> = d
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            assert_eq!(actual.len(), expected.len(), "{name}");
            for (i, (&a, &e)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
        }
    }
}
#[test]
fn motion_playback_preserves_planes_depth_timestamps_seek_and_rewind() {
    use fvid::{playback_native::RawFrame, playback_webm::WebmVideoReader};
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let source = data(name);
        let oracle = data(c["oracle"].as_str().unwrap());
        let depth = c["depth"].as_u64().unwrap() as u8;
        let factors = match c["chroma"].as_str().unwrap() {
            "420" => [2, 2],
            "422" => [2, 1],
            "444" => [1, 1],
            "440" => [1, 2],
            _ => unreachable!(),
        };
        let [w, h]: [usize; 2] = if c["size"].is_array() {
            [8, 8]
        } else {
            [64, 64]
        };
        let expected8: Vec<_> = oracle
            .chunks_exact(2)
            .map(|p| (u16::from_le_bytes([p[0], p[1]]) >> (depth - 8)) as u8)
            .collect();
        let mut player =
            WebmVideoReader::open_software(Cursor::new(source.clone()), 32 << 20).unwrap();
        for pass in 0..3 {
            if pass == 2 {
                assert_eq!(player.seek_to_sync(500_000_000).unwrap(), 0, "{name}");
            } else {
                player.rewind();
            }
            assert!(player.read_frame_planes().unwrap().is_some());
            let frame = player.read_frame_planes().unwrap().unwrap();
            assert_eq!([frame.width, frame.height], [w, h], "{name}");
            assert_eq!(
                [frame.chroma_width, frame.chroma_height],
                [w.div_ceil(factors[0]), h.div_ceil(factors[1])]
            );
            let actual: Vec<_> = [&frame.y, &frame.cb, &frame.cr]
                .into_iter()
                .flat_map(|p| p.iter().copied())
                .collect();
            assert_eq!(actual.len(), expected8.len());
            for (i, (&a, &e)) in actual.iter().zip(&expected8).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
            let (start, end, scale) = player.frame_interval().unwrap();
            assert_eq!(start, 333_000_000);
            assert!(end > start);
            assert_eq!(scale, 1_000_000_000);
            assert!(player.read_frame_planes().unwrap().is_none());
        }
        let mut player = WebmVideoReader::open_software(Cursor::new(source), 32 << 20).unwrap();
        assert!(player.read_frame_raw().unwrap().is_some());
        let frame = player.read_frame_raw().unwrap().unwrap();
        if depth > 8 {
            let RawFrame::Planar(p) = frame else {
                panic!("{name}: lost original depth")
            };
            assert_eq!(p.depth, depth);
            assert_eq!(p.frame.subsampling, Some(factors));
            assert_eq!([p.frame.width, p.frame.height], [w, h]);
            assert_eq!(p.frame.data.len(), oracle.len());
            for (i, (&a, &e)) in p.frame.data.iter().zip(&oracle).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
        } else {
            assert!(matches!(frame, RawFrame::Planar8(_)));
        }
    }
}

use fvid::{
    codec::vp9_decoder::Decoder,
    container::webm::{Limits, WebmReader},
};
use std::{io::Cursor, path::Path};
fn packets(kind: &str) -> Vec<Vec<u8>> {
    let data = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(format!("vp9-scaled-reference-{kind}-synthetic.webm")),
    )
    .unwrap();
    let mut reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
    reader.scan_all().unwrap();
    (0..reader.packets.len())
        .map(|i| reader.read_packet(i).unwrap())
        .collect()
}
#[test]
fn resized_references_decode_both_directions_and_reset_replays_every_sample() {
    for (kind, size) in [
        ("horizontal", [31, 32]),
        ("vertical", [32, 31]),
        ("both", [31, 31]),
    ] {
        let packets = packets(kind);
        assert_eq!(packets.len(), 3);
        let mut decoder = Decoder::new(32 << 20);
        let mut oracle = Vec::new();
        for pass in 0..2 {
            decoder.reset();
            for (i, packet) in packets.iter().enumerate() {
                let frame = decoder
                    .decode(packet)
                    .unwrap_or_else(|e| panic!("{kind} picture {i}: {e}"));
                assert_eq!(frame.picture.size, if i == 1 { size } else { [32, 32] });
                let samples: Vec<_> = frame
                    .picture
                    .planes
                    .iter()
                    .enumerate()
                    .flat_map(|(plane, p)| {
                        let sub = if plane == 0 { 1 } else { 2 };
                        let w = frame.picture.size[0].div_ceil(sub) as usize;
                        let h = frame.picture.size[1].div_ceil(sub) as usize;
                        (0..h).flat_map(move |y| {
                            p.samples[y * p.width..y * p.width + w].iter().copied()
                        })
                    })
                    .collect();
                if i == 0 {
                    assert_eq!(
                        samples,
                        include_bytes!("fixtures/vp9/header-first.yuv")
                            .iter()
                            .map(|v| u16::from(*v))
                            .collect::<Vec<_>>()
                    );
                }
                if pass == 0 {
                    oracle.push(samples);
                } else {
                    assert_eq!(samples, oracle[i], "{kind} picture {i}");
                }
            }
        }
    }
}
#[test]
fn reference_ratio_violation_refuses_before_prediction_and_requires_reset() {
    let packets = packets("bad-ratio");
    let mut decoder = Decoder::new(32 << 20);
    for _ in 0..2 {
        decoder.decode(&packets[0]).unwrap();
        let error = decoder
            .decode(&packets[1])
            .err()
            .expect("invalid ratio must fail");
        assert!(matches!(error, fvid::Error::Invalid(_)));
        assert_eq!(error.to_string(), "VP9 reference scale out of range");
        assert!(decoder.decode(&packets[0]).is_err());
        decoder.reset();
    }
}

#[test]
fn resized_webm_playback_seek_and_rewind_match_direct_decode_and_timestamps() {
    use fvid::playback_webm::WebmVideoReader;
    for kind in ["horizontal", "vertical", "both"] {
        let source = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(format!("vp9-scaled-reference-{kind}-synthetic.webm")),
        )
        .unwrap();
        let mut player = WebmVideoReader::open_software(Cursor::new(source), 32 << 20).unwrap();
        let packets = packets(kind);
        for target in [None, Some(500_000_000), None] {
            if let Some(time) = target {
                assert_eq!(player.seek_to_sync(time).unwrap(), 0);
            } else {
                player.rewind();
            }
            let mut decoder = Decoder::new(32 << 20);
            for (index, packet) in packets.iter().enumerate() {
                let gold = decoder.decode(packet).unwrap();
                let frame = player.read_frame_planes().unwrap().unwrap();
                assert_eq!(
                    [frame.width, frame.height],
                    gold.picture.size.map(|v| v as usize)
                );
                for (plane, actual) in [&frame.y, &frame.cb, &frame.cr].into_iter().enumerate() {
                    let p = &gold.picture.planes[plane];
                    let sub = if plane == 0 { 1 } else { 2 };
                    let w = gold.picture.size[0].div_ceil(sub) as usize;
                    let h = gold.picture.size[1].div_ceil(sub) as usize;
                    let expected: Vec<_> = (0..h)
                        .flat_map(|y| {
                            p.samples[y * p.width..y * p.width + w]
                                .iter()
                                .map(|v| *v as u8)
                        })
                        .collect();
                    assert_eq!(*actual, expected);
                }
                let (start, end, scale) = player.frame_interval().unwrap();
                assert_eq!(start, index as u128 * 333_000_000);
                assert!(end > start);
                assert_eq!(scale, 1_000_000_000);
            }
            assert!(player.read_frame_planes().unwrap().is_none());
        }
    }
}

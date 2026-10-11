use fvid::{codec::vp9_decoder::Decoder, playback_webm::WebmVideoReader};
use std::io::Cursor;
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&data("vp9-ac.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn equal(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a, e, "{name} byte {i}");
    }
}
#[test]
fn signed_ac_tokens_zero_runs_scan_and_segment_quantizers_match_matrix_oracle() {
    let cases = cases();
    assert_eq!(cases.len(), 48);
    for c in cases {
        for frame in c["frames"].as_array().unwrap() {
            let name = frame["file"].as_str().unwrap();
            let mut d = Decoder::new(32 << 20);
            for _ in 0..2 {
                d.reset();
                let f = d
                    .decode(&data(name))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(f.header.lossless(), c["base"] == 0);
                assert_eq!(f.picture.depth, c["depth"].as_u64().unwrap() as u8);
                assert_eq!(f.header.segmentation.enabled, c["q"] != 0);
                let actual: Vec<_> = f
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                equal(&actual, &data(frame["oracle"].as_str().unwrap()), name);
            }
        }
    }
}
#[test]
fn ac_playback_preserves_planes_depth_timestamps_rewind_and_sync_seek() {
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let mut p = WebmVideoReader::open_software(Cursor::new(data(name)), 32 << 20).unwrap();
        for pass in 0..3 {
            let first = if pass == 2 {
                assert_eq!(p.seek_to_sync(700_000_000).unwrap(), 666_000_000);
                2
            } else {
                p.rewind();
                0
            };
            for (i, f) in c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .skip(first)
            {
                let actual = p.read_frame_planes().unwrap().unwrap();
                let actual: Vec<_> = [actual.y, actual.cb, actual.cr]
                    .into_iter()
                    .flatten()
                    .collect();
                let oracle = data(f["oracle"].as_str().unwrap());
                let expected: Vec<_> = oracle
                    .chunks_exact(2)
                    .map(|v| (u16::from_le_bytes([v[0], v[1]]) >> (depth - 8)) as u8)
                    .collect();
                equal(&actual, &expected, name);
                assert_eq!(p.frame_interval().unwrap().0, (i as u128) * 333_000_000);
            }
            assert!(p.read_frame_planes().unwrap().is_none());
        }
    }
}

#[test]
fn inter_ac_residuals_match_matrix_oracle_with_persistent_neutral_reference() {
    use fvid::container::webm::{Limits, WebmReader};
    for c in cases() {
        let name = c["inter_file"].as_str().unwrap();
        let mut r = WebmReader::open(Cursor::new(data(name)), Limits::default()).unwrap();
        r.scan_all().unwrap();
        let packets: Vec<_> = (0..r.packets.len())
            .map(|i| r.read_packet(i).unwrap())
            .collect();
        assert_eq!(packets.len(), 5);
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            let key = d.decode(&packets[0]).unwrap();
            assert!(
                key.picture
                    .planes
                    .iter()
                    .all(|p| p.samples.iter().all(|&v| v == 1 << (key.picture.depth - 1)))
            );
            for (i, f) in c["inter_frames"].as_array().unwrap().iter().enumerate() {
                let output = d
                    .decode(&packets[i + 1])
                    .unwrap_or_else(|e| panic!("{name} frame {i}: {e}"));
                assert!(!output.header.is_intra());
                assert_eq!(output.header.refresh_references, 0);
                let actual: Vec<_> = output
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                equal(&actual, &data(f["oracle"].as_str().unwrap()), name);
            }
        }
    }
}

fn raw_equal(frame: fvid::playback_native::RawFrame, depth: u8, expected: &[u8], name: &str) {
    use fvid::playback_native::RawFrame;
    match frame {
        RawFrame::Planar(p) => {
            assert_eq!(p.depth, depth);
            equal(&p.frame.data, expected, name);
        }
        RawFrame::Planar8(p) => {
            assert_eq!(depth, 8);
            let actual: Vec<_> = [&p.y, &p.cb, &p.cr]
                .into_iter()
                .flat_map(|v| v.iter().copied())
                .collect();
            let expected: Vec<_> = expected
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]) as u8)
                .collect();
            equal(&actual, &expected, name);
        }
        _ => panic!("{name}: expected original planar sample depth"),
    }
}
#[test]
fn ac_raw_playback_and_inter_seek_preserve_original_depth_and_every_sample() {
    for c in cases() {
        let depth = c["depth"].as_u64().unwrap() as u8;
        for (field, frames, offset) in [
            ("file", "frames", 0),
            ("inter_file", "inter_frames", 1),
            ("compound_file", "compound_frames", 2),
        ] {
            let inter = offset > 0;
            let name = c[field].as_str().unwrap();
            let mut p = WebmVideoReader::open_software(Cursor::new(data(name)), 32 << 20).unwrap();
            for pass in 0..3 {
                let first = if pass == 2 {
                    let sync = p.seek_to_sync(700_000_000).unwrap();
                    assert_eq!(sync, if inter { 0 } else { 666_000_000 });
                    if inter { 0 } else { 2 }
                } else {
                    p.rewind();
                    0
                };
                if inter {
                    assert!(p.read_frame_raw().unwrap().is_some());
                }
                for (i, f) in c[frames].as_array().unwrap().iter().enumerate().skip(first) {
                    raw_equal(
                        p.read_frame_raw().unwrap().unwrap(),
                        depth,
                        &data(f["oracle"].as_str().unwrap()),
                        name,
                    );
                    assert_eq!(
                        p.frame_interval().unwrap().0,
                        ((i + offset) as u128) * 333_000_000
                    );
                }
                assert!(p.read_frame_raw().unwrap().is_none());
            }
        }
    }
}

#[test]
fn compound_ac_residuals_are_added_after_distinct_reference_averaging() {
    use fvid::{
        codec::vp9_probs::{CompressedHeader, Probabilities, ReferenceMode},
        container::webm::{Limits, WebmReader},
    };
    for c in cases() {
        let name = c["compound_file"].as_str().unwrap();
        let mut r = WebmReader::open(Cursor::new(data(name)), Limits::default()).unwrap();
        r.scan_all().unwrap();
        let packets: Vec<_> = (0..r.packets.len())
            .map(|i| r.read_packet(i).unwrap())
            .collect();
        assert_eq!(packets.len(), 6);
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            d.decode(&packets[0]).unwrap();
            let hidden = d.decode(&packets[1]).unwrap();
            assert!(!hidden.header.show_frame);
            for (i, f) in c["compound_frames"].as_array().unwrap().iter().enumerate() {
                let output = d
                    .decode(&packets[i + 2])
                    .unwrap_or_else(|e| panic!("{name} frame {i}: {e}"));
                let h = CompressedHeader::parse(
                    &packets[i + 2],
                    &output.header,
                    &Probabilities::default(),
                )
                .unwrap();
                assert_eq!(h.reference_mode, ReferenceMode::Compound);
                let actual: Vec<_> = output
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                equal(&actual, &data(f["oracle"].as_str().unwrap()), name);
            }
        }
    }
}

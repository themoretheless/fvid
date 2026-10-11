use fvid::{
    codec::{
        vp9_decoder::Decoder,
        vp9_probs::{CompressedHeader, Probabilities, ReferenceMode},
    },
    container::webm::{Limits, WebmReader},
};
use std::io::Cursor;
#[test]
fn distinct_references_compound_and_select_match_every_sample() {
    let root = format!(
        "{}/tests/fixtures/playback-errors",
        env!("CARGO_MANIFEST_DIR")
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{root}/vp9-compound.json")).unwrap())
            .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 288);
    for c in cases {
        let name = c["file"].as_str().unwrap();
        let mut r = WebmReader::open(
            Cursor::new(std::fs::read(format!("{root}/{name}")).unwrap()),
            Limits::default(),
        )
        .unwrap();
        r.scan_all().unwrap();
        assert_eq!(
            r.packets.len(),
            if c["moving"].as_bool().unwrap() { 4 } else { 3 }
        );
        let packets: Vec<_> = (0..r.packets.len())
            .map(|i| r.read_packet(i).unwrap())
            .collect();
        let oracle = std::fs::read(format!("{root}/{}", c["oracle"].as_str().unwrap())).unwrap();
        let mut d = Decoder::new(32 << 20);
        for _ in 0..2 {
            d.reset();
            d.decode(&packets[0]).unwrap();
            let hidden = d.decode(&packets[1]).unwrap();
            assert!(!hidden.header.show_frame);
            assert!(hidden.header.intra_only);
            let h = d
                .decode(&packets[2])
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let ch =
                CompressedHeader::parse(&packets[2], &h.header, &Probabilities::default()).unwrap();
            assert_eq!(
                ch.reference_mode,
                if c["select"].as_bool().unwrap() {
                    ReferenceMode::Select
                } else {
                    ReferenceMode::Compound
                }
            );
            let actual: Vec<_> = h
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                .collect();
            assert_eq!(actual.len(), oracle.len());
            for (i, (&a, &e)) in actual.iter().zip(&oracle).enumerate() {
                assert_eq!(a, e, "{name} byte {i}");
            }
            if let Some(name) = c["continuation_oracle"].as_str() {
                let next = d.decode(&packets[3]).unwrap();
                let expected = std::fs::read(format!("{root}/{name}")).unwrap();
                let actual: Vec<_> = next
                    .picture
                    .planes
                    .iter()
                    .flat_map(|p| p.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                assert_eq!(actual.len(), expected.len());
                for (i, (&a, &e)) in actual.iter().zip(&expected).enumerate() {
                    assert_eq!(a, e, "{name} byte {i}");
                }
            }
        }
    }
}

#[test]
fn compound_playback_skips_hidden_reference_and_preserves_seek_timestamps() {
    use fvid::{playback_native::RawFrame, playback_webm::WebmVideoReader};
    let root = format!(
        "{}/tests/fixtures/playback-errors",
        env!("CARGO_MANIFEST_DIR")
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(format!("{root}/vp9-compound.json")).unwrap())
            .unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = std::fs::read(format!("{root}/{name}")).unwrap();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let mut player = WebmVideoReader::open_software(Cursor::new(bytes), 32 << 20).unwrap();
        for pass in 0..3 {
            if pass == 2 {
                assert_eq!(player.seek_to_sync(700_000_000).unwrap(), 0, "{name}");
            } else {
                player.rewind();
            }
            assert!(player.read_frame_raw().unwrap().is_some());
            for (field, timestamp) in [
                ("oracle", 666_000_000),
                ("continuation_oracle", 999_000_000),
            ] {
                let Some(oracle_name) = c[field].as_str() else {
                    continue;
                };
                let oracle = std::fs::read(format!("{root}/{oracle_name}")).unwrap();
                let frame = player.read_frame_raw().unwrap().unwrap();
                match frame {
                    RawFrame::Planar(p) => {
                        assert_eq!(p.depth, depth);
                        assert_eq!(p.frame.data, oracle, "{name}");
                    }
                    RawFrame::Planar8(p) => {
                        assert_eq!(depth, 8);
                        let actual: Vec<_> = [&p.y, &p.cb, &p.cr]
                            .into_iter()
                            .flat_map(|v| v.iter().copied())
                            .collect();
                        let expected: Vec<_> = oracle
                            .chunks_exact(2)
                            .map(|v| u16::from_le_bytes([v[0], v[1]]) as u8)
                            .collect();
                        assert_eq!(actual, expected, "{name}");
                    }
                    _ => panic!("{name}: unexpected frame format"),
                }
                let (start, end, scale) = player.frame_interval().unwrap();
                assert_eq!(start, timestamp, "{name}");
                assert!(end > start);
                assert_eq!(scale, 1_000_000_000);
            }
            assert!(player.read_frame_raw().unwrap().is_none(), "{name}");
        }
    }
}

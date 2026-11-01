use fvid::codec::av1_decoder::{Decoded, Decoder};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 16 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/av1-reserved-obus")
}
fn signature(f: &Decoded) -> (bool, u8, [u32; 2], u8, Vec<u16>, Vec<u8>) {
    (
        f.show,
        f.spatial_id,
        f.picture.size,
        f.picture.depth,
        f.picture
            .planes
            .iter()
            .flat_map(|p| p.samples.iter().copied())
            .collect(),
        f.picture.segment_ids.clone(),
    )
}
#[test]
fn reserved_obus_preserve_hidden_references_pending_frames_hdr_and_pixels() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 16);
    let control = std::fs::read(root().join("control.obu")).unwrap();
    let mut baseline = Decoder::new(BUDGET);
    let expected = baseline.decode_packet(&control).unwrap();
    assert_eq!(expected.len(), 3);
    assert!(!expected[0].show && expected[1].show && expected[2].show);
    assert_eq!(baseline.hdr().light.max_cll, 1000.0);
    assert_eq!(baseline.hdr().light.max_fall, 200.0);
    let pixels: Vec<u8> = expected
        .iter()
        .filter(|f| f.show)
        .flat_map(|f| {
            f.picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|v| *v as u8))
        })
        .collect();
    assert_eq!(pixels, std::fs::read(root().join("display.yuv")).unwrap());
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = std::fs::read(root().join(format!("{name}.obu"))).unwrap();
        let count = fvid::codec::av1::Obus::new(&data)
            .map(Result::unwrap)
            .filter(|o| u64::from(o.kind) == c["kind"].as_u64().unwrap())
            .count();
        assert_eq!(count as u64, c["insertions"].as_u64().unwrap());
        let mut decoder = Decoder::new(BUDGET);
        for _ in 0..2 {
            let actual = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                actual.iter().map(signature).collect::<Vec<_>>(),
                expected.iter().map(signature).collect::<Vec<_>>(),
                "{name}"
            );
            assert_eq!(decoder.hdr(), baseline.hdr());
            decoder.reset();
            assert!(decoder.hdr().is_empty());
        }
        let mut at = 0;
        for (i, n) in c["packet_bytes"].as_array().unwrap().iter().enumerate() {
            let n = n.as_u64().unwrap() as usize;
            let actual = decoder.decode_packet(&data[at..at + n]).unwrap();
            assert_eq!(actual.len(), 1);
            assert_eq!(signature(&actual[0]), signature(&expected[i]));
            at += n;
        }
        assert_eq!(at, data.len());
        assert_eq!(decoder.hdr(), baseline.hdr());
    }
}
fn frames(
    reader: &mut fvid::playback_webm::WebmVideoReader<Cursor<Vec<u8>>>,
) -> Vec<(Vec<u8>, Option<(u128, u128, u32)>)> {
    let mut out = Vec::new();
    while let Some(f) = reader.read_frame_planes().unwrap() {
        out.push((
            f.y.iter().chain(&f.cb).chain(&f.cr).copied().collect(),
            reader.frame_interval(),
        ));
    }
    out
}
#[test]
fn reserved_obu_webm_payloads_are_retained_and_play_rewind_seek_identically() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let control = std::fs::read(root().join("control.webm")).unwrap();
    let mut baseline =
        fvid::playback_webm::WebmVideoReader::open_software(Cursor::new(control), BUDGET).unwrap();
    let expected = frames(&mut baseline);
    assert_eq!(expected.len(), 2);
    assert_eq!(
        expected
            .iter()
            .flat_map(|r| r.0.iter().copied())
            .collect::<Vec<_>>(),
        std::fs::read(root().join("display.yuv")).unwrap()
    );
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = std::fs::read(root().join(format!("{name}.webm"))).unwrap();
        // A wrapper that silently drops unknown OBUs cannot satisfy this test.
        let mut container =
            fvid::container::webm::WebmReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        container.scan_all().unwrap();
        assert_eq!(container.packets.len(), 3);
        let payloads: Vec<u8> = (0..container.packets.len())
            .flat_map(|i| {
                let p = container.read_packet(i).unwrap();
                assert_eq!(p.len() as u64, c["packet_bytes"][i].as_u64().unwrap());
                p
            })
            .collect();
        assert_eq!(
            payloads,
            std::fs::read(root().join(format!("{name}.obu"))).unwrap()
        );
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open_software(Cursor::new(data), BUDGET).unwrap();
        assert_eq!(frames(&mut reader), expected, "{name}");
        reader.rewind();
        assert_eq!(frames(&mut reader), expected);
        for target in [0, 20_000_000, 40_000_000, 0] {
            assert_eq!(
                reader.seek_to_sync(target).unwrap(),
                baseline.seek_to_sync(target).unwrap()
            );
            assert_eq!(
                frames(&mut reader),
                frames(&mut baseline),
                "{name} seek{target}"
            );
        }
    }
}

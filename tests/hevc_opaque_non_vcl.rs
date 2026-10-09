use fvid::{
    codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::HevcDecoder,
        hevc_nal::NalHeader,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
const BASE: &[u8] = include_bytes!("fixtures/playback-errors/hevc-multislice-main.mp4");
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/hevc-opaque-non-vcl.json"
    ))
    .unwrap()
}
fn video(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn opaque_non_vcl_videos_keep_slices_poc_planes_and_references() {
    for c in cases()["cases"].as_array().unwrap() {
        let data = video(c);
        let mut r = Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let mut base = Mp4Reader::open(Cursor::new(BASE), Default::default()).unwrap();
        let config = r.tracks()[0].configuration.clone();
        let h = HevcConfig::parse(&config).unwrap();
        let mut d = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
        let mut expected =
            HevcDecoder::from_configuration(&base.tracks()[0].configuration, 16 << 20).unwrap();
        for i in 0..r.tracks()[0].samples.len() {
            let mut packet = vec![];
            let mut baseline = vec![];
            r.read_packet(0, i, &mut packet).unwrap();
            base.read_packet(0, i, &mut baseline).unwrap();
            let types: Vec<_> = NalUnits::new(&packet, h.length_size)
                .unwrap()
                .map(|n| NalHeader::parse(n.unwrap()).unwrap().unit_type)
                .filter(|&t| t >= 41)
                .collect();
            assert_eq!(types, (41..=63).collect::<Vec<_>>());
            assert_eq!(d.slice_headers(&packet).unwrap().len(), 2);
            let actual = d.decode_packet(&packet).unwrap().unwrap();
            let gold = expected.decode_packet(&baseline).unwrap().unwrap();
            assert_eq!(actual.poc, gold.poc);
            assert_eq!(actual.output, gold.output);
            for p in 0..3 {
                assert_eq!(
                    actual.picture.planes[p].samples(),
                    gold.picture.planes[p].samples()
                );
            }
        }
        for t in 41..=63 {
            let mut packet = (4u32).to_be_bytes()[4 - h.length_size as usize..].to_vec();
            packet.extend_from_slice(&[t << 1, 1, t, 128]);
            assert!(d.decode_packet(&packet).unwrap().is_none());
        }
    }
}
#[test]
fn opaque_non_vcl_playback_matches_saved_yuv_after_rewind_and_seek() {
    let gold = include_bytes!("fixtures/playback-errors/hevc-multislice-main.yuv");
    for c in cases()["cases"].as_array().unwrap() {
        let data = video(c);
        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(&data),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut out = vec![];
            let mut count = 0;
            while let Some(f) = player.read_frame().unwrap() {
                f.picture.write_planar(&mut out).unwrap();
                count += 1;
            }
            assert_eq!(count, 3);
            assert_eq!(out, gold);
            if pass == 0 {
                player.rewind();
            } else if pass == 1 {
                let mut baseline = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(BASE),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                assert_eq!(player.seek_to_sync(2), baseline.seek_to_sync(2));
            }
        }
    }
}

#[test]
fn opaque_payload_does_not_bypass_header_validation_or_multilayer_refusal() {
    for c in cases()["invalid"].as_array().unwrap() {
        let data = video(c);
        let mut r = Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let mut d =
            HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
        let mut bad = vec![];
        r.read_packet(0, 0, &mut bad).unwrap();
        assert!(
            d.decode_packet(&bad)
                .err()
                .unwrap()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        let mut base = Mp4Reader::open(Cursor::new(BASE), Default::default()).unwrap();
        let mut packet = vec![];
        base.read_packet(0, 0, &mut packet).unwrap();
        assert!(
            d.decode_packet(&packet)
                .err()
                .unwrap()
                .to_string()
                .contains("requires reset")
        );
        d.reset();
        assert!(d.decode_packet(&packet).unwrap().is_some());
    }
}

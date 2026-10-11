use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::{io::Cursor, path::Path};
fn movie(name: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    Mp4Reader::open(
        Cursor::new(
            std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/playback-errors")
                    .join(name),
            )
            .unwrap(),
        ),
        Default::default(),
    )
    .unwrap()
}
#[test]
fn single_layer_vps_vui_preserves_every_reset_picture() {
    for kind in ["basic", "rates", "bsp"] {
        let mut input = movie(&format!("hevc-vps-vui-{kind}-synthetic.mp4"));
        let mut seed = movie("hevc-poc-vps-aligned-msb-reset-synthetic.mp4");
        let mut decoder =
            HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20).unwrap();
        let mut oracle =
            HevcDecoder::from_configuration(&seed.tracks()[0].configuration, 32 << 20).unwrap();
        for _ in 0..2 {
            decoder.reset();
            oracle.reset();
            for index in 0..35 {
                let mut a = Vec::new();
                let mut b = Vec::new();
                input.read_packet(0, index, &mut a).unwrap();
                seed.read_packet(0, index, &mut b).unwrap();
                let frame = decoder
                    .decode_packet(&a)
                    .unwrap_or_else(|e| panic!("{kind} picture {index}: {e}"))
                    .unwrap();
                let gold = oracle.decode_packet(&b).unwrap().unwrap();
                assert_eq!(frame.poc, gold.poc);
                for (a, b) in frame.picture.planes.iter().zip(&gold.picture.planes) {
                    assert_eq!(a.samples(), b.samples(), "{kind} picture {index}");
                }
            }
        }
    }
}
#[test]
fn malformed_vps_vui_fails_exactly_and_requires_reset() {
    for (kind, message) in [
        ("bad-alignment", "HEVC VPS VUI alignment bit is not one"),
        (
            "bad-rate",
            "HEVC VPS VUI constant picture rate out of range",
        ),
        ("bad-index", "HEVC VPS VUI video signal index out of range"),
        (
            "bad-skip",
            "HEVC VPS VUI higher-layer skip requires single-layer constraint",
        ),
        (
            "bad-hrd-count",
            "HEVC VPS VUI additional HRD count out of range",
        ),
        ("bad-sublayers", "HEVC VPS VUI HRD sublayers exceed VPS"),
    ] {
        let mut input = movie(&format!("hevc-vps-vui-{kind}-synthetic.mp4"));
        let mut decoder =
            HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        input.read_packet(0, 0, &mut packet).unwrap();
        for _ in 0..2 {
            let error = decoder
                .decode_packet(&packet)
                .err()
                .expect("invalid VUI must fail");
            assert!(matches!(error, fvid::Error::Invalid(_)));
            assert_eq!(error.to_string(), message, "{kind}");
            assert!(decoder.decode_packet(&packet).is_err());
            decoder.reset();
        }
    }
}

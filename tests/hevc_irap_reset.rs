use fvid::{
    codec::{hevc_decoder::HevcDecoder, hevc_poc::ResetSyntax},
    container::mp4::Mp4Reader,
};
use std::{io::Cursor, path::Path};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn movie(name: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    Mp4Reader::open(Cursor::new(bytes(name)), Default::default()).unwrap()
}
#[test]
fn cra_and_all_bla_types_infer_required_cycle_and_match_pixels() {
    for kind in [16, 17, 18, 21] {
        for aligned in [false, true] {
            for mode in ["cycle", "msb"] {
                let name = format!(
                    "hevc-irap-reset-{kind}-{}-{mode}-synthetic.mp4",
                    if aligned { "aligned" } else { "unaligned" }
                );
                let mut input = movie(&name);
                let mut seed = movie("hevc-pcm-mono-reference-rext8.mp4");
                let mut decoder =
                    HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20)
                        .unwrap();
                let mut oracle =
                    HevcDecoder::from_configuration(&seed.tracks()[0].configuration, 32 << 20)
                        .unwrap();
                for _ in 0..2 {
                    decoder.reset();
                    oracle.reset();
                    for index in 0..3 {
                        let mut packet = Vec::new();
                        let mut original = Vec::new();
                        input.read_packet(0, index, &mut packet).unwrap();
                        seed.read_packet(0, index, &mut original).unwrap();
                        if index == 0 {
                            let header = &decoder.slice_headers(&packet).unwrap()[0];
                            assert_eq!(header.nal.unit_type, kind);
                            assert!(header.short_term.is_empty());
                            let syntax = ResetSyntax::parse_base(
                                &header.extension,
                                true,
                                4,
                                header.nal,
                                aligned,
                            )
                            .unwrap();
                            assert_eq!(syntax.msb_cycle, Some(1));
                            assert_eq!(
                                syntax.reset,
                                if mode == "msb" {
                                    Some(fvid::codec::hevc_poc::Reset::Msb)
                                } else {
                                    None
                                }
                            );
                        }
                        let frame = decoder
                            .decode_packet(&packet)
                            .unwrap_or_else(|e| panic!("{name}, picture {index}: {e}"))
                            .unwrap();
                        let gold = oracle.decode_packet(&original).unwrap().unwrap();
                        assert_eq!(
                            frame.poc,
                            index as i32 + if mode == "cycle" { 17 } else { 1 },
                            "{name}"
                        );
                        for (a, b) in frame.picture.planes.iter().zip(&gold.picture.planes) {
                            assert_eq!(a.samples(), b.samples(), "{name} picture {index}");
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn cra_bla_reset_constraints_and_cycle_truncation_fail_exactly() {
    for kind in [16, 17, 18, 21] {
        for alignment in ["aligned", "unaligned"] {
            for (mode, message) in [
                (
                    "bad-full",
                    "HEVC internal base-layer full POC reset requires IDR",
                ),
                (
                    "bad-signalled",
                    "HEVC CRA/BLA cannot use signalled POC reset",
                ),
                ("bad-cycle-range", "HEVC reset POC cycle out of range"),
                ("bad-cycle-truncated", "truncated or oversized bit field"),
            ] {
                let name = format!("hevc-irap-reset-{kind}-{alignment}-{mode}-synthetic.mp4");
                let mut input = movie(&name);
                let mut decoder =
                    HevcDecoder::from_configuration(&input.tracks()[0].configuration, 32 << 20)
                        .unwrap();
                let mut packet = Vec::new();
                input.read_packet(0, 0, &mut packet).unwrap();
                for _ in 0..2 {
                    let error = decoder
                        .decode_packet(&packet)
                        .err()
                        .expect("invalid first picture must fail");
                    assert!(matches!(error, fvid::Error::Invalid(_)));
                    assert_eq!(error.to_string(), message, "{name}");
                    assert_eq!(
                        decoder.decode_packet(&packet).err().unwrap().to_string(),
                        "HEVC decoder requires reset after error"
                    );
                    decoder.reset();
                }
            }
        }
    }
}

#[test]
fn cra_bla_software_seek_and_rewind_preserve_pixels_and_timeline() {
    use fvid::playback_mp4::Mp4VideoReader;
    for kind in [16, 17, 18, 21] {
        for alignment in ["aligned", "unaligned"] {
            for mode in ["cycle", "msb"] {
                let name = format!("hevc-irap-reset-{kind}-{alignment}-{mode}-synthetic.mp4");
                let mut actual = Mp4VideoReader::open_software(
                    Cursor::new(bytes(&name)),
                    Default::default(),
                    32 << 20,
                )
                .unwrap();
                let mut oracle = Mp4VideoReader::open_software(
                    Cursor::new(bytes("hevc-poc-baseline-synthetic.mp4")),
                    Default::default(),
                    32 << 20,
                )
                .unwrap();
                for target in [None, Some(2), None] {
                    if let Some(time) = target {
                        assert_eq!(actual.seek_to_sync(time), oracle.seek_to_sync(time));
                    } else {
                        actual.rewind();
                        oracle.rewind();
                    }
                    for _ in 0..3 {
                        let a = actual
                            .read_frame()
                            .unwrap_or_else(|e| panic!("{name}: {e}"))
                            .unwrap();
                        let b = oracle.read_frame().unwrap().unwrap();
                        assert_eq!(a.sample_index, b.sample_index);
                        assert_eq!(a.presentation_time, b.presentation_time);
                        assert_eq!(a.duration, b.duration);
                        assert_eq!(a.picture.y, b.picture.y);
                        assert_eq!(a.picture.cb, b.picture.cb);
                        assert_eq!(a.picture.cr, b.picture.cr);
                    }
                    assert!(actual.read_frame().unwrap().is_none());
                }
            }
        }
    }
}

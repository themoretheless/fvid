use fvid::{
    codec::{hevc_decoder::HevcDecoder, hevc_poc::ResetSyntax},
    container::mp4::Mp4Reader,
};
use std::{io::Cursor, path::Path};

fn movie(name: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap();
    Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap()
}

#[test]
fn explicit_cycles_preserve_every_reset_picture_and_survive_decoder_reset() {
    for kind in ["msb-reset", "msb-reset-signalled", "full-reset"] {
        let mut actual = movie(&format!(
            "hevc-poc-vps-aligned-explicit-cycle-{kind}-synthetic.mp4"
        ));
        let mut baseline = movie(&format!("hevc-poc-vps-aligned-{kind}-synthetic.mp4"));
        let mut decoder =
            HevcDecoder::from_configuration(&actual.tracks()[0].configuration, 32 << 20).unwrap();
        let mut oracle =
            HevcDecoder::from_configuration(&baseline.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        let mut expected_packet = Vec::new();
        for _ in 0..2 {
            decoder.reset();
            oracle.reset();
            for index in 0..35 {
                actual.read_packet(0, index, &mut packet).unwrap();
                baseline
                    .read_packet(0, index, &mut expected_packet)
                    .unwrap();
                for header in decoder.slice_headers(&packet).unwrap() {
                    let syntax =
                        ResetSyntax::parse_base(&header.extension, true, 4, header.nal, true)
                            .unwrap();
                    let cycle = if kind == "full-reset" {
                        (index
                            - if index >= 33 {
                                33
                            } else if index >= 17 {
                                17
                            } else {
                                0
                            })
                            / 16
                    } else {
                        (index
                            - if index > 33 {
                                32
                            } else if index > 17 {
                                16
                            } else {
                                0
                            })
                            / 16
                    };
                    assert_eq!(
                        syntax.msb_cycle,
                        Some(cycle as u32),
                        "{kind} picture {index}"
                    );
                }
                let frame = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|error| panic!("{kind} picture {index}: {error}"))
                    .unwrap();
                let expected = oracle.decode_packet(&expected_packet).unwrap().unwrap();
                assert_eq!(frame.poc, expected.poc, "{kind} picture {index}");
                for (plane, gold) in frame.picture.planes.iter().zip(&expected.picture.planes) {
                    assert_eq!(plane.samples(), gold.samples(), "{kind} picture {index}");
                }
            }
        }
    }
}

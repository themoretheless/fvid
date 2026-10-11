use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::{io::Cursor, path::Path};

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn extracted_signalled_reset_enters_period_zero_without_prior_picture() {
    for kind in ["msb", "full"] {
        let name = format!("hevc-extracted-reset-{kind}-synthetic.mp4");
        let mut movie = Mp4Reader::open(Cursor::new(bytes(&name)), Default::default()).unwrap();
        let mut seed = Mp4Reader::open(
            Cursor::new(bytes("hevc-pcm-mono-reference-rext8.mp4")),
            Default::default(),
        )
        .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
        let mut oracle =
            HevcDecoder::from_configuration(&seed.tracks()[0].configuration, 32 << 20).unwrap();
        for _ in 0..2 {
            decoder.reset();
            oracle.reset();
            for index in 0..3 {
                let mut packet = Vec::new();
                let mut original = Vec::new();
                movie.read_packet(0, index, &mut packet).unwrap();
                seed.read_packet(0, index, &mut original).unwrap();
                let headers = decoder.slice_headers(&packet).unwrap();
                assert_eq!(headers[0].nal.unit_type, 1);
                assert_eq!(headers[0].poc_lsb, index as u32 + 1);
                if index == 0 {
                    assert!(headers[0].short_term.is_empty());
                    let syntax = fvid::codec::hevc_poc::ResetSyntax::parse_base(
                        &headers[0].extension,
                        true,
                        4,
                        headers[0].nal,
                        true,
                    )
                    .unwrap();
                    assert_eq!(syntax.period, Some(0));
                    assert_eq!(
                        syntax.reset,
                        Some(fvid::codec::hevc_poc::Reset::Signalled {
                            full: kind == "full",
                            lsb: 1
                        })
                    );
                }
                let actual = decoder
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{kind} picture {index}: {e}"))
                    .unwrap();
                let expected = oracle.decode_packet(&original).unwrap().unwrap();
                assert_eq!(actual.poc, index as i32 + 1);
                for (a, b) in actual.picture.planes.iter().zip(&expected.picture.planes) {
                    assert_eq!(a.samples(), b.samples(), "{kind} picture {index}");
                }
            }
        }
    }
}
#[test]
fn non_irap_entry_without_reset_refuses_exactly_and_recovers_on_reset() {
    let mut movie = Mp4Reader::open(
        Cursor::new(bytes("hevc-extracted-reset-bad-no-reset-synthetic.mp4")),
        Default::default(),
    )
    .unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
    let mut packet = Vec::new();
    movie.read_packet(0, 0, &mut packet).unwrap();
    for _ in 0..2 {
        let error = decoder
            .decode_packet(&packet)
            .err()
            .expect("non-IRAP entry must require reset");
        assert_eq!(
            error.to_string(),
            "HEVC stream must start at a random-access picture"
        );
        assert!(decoder.decode_packet(&packet).is_err());
        decoder.reset();
    }
}

#[test]
fn extracted_reset_software_playback_seek_and_rewind_preserve_timing() {
    use fvid::playback_mp4::Mp4VideoReader;
    for kind in ["msb", "full"] {
        let mut actual = Mp4VideoReader::open_software(
            Cursor::new(bytes(&format!("hevc-extracted-reset-{kind}-synthetic.mp4"))),
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
        for seek in [None, Some(2), None] {
            if let Some(time) = seek {
                assert_eq!(actual.seek_to_sync(time), oracle.seek_to_sync(time));
            } else {
                actual.rewind();
                oracle.rewind();
            }
            for _ in 0..3 {
                let a = actual.read_frame().unwrap().unwrap();
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

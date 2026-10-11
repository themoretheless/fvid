//! Coded-picture acceptance: explicit Annex F cycles without reset IDC syntax.
use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_pps::Pps, hevc_sps::Sps},
    container::mp4::Mp4Reader,
    playback_mp4::Mp4VideoReader,
};
use std::io::Cursor;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn movie(name: &str) -> Mp4Reader<Cursor<Vec<u8>>> {
    Mp4Reader::open(Cursor::new(fixture(name)), Default::default()).unwrap()
}
#[test]
fn explicit_cycles_without_reset_idc_decode_exact_pixels_poc_seek_and_rewind() {
    for family in ["", "multislice-", "dependent-"] {
        let name = format!("hevc-poc-{family}vps-aligned-cycle-only-cycle-only-synthetic.mp4");
        let baseline = format!("hevc-poc-{family}baseline-synthetic.mp4");
        let mut mp4 = movie(&name);
        let config = HevcConfig::parse(&mp4.tracks()[0].configuration).unwrap();
        let unit = |kind| {
            config
                .arrays
                .iter()
                .find(|a| a.nal_type == kind)
                .unwrap()
                .units[0]
        };
        let sps = Sps::parse(unit(33), 32 << 20).unwrap();
        let pps = Pps::parse(unit(34), &sps, 32 << 20).unwrap();
        assert!(pps.slice_header_extension);
        assert!(!pps.poc_reset_info_present);
        let mut decoder =
            HevcDecoder::from_configuration(&mp4.tracks()[0].configuration, 32 << 20).unwrap();
        let mut packet = Vec::new();
        for pass in 0..2 {
            if pass != 0 {
                decoder.reset();
            }
            let mut gold = Mp4VideoReader::open_software(
                Cursor::new(fixture(&baseline)),
                Default::default(),
                32 << 20,
            )
            .unwrap();
            for i in 0..3 {
                mp4.read_packet(0, i, &mut packet).unwrap();
                let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                assert_eq!(decoded.poc, 32 + i as i32, "{family} pass {pass} frame {i}");
                let expected = gold.read_frame().unwrap().unwrap();
                let planes = [
                    &expected.picture.y,
                    &expected.picture.cb,
                    &expected.picture.cr,
                ];
                for (c, plane) in planes.iter().enumerate() {
                    if sps.chroma_format == 0 && c > 0 {
                        continue;
                    }
                    assert_eq!(
                        decoded.picture.planes[c].samples(),
                        plane.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(),
                        "{family} frame {i} plane {c}"
                    );
                }
            }
        }
        let open = |name: &str| {
            Mp4VideoReader::open_software(Cursor::new(fixture(name)), Default::default(), 32 << 20)
                .unwrap()
        };
        let mut actual = open(&name);
        let mut gold = open(&baseline);
        for target in [None, Some(1), Some(2), None] {
            if let Some(pts) = target {
                assert_eq!(actual.seek_to_sync(pts), gold.seek_to_sync(pts));
            } else {
                actual.rewind();
                gold.rewind();
            }
            for _ in 0..3 {
                let a = actual.read_frame().unwrap().unwrap();
                let b = gold.read_frame().unwrap().unwrap();
                assert_eq!(
                    (a.sample_index, a.presentation_time, a.duration),
                    (b.sample_index, b.presentation_time, b.duration)
                );
                assert_eq!(
                    (&a.picture.y, &a.picture.cb, &a.picture.cr),
                    (&b.picture.y, &b.picture.cb, &b.picture.cr)
                );
            }
            assert!(actual.read_frame().unwrap().is_none());
        }
    }
}
#[test]
fn cycle_only_malformed_headers_refuse_at_the_authored_picture() {
    for family in ["", "multislice-", "dependent-"] {
        for (label, error) in [
            ("bad-cycle-truncated", "truncated or oversized bit field"),
            ("bad-cycle-overflow", "cycle out of range"),
            (
                "bad-vps-before-slices",
                "active VPS changed within sequence",
            ),
            ("bad-vps-after-slices", "changed VPS follows picture slices"),
            ("bad-slice-cycle", "slices disagree on POC reset syntax"),
        ] {
            if family.is_empty() && label == "bad-slice-cycle" {
                continue;
            }
            let name = format!("hevc-poc-{family}vps-aligned-cycle-only-{label}-synthetic.mp4");
            let mut mp4 = movie(&name);
            let mut decoder =
                HevcDecoder::from_configuration(&mp4.tracks()[0].configuration, 32 << 20).unwrap();
            let mut packet = Vec::new();
            mp4.read_packet(0, 0, &mut packet).unwrap();
            assert_eq!(decoder.decode_packet(&packet).unwrap().unwrap().poc, 32);
            mp4.read_packet(0, 1, &mut packet).unwrap();
            let failure = decoder
                .decode_packet(&packet)
                .err()
                .expect("authored bad cycle must refuse");
            assert!(
                matches!(failure, fvid::Error::Invalid(_)),
                "{name}: {failure}"
            );
            assert!(
                failure.to_string().contains(error),
                "{name}: {failure}; expected {error}"
            );
            decoder.reset();
            mp4.read_packet(0, 0, &mut packet).unwrap();
            assert_eq!(decoder.decode_packet(&packet).unwrap().unwrap().poc, 32);
        }
    }
}

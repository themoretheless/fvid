use fvid_codecs::flac::{Config, decode, frame_size};
use std::path::Path;
#[test]
fn frame_prefix_layout_preserves_all_owned_flac_profiles_and_strict_packet_decode() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-native");
    let mut count = 0;
    for entry in std::fs::read_dir(&base).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "case") {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let config = Config {
            sample_rate: u32::from_be_bytes(data[..4].try_into().unwrap()),
            channels: u16::from(data[4]),
            bits: data[5],
        };
        let packet = &data[10..];
        assert_eq!(
            frame_size(packet, config).unwrap(),
            packet.len(),
            "{}",
            path.display()
        );
        let joined = [packet, packet].concat();
        assert_eq!(
            frame_size(&joined, config).unwrap(),
            packet.len(),
            "{}: first prefix",
            path.display()
        );
        assert!(
            decode(&joined, config).is_err(),
            "{}: strict decoder must reject trailing frame",
            path.display()
        );
        let metadata = [packet, &[0x81, 0, 0, 0]].concat();
        assert_eq!(frame_size(&metadata, config).unwrap(), packet.len());
        for cut in [1, 2, packet.len() / 2] {
            assert!(
                frame_size(&packet[..packet.len() - cut], config).is_err(),
                "{}: cut{cut}",
                path.display()
            );
        }
        let mut damaged = packet.to_vec();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(
            frame_size(&damaged, config).is_err(),
            "{}: CRC",
            path.display()
        );
        count += 1;
    }
    assert_eq!(count, 1460);
}
#[test]
fn structural_flac_prefix_refuses_malformed_entropy_and_padding() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-native");
    let config = Config {
        sample_rate: 8000,
        channels: 1,
        bits: 32,
    };
    for name in [
        "subframe-padding",
        "subframe-type",
        "wasted-width",
        "residual-method",
        "partition-geometry",
        "lpc-precision",
        "lpc-shift",
        "footer-padding",
    ] {
        let packet = std::fs::read(base.join(format!("{name}.invalid"))).unwrap();
        assert!(frame_size(&packet, config).is_err(), "{name}");
        assert!(
            decode(&packet, config).is_err(),
            "{name}: native decoder guard"
        );
    }
}

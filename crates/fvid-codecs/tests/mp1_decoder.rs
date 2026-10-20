use fvid_codecs::mp1::{Decoder, Header};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/mp1-native")
}
fn packets(name: &str) -> (u32, u16, Vec<Vec<u8>>) {
    let meta = std::fs::read(root().join(format!("{name}.meta"))).unwrap();
    let rate = u32::from_be_bytes(meta[..4].try_into().unwrap());
    let channels = u16::from(meta[4]);
    let n = u32::from_be_bytes(meta[5..9].try_into().unwrap());
    assert_eq!(n, 7);
    let data = std::fs::read(root().join(format!("{name}.mp1"))).unwrap();
    let mut at = 0;
    let packets = meta[9..]
        .chunks_exact(4)
        .map(|s| {
            let size = u32::from_be_bytes(s.try_into().unwrap()) as usize;
            let packet = data[at..at + size].to_vec();
            at += size;
            packet
        })
        .collect();
    assert_eq!(at, data.len());
    (rate, channels, packets)
}
#[test]
fn layer1_all_rates_modes_allocations_scales_crc_and_free_frames_match_independent_pcm() {
    let mut checked = 0;
    for entry in std::fs::read_dir(root()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "meta") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let (rate, channels, packets) = packets(name);
        let reference = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        let mut decoder = Decoder::new(rate, channels).unwrap();
        let mut continuous = Vec::new();
        for (i, packet) in packets.iter().enumerate() {
            let pcm = decoder
                .decode(packet)
                .unwrap_or_else(|e| panic!("{name}/{i}: {e}"));
            assert_eq!(pcm.len(), 384 * usize::from(channels));
            let expected = &reference[i * pcm.len() * 4..(i + 1) * pcm.len() * 4];
            for (j, (a, b)) in pcm.iter().zip(expected.chunks_exact(4)).enumerate() {
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 2e-6, "{name}/{i}/{j}: {a} versus {b}");
            }
            continuous.push(pcm);
        }
        decoder.reset();
        for (packet, expected) in packets.iter().zip(continuous) {
            assert_eq!(decoder.decode(packet).unwrap(), expected, "{name} reset");
        }
        checked += 1;
    }
    assert_eq!(checked, 258);
}
#[test]
fn layer1_crc_reserved_allocation_truncation_and_spacing_fail_without_poisoning_history() {
    let (_, _, normal) = packets("48000-m3-e0-crc");
    for (name, reason) in [
        ("bad-crc", "CRC mismatch"),
        ("reserved-allocation", "reserved MP1 bit allocation"),
        ("truncated", "truncated MP1 payload"),
    ] {
        let mut decoder = Decoder::new(48000, 1).unwrap();
        decoder.decode(&normal[0]).unwrap();
        let mut control = decoder.clone();
        let bad = std::fs::read(root().join(format!("{name}.frame"))).unwrap();
        let error = decoder.decode(&bad).unwrap_err().to_string();
        assert!(error.contains(reason), "{name}: {error}");
        assert_eq!(
            decoder.decode(&normal[1]).unwrap(),
            control.decode(&normal[1]).unwrap()
        );
    }
    let (_, _, free) = packets("48000-m3-e0-crc-free");
    let mut decoder = Decoder::new(48000, 1).unwrap();
    decoder.decode(&free[0]).unwrap();
    let mut control = decoder.clone();
    let bad = std::fs::read(root().join("free-spacing.frame")).unwrap();
    assert!(
        decoder
            .decode(&bad)
            .unwrap_err()
            .to_string()
            .contains("length changes beyond padding")
    );
    assert_eq!(
        decoder.decode(&free[1]).unwrap(),
        control.decode(&free[1]).unwrap()
    );
    for length in 0..normal[0].len() {
        let mut decoder = Decoder::new(48000, 1).unwrap();
        assert!(
            decoder.decode(&normal[0][..length]).is_err(),
            "indexed prefix {length}"
        );
    }
}
#[test]
fn layer1_header_lengths_use_four_byte_slots_and_refuse_reserved_fields() {
    for version in [0u32, 2, 3] {
        for index in 1..15 {
            for rate_index in 0..3 {
                for padding in 0..=1 {
                    let word = (2047 << 21)
                        | (version << 19)
                        | (3 << 17)
                        | (1 << 16)
                        | (index << 12)
                        | (rate_index << 10)
                        | (padding << 9)
                        | (3 << 6);
                    let h = Header::parse(&word.to_be_bytes()).unwrap();
                    assert_eq!(h.frame_bytes % 4, 0);
                    assert_eq!(h.padding_bytes, padding as usize * 4);
                    let next = Header::parse(&(word ^ (1 << 9)).to_be_bytes()).unwrap();
                    assert_eq!(h.frame_bytes.abs_diff(next.frame_bytes), 4);
                }
            }
        }
    }
    for word in [
        0xfffe_e4c0u32 | (3 << 10),
        0xfffe_e4c0 | (15 << 12),
        (0xfffe_e4c0 & !(3 << 19)) | (1 << 19),
        0xfffe_e4c0 | 2,
    ] {
        assert!(Header::parse(&word.to_be_bytes()).is_err());
    }
}

use fvid_codecs::mp3_gapless::{Gapless, parse, tag_offset};
#[test]
fn every_flag_layout_rate_channel_and_crc_offset_preserves_gapless_fields() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-gapless/tags.bin");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"FVMP3G1\0");
    let count = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(count, 864);
    let mut at = 12;
    for case in 0..count {
        let h = &bytes[at..at + 16];
        at += 16;
        let n = u32::from_be_bytes(h[..4].try_into().unwrap()) as usize;
        let packet = &bytes[at..at + n];
        at += n;
        assert_eq!(
            tag_offset(packet).unwrap(),
            Some(u32::from_be_bytes(h[12..16].try_into().unwrap()) as usize)
        );
        assert!(
            fvid_codecs::mp3::is_info_frame(packet).unwrap(),
            "case {case}"
        );
        let info = parse(packet).unwrap().unwrap();
        assert_eq!(
            info.frames,
            if h[4] & 1 != 0 { Some(4) } else { None },
            "case {case}"
        );
        assert_eq!(
            info.bytes,
            if h[4] & 2 != 0 {
                Some(n as u32 * 5 + 2)
            } else {
                None
            }
        );
        assert_eq!(
            info.gapless,
            Some(Gapless {
                encoder_delay: 7,
                end_padding: 600
            })
        );
        // Corrupting a protected tag field must remove trim, not invent another window.
        let mut bad = packet.to_vec();
        let offset = tag_offset(packet).unwrap().unwrap();
        bad[offset + 8] ^= 1;
        if let Ok(Some(info)) = parse(&bad) {
            assert!(info.gapless.is_none());
        }
    }
    assert_eq!(at, bytes.len());
}
#[test]
fn windows_are_bounded_and_decoder_latency_moves_between_head_and_tail() {
    for delay in [0, 1, 529, 4095] {
        for padding in [0, 1, 528, 529, 600, 4095] {
            let tag = Gapless {
                encoder_delay: delay,
                end_padding: padding,
            };
            let window = tag.window(16384, 529).unwrap();
            assert_eq!(window.start, u64::from(delay) + 529);
            assert_eq!(window.end, 16384 - u64::from(padding.saturating_sub(529)));
            assert!(window.start <= window.end && window.end <= 16384);
        }
    }
    assert!(
        Gapless {
            encoder_delay: 7,
            end_padding: 600
        }
        .window(10, 529)
        .is_err()
    );
}
#[test]
fn saved_synthetic_lavc_fixed_prefix_crc_is_recognized_without_external_decoder() {
    use fvid_codecs::mp3_frame::Header;
    let p =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mp3/tone.mp3");
    let bytes = std::fs::read(p).unwrap();
    let at = 10
        + bytes[6..10]
            .iter()
            .fold(0usize, |v, &b| (v << 7) | usize::from(b & 127));
    let h = Header::parse(&bytes[at..]).unwrap();
    let info = parse(&bytes[at..at + h.frame_bytes]).unwrap().unwrap();
    let tag = info.gapless.unwrap();
    assert_eq!(
        tag,
        Gapless {
            encoder_delay: 576,
            end_padding: 1344
        }
    );
    assert_eq!(tag.window(11520, 529).unwrap(), 1105..10705);
}

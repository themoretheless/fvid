use fvid::codec::{av1::Obus, av1_decoder::Decoder};
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn all_frame_id_widths_accept_window_edges_and_wraparound() {
    let mut cases = 0;
    for delta in 2..16 {
        for width in delta + 1..=usize::min(delta + 8, 16) {
            for mode in ["edge", "wrap"] {
                let name = format!("av1-inter-id-d{delta}-w{width}-{mode}.obu");
                let bytes = fixture(&name);
                let mut decoder = Decoder::new(8 << 20);
                for _ in 0..2 {
                    let frames = decoder
                        .decode_packet(&bytes)
                        .unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert_eq!(frames.len(), if mode == "edge" { 3 } else { 2 });
                    assert_eq!(frames.iter().filter(|f| f.show).count(), 1);
                    for frame in frames {
                        assert_eq!(frame.picture.size, [32, 32]);
                        assert!(
                            frame
                                .picture
                                .planes
                                .iter()
                                .all(|p| p.samples.iter().all(|&v| v == 128))
                        );
                    }
                    decoder.reset();
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 168);
}
#[test]
fn invalid_current_frame_id_progression_has_specific_refusal() {
    for mode in ["repeat", "half-window", "backward"] {
        let bytes = fixture(&format!("av1-inter-id-invalid-current-{mode}.obu"));
        let mut end = 0;
        let mut offset = 0;
        for obu in Obus::new(&bytes) {
            let obu = obu.unwrap();
            offset = end;
            end = obu.payload.as_ptr() as usize - bytes.as_ptr() as usize + obu.payload.len();
        }
        let mut decoder = Decoder::new(8 << 20);
        for _ in 0..2 {
            let prefix = decoder.decode_packet(&bytes[..offset]).unwrap();
            assert_eq!(prefix.len(), 1);
            assert!(!prefix[0].show);
            let error = decoder.decode_packet(&bytes[offset..]).err().unwrap();
            assert!(
                error
                    .to_string()
                    .contains("AV1 invalid current frame ID progression"),
                "{mode}: {error}"
            );
            assert!(
                decoder
                    .decode_packet(&bytes[offset..])
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires reset")
            );
            decoder.reset();
        }
    }
}

#[test]
fn showing_an_existing_key_restores_current_id_before_inter_decode() {
    let bytes = fixture("av1-inter-id-shown-key-restore.obu");
    let mut decoder = Decoder::new(8 << 20);
    for _ in 0..2 {
        let frames = decoder.decode_packet(&bytes).unwrap();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames.iter().filter(|f| f.show).count(), 2);
        assert!(frames.iter().all(|f| {
            f.picture
                .planes
                .iter()
                .all(|p| p.samples.iter().all(|&v| v == 128))
        }));
        decoder.reset();
    }
}

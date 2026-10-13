//! Authored Python RFC 9043 traces, checked independently of the Rust encoder.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-rgb.json")).unwrap()).unwrap()
}
#[test]
fn rgb_encoder_and_decoder_match_independent_rfc_packets() {
    for case in manifest()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
        for _ in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
                let reference =
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap();
                let decoded = decoder.decode(&packet).unwrap();
                assert!(decoded.keyframe);
                assert_eq!(decoded.depth, depth);
                assert_eq!(decoded.frame.subsampling, None);
                assert_eq!(decoded.frame.data, reference);
                assert_eq!(decoder.monochrome(), Some(false));
                let encoded =
                    fvid_media::owned_ffv1_encoder::encode(&decoded.frame, depth).unwrap();
                assert_eq!(encoded, packet, "depth {depth}: RFC trace bytes");
            }
            decoder.reset();
        }
    }
}
#[test]
fn rgb_matroska_playback_preserves_precision_timing_rewind_and_seek() {
    for case in manifest()["cases"].as_array().unwrap() {
        let depth = case["depth"].as_u64().unwrap() as u8;
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            1 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let reference =
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap();
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let fvid::playback_native::RawFrame::Planar(p) = &raw else {
                    panic!("RGB samples lost precision")
                };
                assert_eq!(p.depth, depth);
                assert_eq!(p.frame.subsampling, None);
                assert_eq!(p.frame.data, reference);
                let actual = raw.into_rgb(1 << 20).unwrap();
                let max = (1u32 << depth) - 1;
                let expected = if depth == 8 {
                    reference
                } else {
                    reference
                        .chunks_exact(2)
                        .map(|p| {
                            ((u32::from(u16::from_le_bytes([p[0], p[1]])) * 255 + max / 2) / max)
                                as u8
                        })
                        .collect()
                };
                assert_eq!(actual, expected);
                let (start, end, scale) = reader.frame_interval().unwrap();
                assert_eq!(
                    (
                        start * 1_000_000_000 / u128::from(scale),
                        end * 1_000_000_000 / u128::from(scale)
                    ),
                    (index as u128 * 20_000_000, (index as u128 + 1) * 20_000_000)
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        assert!(
            reader
                .seek_raw(std::time::Duration::ZERO)
                .unwrap()
                .is_some()
        );
    }
}
#[test]
fn rgb_invalid_storage_and_failed_decoder_require_keyframe_recovery() {
    let packet = std::fs::read(root().join("ffv1-rgb-depth12-frame0.packet")).unwrap();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
    decoder.decode(&packet).unwrap();
    assert!(decoder.decode(&[255, 255]).is_err());
    assert_eq!(
        decoder.decode(&[0, 0]).err().unwrap(),
        "FFV1 frame requires a preceding keyframe"
    );
    assert_eq!(decoder.decode(&packet).unwrap().depth, 12);
    let mut limited = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1).unwrap();
    assert!(
        limited
            .decode(&packet)
            .err()
            .unwrap()
            .contains("memory budget")
    );
    let frame = fvid_media::owned_frame::GeometryFrame {
        width: 1,
        height: 1,
        subsampling: None,
        data: vec![0, 0, 0, 0, 0, 16],
    };
    assert_eq!(
        fvid_media::owned_ffv1_encoder::encode(&frame, 12).unwrap_err(),
        "FFV1 RGB sample exceeds depth"
    );
    for end in 0..packet.len() {
        let mut d = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
        let _ = d.decode(&packet[..end]);
    }
}
#[test]
fn malformed_rgb_headers_and_reconstruction_fail_before_frame_publication() {
    for case in manifest()["invalid"].as_array().unwrap() {
        let packet = std::fs::read(root().join(case["packet"].as_str().unwrap())).unwrap();
        let expected = case["error"].as_str().unwrap();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(4, 2, 1 << 20).unwrap();
        assert_eq!(decoder.decode(&packet).err().unwrap(), expected);
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            1 << 20,
        )
        .unwrap();
        assert_eq!(
            reader
                .read_frame_raw()
                .err()
                .expect("invalid RGB frame published")
                .to_string(),
            expected
        );
    }
}

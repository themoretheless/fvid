use fvid::codec::avc_decoder::AvcDecoder;
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/avc-si.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn si_switching_and_ordinary_intra_types_match_normative_oracle_after_reset() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut decoder =
            AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 1 << 20).unwrap();
        let expected = fixture(c["reference"].as_str().unwrap());
        for _ in 0..2 {
            let mut actual = vec![];
            for p in c["packets"].as_array().unwrap() {
                decoder
                    .decode(&hex(p.as_str().unwrap()))
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual, expected, "{}", c["kind"]);
            decoder.reset();
        }
    }
}
#[test]
fn si_constrained_neighbour_controls_exercise_chroma_availability() {
    let a = fixture("avc-si-mixed-neighbours-constrained0-reference.yuv");
    let b = fixture("avc-si-mixed-neighbours-constrained1-reference.yuv");
    assert_ne!(a, b);
    for frame in 0..3 {
        assert_eq!(
            &a[frame * 768..frame * 768 + 512],
            &b[frame * 768..frame * 768 + 512]
        );
        for offset in [512, 640] {
            for row in 0..8 {
                let start = frame * 768 + offset + row * 16;
                assert_eq!(&a[start..start + 8], &b[start..start + 8]);
                assert_eq!(&a[start + 8..start + 16], &[26; 8]);
                assert_eq!(&b[start + 8..start + 16], &[128; 8]);
            }
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn si_software_mp4_seek_and_rewind_match_normative_oracle() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(c["file"].as_str().unwrap())),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        let expected = fixture(c["reference"].as_str().unwrap());
        for target in [None, Some(1), Some(2), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            while let Some(f) = reader.read_frame().unwrap() {
                actual.extend(
                    f.picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .map(|v| *v as u8),
                );
            }
            assert_eq!(actual, expected, "{}", c["kind"]);
        }
    }
}

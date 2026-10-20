use fvid_codecs::mp2::Decoder;
fn directory() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp2-native")
}
#[test]
fn authored_mp2_quantizers_scalefactors_stereo_and_history_match_pcm() {
    let mut count = 0;
    for entry in std::fs::read_dir(directory()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "meta") {
            continue;
        }
        let meta = std::fs::read(&path).unwrap();
        let rate = u32::from_be_bytes(meta[..4].try_into().unwrap());
        let channels = u16::from(meta[4]);
        let size = u32::from_be_bytes(meta[5..9].try_into().unwrap()) as usize;
        let encoded = std::fs::read(path.with_extension("mp2")).unwrap();
        let expected = std::fs::read(path.with_extension("f32le")).unwrap();
        let expected: Vec<f32> = expected
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let mut decoder = Decoder::new(rate, channels).unwrap();
        for _ in 0..2 {
            decoder.reset();
            let mut actual = Vec::new();
            for frame in encoded.chunks_exact(size) {
                actual.extend(
                    decoder
                        .decode(frame)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
                );
            }
            assert_eq!(actual.len(), expected.len());
            assert!(expected.iter().any(|v| v.abs() > 0.001));
            for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
                assert!(
                    (a - b).abs() < 3e-6,
                    "{} sample {i}: {a} != {b}",
                    path.display()
                );
            }
        }
        count += 1;
    }
    assert_eq!(count, 174);
}
#[test]
fn mp2_failed_frame_does_not_commit_synthesis_history() {
    let raw = std::fs::read(directory().join("0-0-1.mp2")).unwrap();
    let size = raw.len() / 2;
    let first = &raw[..size];
    let second = &raw[size..];
    let mut decoder = Decoder::new(48000, 2).unwrap();
    let mut control = Decoder::new(48000, 2).unwrap();
    assert_eq!(
        decoder.decode(first).unwrap(),
        control.decode(first).unwrap()
    );
    let mut bad = second.to_vec();
    bad[6] ^= 0x10;
    assert!(decoder.decode(&bad).is_err());
    for end in [0, 1, 3, 6, second.len() - 1] {
        assert!(decoder.decode(&second[..end]).is_err());
    }
    assert_eq!(
        decoder.decode(second).unwrap(),
        control.decode(second).unwrap()
    );
}

use fvid_codecs::mp2::Decoder;
#[test]
fn free_format_mp2_packet_boundaries_padding_crc_and_pcm() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp2-native");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "ffmeta") {
            continue;
        }
        let meta = std::fs::read(&path).unwrap();
        let rate = u32::from_be_bytes(meta[..4].try_into().unwrap());
        let channels = u16::from(meta[4]);
        let input = std::fs::read(path.with_extension("mp2")).unwrap();
        let expected = std::fs::read(path.with_extension("f32le")).unwrap();
        let expected: Vec<f32> = expected
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let mut decoder = Decoder::new(rate, channels).unwrap();
        let mut actual = Vec::new();
        let mut at = 0;
        for size in meta[5..]
            .chunks_exact(4)
            .map(|v| u32::from_be_bytes(v.try_into().unwrap()) as usize)
        {
            let packet = &input[at..at + size];
            actual.extend(
                decoder
                    .decode(packet)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display())),
            );
            at += size;
        }
        assert_eq!(at, input.len());
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(&expected) {
            assert!((a - b).abs() < 3e-6, "{}: PCM difference", path.display());
        }
        count += 1;
    }
    assert!(count > 30, "missing free-format matrix");
}

#[test]
fn changed_free_length_is_transactional_and_reset_accepts_a_new_stream() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp2-native");
    let meta = std::fs::read(dir.join("free-0-0-1-48000.ffmeta")).unwrap();
    let first_size = u32::from_be_bytes(meta[5..9].try_into().unwrap()) as usize;
    let second_size = u32::from_be_bytes(meta[9..13].try_into().unwrap()) as usize;
    let bytes = std::fs::read(dir.join("free-0-0-1-48000.mp2")).unwrap();
    let first = &bytes[..first_size];
    let second = &bytes[first_size..first_size + second_size];
    let mut decoder = Decoder::new(48000, 2).unwrap();
    let mut control = Decoder::new(48000, 2).unwrap();
    let original = decoder.decode(first).unwrap();
    control.decode(first).unwrap();
    let error = decoder
        .decode(&second[..second.len() - 1])
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("frame length changes beyond padding"),
        "{error}"
    );
    assert_eq!(
        decoder.decode(second).unwrap(),
        control.decode(second).unwrap()
    );
    decoder.reset();
    let mut different = first.to_vec();
    different.push(0);
    assert_eq!(decoder.decode(&different).unwrap(), original);
}

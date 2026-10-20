#![cfg(feature = "player")]
use fvid::audio::AudioStream;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn aifc_float_samples_keep_byte_order_and_exact_seek_clock() {
    for bits in [32, 64] {
        for channels in [1, 2] {
            for suffix in ["", "-uppercase"] {
                let name = format!("aifc-float-{bits}-{channels}ch");
                let data = fixture(&format!("{name}{suffix}.aifc"));
                let expected = fixture(&format!("{name}.f32le"));
                let mut stream =
                    fvid::playback_aiff::AiffAudioReader::open(data.as_slice(), Default::default())
                        .unwrap();
                assert_eq!(stream.duration().unwrap().as_secs_f64(), 4103. / 8000.);
                for target in [0, 19, 2047, 2048, 2049, 4096, 4102, 4103, 5000, -1] {
                    let landed = target.clamp(0, 4103);
                    assert_eq!(stream.seek_to(target), landed);
                    let mut decoder = stream.make_decoder().unwrap();
                    let mut actual = Vec::new();
                    let mut cursor = landed as u64;
                    while let Some(packet) = stream.next_packet().unwrap() {
                        assert_eq!(packet.pts, cursor as i64);
                        let pcm = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                            .unwrap()
                            .packet;
                        let count = pcm.data.len() / (channels * 4);
                        assert_eq!(packet.duration, count as i64);
                        cursor += count as u64;
                        actual.extend(pcm.data);
                    }
                    assert_eq!(cursor, 4103);
                    assert_eq!(actual, expected[landed as usize * channels * 4..]);
                }
                stream.seek_to(19);
                stream.rewind();
                assert_eq!(stream.next_packet().unwrap().unwrap().pts, 0);
            }
        }
    }
}

#[test]
fn aifc_float_header_width_must_match_its_codec() {
    for bits in [32, 64] {
        let mut data = fixture(&format!("aifc-float-{bits}-1ch.aifc"));
        let comm = data.windows(4).position(|b| b == b"COMM").unwrap() + 8;
        let width = if bits == 32 { 64u16 } else { 32u16 };
        data[comm + 6..comm + 8].copy_from_slice(&width.to_be_bytes());
        let error =
            match fvid::playback_aiff::AiffAudioReader::open(data.as_slice(), Default::default()) {
                Ok(_) => panic!("contradictory float width accepted"),
                Err(error) => error.to_string(),
            };
        assert!(
            error.contains("sample size disagrees with compression type"),
            "{error}"
        );
    }
}

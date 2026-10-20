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
fn aiff_signed8_uses_player_decoder_and_exact_sample_clock() {
    for coding in ["AIFF", "NONE", "sowt"] {
        for channels in [1, 2] {
            {
                let name = format!("aiff-signed8-{coding}-{channels}ch");
                let data = fixture(&format!("{name}.aiff"));
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
                    assert!(
                        actual == expected[landed as usize * channels * 4..],
                        "AIFF {coding} {channels}ch target {target}: PCM mismatch"
                    );
                }
                stream.seek_to(19);
                stream.rewind();
                assert_eq!(stream.next_packet().unwrap().unwrap().pts, 0);
            }
        }
    }
}

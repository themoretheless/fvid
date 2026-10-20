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
fn g711_containers_present_exact_pcm_and_seek() {
    for law in ["alaw", "mulaw"] {
        for channels in [1, 2] {
            let name = format!("g711-{law}-{channels}ch");
            for container in ["avi", "wav", "au", "aifc", "uppercase.aifc"] {
                let bytes = fixture(&if container == "uppercase.aifc" {
                    format!("{name}-uppercase.aifc")
                } else {
                    format!("{name}.{container}")
                });
                let mut stream: Box<dyn AudioStream> = match container {
                    "avi" => Box::new(
                        fvid::playback_avi_audio::AviAudioReader::open(
                            bytes.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    ),
                    "wav" => Box::new(
                        fvid::playback_wav::WavAudioReader::open(
                            bytes.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    ),
                    "au" => Box::new(
                        fvid::playback_au::AuAudioReader::open(
                            bytes.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    ),
                    "aifc" | "uppercase.aifc" => Box::new(
                        fvid::playback_aiff::AiffAudioReader::open(
                            bytes.as_slice(),
                            Default::default(),
                        )
                        .unwrap(),
                    ),
                    _ => unreachable!(),
                };
                let expected: Vec<u8> = fixture(&format!("{name}.s16le"))
                    .chunks_exact(2)
                    .flat_map(|v| {
                        (i16::from_le_bytes(v.try_into().unwrap()) as f32 / 32768.).to_le_bytes()
                    })
                    .collect();
                for requested in [0, 19, 127, 128, 129, 384, 385, 1023, 1024, 2000, -1, 0] {
                    let target = requested.clamp(0, 1024);
                    assert_eq!(stream.seek_to(requested), target, "{name}.{container}");
                    let mut decoder = stream.make_decoder().unwrap();
                    let mut actual = Vec::new();
                    let mut cursor = target as u64;
                    while let Some(packet) = stream.next_packet().unwrap() {
                        assert_eq!(packet.pts, cursor as i64);
                        let pcm = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                            .unwrap()
                            .packet;
                        let frames = pcm.data.len() as u64 / (channels * 4);
                        assert_eq!(packet.duration as u64, frames, "{name}.{container}");
                        cursor += frames;
                        actual.extend(pcm.data);
                    }
                    assert_eq!(cursor, 1024);
                    assert_eq!(actual, expected[target as usize * channels as usize * 4..]);
                }
                stream.seek_to(19);
                stream.rewind();
                assert_eq!(stream.next_packet().unwrap().unwrap().pts, 0);
            }
        }
    }
}

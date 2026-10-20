#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[test]
fn authored_mp2_files_decode_through_player_factory_with_sample_pts() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp2-native");
    let mut cases = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "meta") {
            continue;
        }
        let meta = std::fs::read(&path).unwrap();
        let channels = usize::from(meta[4]);
        let raw = std::fs::read(path.with_extension("mp2")).unwrap();
        let expected = std::fs::read(path.with_extension("f32le")).unwrap();
        let expected: Vec<f32> = expected
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let mut stream =
            fvid::playback_mp3::Mp3AudioReader::open(raw.as_slice(), Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(stream.codec(), "A_MPEG/L2");
        for _ in 0..2 {
            stream.rewind();
            let mut decoder = stream.make_decoder().unwrap();
            let mut cursor = 0;
            let mut actual = Vec::new();
            while let Some(packet) = stream.next_packet().unwrap() {
                assert_eq!(packet.pts, cursor);
                assert_eq!(packet.duration, 1152);
                let pcm = decoder
                    .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                    .unwrap()
                    .unwrap()
                    .packet;
                assert_eq!(pcm.pts, cursor as u64);
                assert_eq!(pcm.data.len(), 1152 * channels * 4);
                actual.extend(
                    pcm.data
                        .chunks_exact(4)
                        .map(|v| f32::from_le_bytes(v.try_into().unwrap())),
                );
                cursor += 1152;
            }
            assert_eq!(cursor, 2304);
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(&expected) {
                assert!((a - b).abs() < 3e-6, "{}: PCM difference", path.display());
            }
        }
        cases += 1;
    }
    assert_eq!(cases, 174);
}

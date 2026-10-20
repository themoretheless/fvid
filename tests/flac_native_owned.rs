#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[test]
fn own_flac_frames_reach_player_without_symphonia() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-native");
    let mut count = 0;
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "case") {
            continue;
        }
        let case = std::fs::read(&path).unwrap();
        let depth = case[5];
        let channels = case[4] as usize;
        let expected: Vec<u8> = std::fs::read(path.with_extension("s32le"))
            .unwrap()
            .chunks_exact(4)
            .flat_map(|v| {
                ((f64::from(i32::from_le_bytes(v.try_into().unwrap()))
                    / (1u64 << (depth - 1)) as f64) as f32)
                    .to_le_bytes()
            })
            .collect();
        let data = std::fs::read(path.with_extension("flac")).unwrap();
        let mut stream =
            fvid::playback_flac::FlacAudioReader::open(data.as_slice(), Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for _ in 0..2 {
            stream.rewind();
            let mut decoder = stream.make_decoder().unwrap();
            let packet = stream.next_packet().unwrap().unwrap();
            assert_eq!(packet.pts, 0);
            assert_eq!(packet.duration, 64);
            let pcm = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap()
                .packet;
            assert_eq!(pcm.data.len(), 64 * channels * 4);
            assert!(pcm.data == expected, "{}", path.display());
            assert!(stream.next_packet().unwrap().is_none());
        }
        count += 1;
    }
    assert_eq!(count, 1460);
}

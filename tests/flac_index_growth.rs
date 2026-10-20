#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn index_growth_preserves_frame_offsets_pcm_and_short_stream_navigation() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-index-growth");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let data = std::fs::read(base.join(name)).unwrap();
        let count = case["frames"].as_u64().unwrap() as usize;
        let source = fvid::playback_flac::FlacAudioReader::open(
            Cursor::new(data.clone()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.flac().frames.len(), count);
        assert_eq!(source.flac().samples(), (count * 64) as u64);
        let mut decoder =
            fvid::codec::flac_decoder::FlacDecoder::new(source.extra_data(), 44100, 1).unwrap();
        let mut expected = Vec::new();
        for (index, frame) in source.flac().frames.iter().enumerate() {
            assert_eq!(frame.pts, (index * 64) as u64);
            assert_eq!(frame.samples, 64);
            if index != 0 {
                let previous = &source.flac().frames[index - 1];
                assert_eq!(frame.start, previous.start + previous.size);
            }
            let packet = decoder
                .decode_packet(source.flac().packet(index), frame.pts as i64, 64)
                .unwrap()
                .unwrap();
            let value = [4096.0f32, -4096.0, 8192.0][index % 3] / 32768.0;
            assert_eq!(
                packet.packet.data,
                (0..64)
                    .flat_map(|_| value.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            if count == 129 {
                expected.push((
                    Duration::from_nanos((index as u128 * 64 * 1_000_000_000 / 44100) as u64),
                    packet.packet.data,
                ));
            }
        }
        if count == 129 {
            for target in [expected[64].0, source.duration().unwrap()] {
                let opened = fvid::playback_flac::FlacAudioReader::open(
                    Cursor::new(data.clone()),
                    Default::default(),
                )
                .unwrap();
                capture::worker_check_with_seek(Box::new(opened), &expected, Some(target));
            }
        }
    }
}

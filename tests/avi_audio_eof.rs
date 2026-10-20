#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn avi_block_audio_seek_at_or_beyond_eof_does_not_replay_its_last_block() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avi-audio-eof");
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let frames = case["frames"].as_i64().unwrap();
        let block = case["block_frames"].as_i64().unwrap();
        let bytes = std::fs::read(base.join(format!("{name}.avi"))).unwrap();
        let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
        let mut source = fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let mut pcm = Vec::new();
        let mut packets = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            assert!(
                frame
                    .packet
                    .data
                    .chunks_exact(4)
                    .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
            );
            pcm.extend_from_slice(&frame.packet.data);
            packets.push((frame.packet.presentation_time(), frame.packet.data));
        }
        assert_eq!(pcm, expected, "{name} exact native audio control");
        assert_eq!(pcm.len() / 4, frames as usize);
        assert_eq!(source.duration(), Some(source.time_of(frames)));
        assert_eq!(source.seek_to(block + 1), block);
        let p = source.next_packet().unwrap().unwrap();
        assert_eq!((p.pts, p.duration), (block, block));
        assert_eq!(source.seek_to(-1), 0);
        for target in [frames, frames + 1, i64::MAX] {
            let at = source.seek_to(target);
            let replayed = source.next_packet().unwrap().is_some();
            if at != frames || replayed {
                wrong.push(format!(
                    "{name} target{target}: landed{at}, replayed={replayed}"
                ));
            }
        }
        let source =
            fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        if wrong.is_empty() {
            capture::worker_check_with_seek(
                Box::new(source),
                &packets,
                Some(Duration::from_nanos(frames as u64 * 1_000_000_000 / 8000)),
            );
        }
    }
    assert!(
        wrong.is_empty(),
        "AVI EOF seek replayed last native block: {wrong:?}"
    );
}

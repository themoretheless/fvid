#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use fvid::codec::eac3_decoder::{TransientWindow, transient::Stream};
use std::{io::Cursor, path::Path};
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-transient-stream")
            .join(n),
    )
    .unwrap()
}
fn floats(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn streaming_transients_match_whole_buffer_oracle_clocks_eof_and_reset() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["decoder_integration_accepted"], false);
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let original = floats(&read(&format!("{name}-input.f32le")));
        let expected = floats(&read(&format!("{name}-expected.f32le")));
        let mut video = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(read(&format!("{name}.mkv"))),
            Default::default(),
        )
        .unwrap();
        let packet = match video.next_step().unwrap().unwrap() {
            AudioStep::Pcm(p) => p.data,
            AudioStep::Encoded(p) => p.data,
            AudioStep::ResetDecoder => panic!("unexpected reset"),
        };
        assert_eq!(floats(&packet), original);
        let mut stream = Stream::default();
        for _ in 0..2 {
            let mut got = Vec::new();
            let mut offset = 0;
            for (size, window) in case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(case["windows"].as_array().unwrap())
            {
                let size = size.as_u64().unwrap() as usize;
                let window = if window.is_null() {
                    None
                } else {
                    Some(TransientWindow {
                        location: window["location"].as_u64().unwrap() as u16,
                        length: window["length"].as_u64().unwrap() as u8,
                    })
                };
                let chunk = stream
                    .push_frame(&original[offset..offset + size], window)
                    .unwrap();
                assert_eq!(chunk.start, got.len() as i64);
                assert!(chunk.samples.len().is_multiple_of(256));
                got.extend(chunk.samples);
                offset += size;
                assert!(
                    stream.retained_samples() <= 2304,
                    "history must remain bounded"
                );
            }
            let tail = stream.finish().unwrap();
            assert_eq!(tail.start, got.len() as i64);
            got.extend(tail.samples);
            assert_eq!(got.len(), original.len());
            assert!(stream.finish().unwrap().samples.is_empty());
            assert!(stream.push_frame(&original[..256], None).is_err());
            for (i, (&actual, &want)) in got.iter().zip(&expected).enumerate() {
                assert!(actual.is_finite());
                assert!(
                    (actual - want).abs() <= 0.000003,
                    "{name} sample{i}: {actual} vs {want}"
                );
            }
            assert_eq!(got != original, case["changes"].as_bool().unwrap());
            stream.reset();
        }
    }
}
#[test]
fn invalid_events_do_not_poison_state_and_long_stream_does_not_accumulate() {
    let mut stream = Stream::default();
    let block = vec![0.125; 256];
    assert!(stream.push_frame(&block[..255], None).is_err());
    for location in [1, 4096] {
        assert!(
            stream
                .push_frame(
                    &block,
                    Some(TransientWindow {
                        location,
                        length: 0
                    })
                )
                .is_err()
        );
    }
    for (mode, window) in [
        None,
        Some(TransientWindow {
            location: 4092,
            length: 255,
        }),
    ]
    .into_iter()
    .enumerate()
    {
        if mode != 0 {
            stream.reset();
        }
        let mut count = 0;
        for _ in 0..1000 {
            let chunk = stream.push_frame(&block, window).unwrap();
            assert_eq!(chunk.start, count);
            assert!(chunk.samples.iter().all(|&s| s == 0.125));
            count += chunk.samples.len() as i64;
            assert!(stream.retained_samples() <= 2304);
        }
        let tail = stream.finish().unwrap();
        assert_eq!(tail.start, count);
        assert!(tail.samples.iter().all(|&s| s == 0.125));
        count += tail.samples.len() as i64;
        assert_eq!(count, 256000);
    }
}

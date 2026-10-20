#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn decoded(mut source: fvid::playback_wav::WavAudioReader) -> Vec<u8> {
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        pcm.extend(frame.packet.data);
    }
    pcm
}
#[test]
fn unsigned8_wave_plays_numeric_boundaries_channels_extensible_headers_and_navigation() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wav-unsigned8");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut refused = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let rate = case["rate"].as_u64().unwrap() as u32;
        let frames = case["frames"].as_u64().unwrap() as usize;
        let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
        let video = std::fs::read(base.join(format!("{name}-video.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
        let control = std::fs::read(base.join(format!("{name}-control.wav"))).unwrap();
        assert_eq!(
            decoded(
                fvid::playback_wav::WavAudioReader::open(Cursor::new(control), Default::default())
                    .unwrap()
            ),
            expected,
            "{name} native signed16 control"
        );
        let bytes = std::fs::read(base.join(format!("{name}.wav"))).unwrap();
        let mut source = match fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        ) {
            Ok(source) => source,
            Err(error) => {
                refused.push(format!("{name}: {error}"));
                continue;
            }
        };
        assert_eq!(
            (
                source.sample_rate(),
                source.channels(),
                source.bits_per_sample()
            ),
            (rate, channels as u16, 8)
        );
        assert_eq!(source.duration(), Some(Duration::from_millis(20)));
        assert_eq!(
            decoded(
                fvid::playback_wav::WavAudioReader::open(
                    Cursor::new(bytes.clone()),
                    Default::default()
                )
                .unwrap()
            ),
            expected,
            "{name} unsigned boundaries"
        );
        assert_eq!(source.seek_to(7), 7);
        let p = source.next_packet().unwrap().unwrap();
        assert_eq!((p.pts, p.duration), (7, (frames - 7) as i64));
        let mut decoder = source.make_decoder().unwrap();
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(frame.packet.data, &expected[7 * channels * 4..]);
        assert_eq!(frame.packet.presentation_time(), source.time_of(7));
        assert_eq!(source.seek_to(i64::MAX), frames as i64);
        assert!(source.next_packet().unwrap().is_none());
        let packets = vec![(Duration::ZERO, expected)];
        capture::worker_check_with_seek(
            Box::new(
                fvid::playback_wav::WavAudioReader::open(
                    Cursor::new(bytes.clone()),
                    Default::default(),
                )
                .unwrap(),
            ),
            &packets,
            Some(Duration::ZERO),
        );
        capture::worker_check_with_seek(
            Box::new(
                fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                    .unwrap(),
            ),
            &packets,
            Some(Duration::from_millis(20)),
        );
    }
    let bad = std::fs::read(base.join("unsupported-float8.wav")).unwrap();
    let error = match fvid::playback_wav::WavAudioReader::open(Cursor::new(bad), Default::default())
    {
        Err(error) => error,
        Ok(_) => panic!("float8 must remain refused"),
    };
    assert!(
        error.to_string().contains("Wave PCM of 8 bits a sample"),
        "{error}"
    );
    assert!(
        refused.is_empty(),
        "unsigned8 has a native decoder but Wave admission refused it: {refused:?}"
    );
}

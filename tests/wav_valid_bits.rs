#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn read(mut source: fvid::playback_wav::WavAudioReader) -> Vec<u8> {
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
fn extensible_wave_uses_stored_width_for_frames_and_left_aligned_precision_for_the_signal() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wav-valid-bits");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let storage = case["storage"].as_u64().unwrap() as u16;
        let valid = case["valid"].as_u64().unwrap() as u16;
        let channels = case["channels"].as_u64().unwrap() as u16;
        let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
        let video = std::fs::read(base.join(format!("{name}-video.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
        let control = std::fs::read(base.join(format!("{name}-control.wav"))).unwrap();
        assert_eq!(
            read(
                fvid::playback_wav::WavAudioReader::open(Cursor::new(control), Default::default())
                    .unwrap()
            ),
            expected,
            "{name} exact stored-width control"
        );
        let bytes = std::fs::read(base.join(format!("{name}.wav"))).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
        assert_eq!(
            (
                info.bits_per_sample,
                info.valid_bits,
                info.block,
                info.sample_frames
            ),
            (storage, valid, channels * storage / 8, 160)
        );
        let source = match fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        ) {
            Ok(source) => source,
            Err(error) => {
                wrong.push(format!("{name}: refused {error}"));
                continue;
            }
        };
        assert_eq!(
            (
                source.bits_per_sample(),
                source.channels(),
                source.duration()
            ),
            (storage, channels, Some(Duration::from_millis(20)))
        );
        assert_eq!(
            read(source),
            expected,
            "{name} normalized amplitude including signed/unsigned extrema"
        );
        let packets = vec![(Duration::ZERO, expected)];
        capture::worker_check_with_seek(
            Box::new(
                fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                    .unwrap(),
            ),
            &packets,
            Some(Duration::from_millis(20)),
        );
    }
    for (name, needle) in [
        (
            "invalid-container12",
            "extensible PCM container width is not byte-aligned",
        ),
        (
            "invalid-valid-zero",
            "extensible PCM has invalid valid-bit precision",
        ),
        (
            "invalid-valid-too-wide",
            "extensible PCM has invalid valid-bit precision",
        ),
        (
            "invalid-float-precision",
            "extensible float precision differs from storage width",
        ),
    ] {
        let bytes = std::fs::read(base.join(format!("{name}.wav"))).unwrap();
        let error = match fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("malformed precision admitted: {name}"),
        };
        assert!(error.to_string().contains(needle), "{name}: {error}");
    }
    let float = std::fs::read(base.join("float-control.wav")).unwrap();
    assert_eq!(
        read(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(float), Default::default())
                .unwrap()
        ),
        vec![0; 640]
    );
    assert!(
        wrong.is_empty(),
        "extensible precision replaced its storage width: {wrong:?}"
    );
}

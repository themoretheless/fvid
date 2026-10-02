use fvid_media::{CancelFlag, CopyOptions, ProgressHook};
use std::{
    fs,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn adts_loudness_matches_independent_decoded_wave_and_encoded_packet_progress() {
    let dir = Temp(std::env::temp_dir().join(format!("fvid-adts-loudness-{}", std::process::id())));
    fs::create_dir_all(&dir.0).unwrap();
    for name in [
        "audio/aac-mono-44k.aac",
        "audio/aac-stereo.aac",
        "audio/aac-51-active.aac",
        "audio/aac-pce-wide8.aac",
        "playback-errors/aac-packet-prefix.aac",
    ] {
        let path = fixture(name);
        let data = fs::read(&path).unwrap();
        let reader = fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
        let mask = fvid_media::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
            .unwrap()
            .channel_mask();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let options = CopyOptions {
            max_packets: Some(3),
            progress: Some(ProgressHook::new(move |e| captured.lock().unwrap().push(e))),
            ..Default::default()
        };
        let mut raw = Vec::new();
        let mut raw_options = options.clone();
        raw_options.progress = None;
        let decoded = fvid_media::owned_aac::decode_adts_pcm(
            BufReader::new(Cursor::new(&data)),
            &mut raw,
            None,
            &raw_options,
        )
        .unwrap();
        let wave = dir.0.join("expected.wav");
        let mut payload = fvid_media::owned_wav::float_wav_header_with_mask(
            decoded.sample_rate,
            decoded.channels,
            decoded.sample_frames,
            mask,
        )
        .unwrap();
        payload.extend(raw);
        fs::write(&wave, payload).unwrap();
        let mut expected =
            fvid_media::owned_wave_loudness::measure_loudness(&wave, &Default::default()).unwrap();
        expected.backend = "owned ADTS AAC-LC loudness";
        let actual = fvid_media::measure_loudness(&path, &options).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{name}"
        );
        let events = events.lock().unwrap();
        assert_eq!(events.iter().filter(|e| e.done).count(), 1);
        assert!(events.last().unwrap().done);
        assert_eq!(events.last().unwrap().packets, 3);
        assert!(
            events
                .windows(2)
                .all(|w| w[0].packets <= w[1].packets && w[0].payload_bytes <= w[1].payload_bytes)
        );
    }
}
#[test]
fn adts_analysis_plan_and_failure_controls_do_not_report_false_completion() {
    let source = fixture("playback-errors/aac-packet-prefix.aac");
    let options = CopyOptions {
        max_packets: Some(3),
        progress: Some(ProgressHook::new(|_| panic!("plan progress"))),
        ..Default::default()
    };
    let plan = fvid_media::plan_loudness(&source, &options).unwrap();
    assert_eq!(plan.command, "loudness");
    assert!(
        !fvid_media::owned_loudnorm::supports_request(
            &source,
            Path::new("output.wav"),
            None,
            true,
            &options
        ),
        "AAC measurement must not qualify the WAVE-only normalizer"
    );
    assert!(plan.steps.iter().any(|s| s.action == "analyze"));
    assert!(!plan.steps.iter().any(|s| s.action == "write"));
    assert!(
        fvid_media::owned_adts_loudness::measure_loudness(&source, &Default::default())
            .unwrap_err()
            .contains("fill whole buffer")
    );
    let cancel = CancelFlag::new();
    let token = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let options = CopyOptions {
        cancel: Some(cancel),
        progress: Some(ProgressHook::new(move |e| {
            captured.lock().unwrap().push(e);
            if e.packets == 1 {
                token.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_adts_loudness::measure_loudness(&source, &options)
            .unwrap_err()
            .contains("cancel")
    );
    assert!(events.lock().unwrap().iter().all(|e| !e.done));
    let options = CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    assert!(fvid_media::owned_adts_loudness::measure_loudness(&source, &options).is_err());
}

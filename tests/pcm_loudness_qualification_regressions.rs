use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fvid::media::{self, CopyOptions, NormalizeTarget, ProgressHook};

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/pcm-loudness-qualification")
        .join(name)
}

#[test]
fn discrete_multichannel_and_non_standard_rate_standalone_flac_measure_and_normalize_loudness() {
    let flac_3ch = fixture("discrete-3ch-48k.flac");
    let flac_20k = fixture("mono-20k.flac");
    let options = CopyOptions::default();
    let weights_3ch = [1.0, 1.0, 1.0];
    let target = NormalizeTarget::default();

    // 1. 3-channel discrete FLAC with explicit weights:
    // Without weights, default_weights rejects >2ch as expected.
    let no_weights_err = media::measure_loudness(&flac_3ch, &options).unwrap_err();
    assert!(
        no_weights_err.contains("multichannel loudness requires explicit channel weights"),
        "unexpected error: {no_weights_err}"
    );
    let plan_3ch =
        fvid::native_plan::loudness_with_options(&flac_3ch, &options, Some(&weights_3ch)).unwrap();
    assert_eq!(plan_3ch.command, "loudness");
    let norm_plan_3ch =
        media::plan_normalize_loudness(&flac_3ch, &options, Some(&weights_3ch), target).unwrap();
    assert_eq!(norm_plan_3ch.command, "normalize-loudness");

    let events_3ch = Arc::new(Mutex::new(Vec::new()));
    let events_3ch_clone = Arc::clone(&events_3ch);
    let mut tracked_options = CopyOptions::default();
    tracked_options.progress = Some(ProgressHook::new(move |ev| {
        events_3ch_clone.lock().unwrap().push(ev);
    }));
    let measured_3ch =
        media::measure_loudness_with_weights(&flac_3ch, &tracked_options, &weights_3ch).unwrap();
    assert_eq!(measured_3ch.sample_frames, 23_040);
    assert!(measured_3ch.integrated_lufs.is_some());

    // Progress must report encoded FLAC packet count (10 frames) and payload bytes (170 bytes),
    // not internal 64 KiB WAVE PCM blocks!
    let log_3ch = events_3ch.lock().unwrap().clone();
    let final_3ch = log_3ch.last().expect("expected final progress event");
    assert!(final_3ch.done);
    assert_eq!(final_3ch.packets, 10);
    assert_eq!(final_3ch.payload_bytes, 170);

    let tmp = TempDir::new("pcm-loudness-qual");
    let out_3ch_wav = tmp.path().join("norm-3ch.wav");
    let report_3ch = media::normalize_loudness(
        &flac_3ch,
        &out_3ch_wav,
        &options,
        Some(&weights_3ch),
        target,
    )
    .unwrap();
    assert_eq!(report_3ch.sample_frames, 23_040);

    // 2. Mono 20 kHz FLAC (valid for LoudnessMeter, not in true-peak standard rate list):
    let plan_20k = media::plan_loudness(&flac_20k, &options).unwrap();
    assert_eq!(plan_20k.command, "loudness");
    let measured_20k = media::measure_loudness(&flac_20k, &options).unwrap();
    assert_eq!(measured_20k.sample_frames, 11_520);
    assert!(measured_20k.integrated_lufs.is_some());

    let out_20k_wav = tmp.path().join("norm-20k.wav");
    let report_20k =
        media::normalize_loudness(&flac_20k, &out_20k_wav, &options, None, target).unwrap();
    assert_eq!(report_20k.sample_frames, 11_520);
}

#[test]
fn matroska_loudness_scopes_packet_byte_budget_to_selected_audio_stream_and_preserves_adts_progress()
 {
    let mkv = fixture("video-4096-audio-1920.mkv");
    // Stream 0 is video (4096-byte packet), stream 1 is PCM audio (25 packets of 1920 bytes).
    let mut tight_audio = CopyOptions::default();
    tight_audio.streams = vec![1];
    tight_audio.max_packet_bytes = 1919;
    let err = media::plan_loudness(&mkv, &tight_audio).unwrap_err();
    assert!(
        err.contains("packet exceeds budget"),
        "expected audio packet budget rejection, got: {err}"
    );
    let err_measure = media::measure_loudness(&mkv, &tight_audio).unwrap_err();
    assert!(
        err_measure.contains("packet exceeds budget"),
        "expected audio packet budget rejection in measure_loudness, got: {err_measure}"
    );

    // With max_packet_bytes = 2000 (admitting 1920-byte audio packets while stream 0 has a 4096-byte video packet):
    let mut admitted_audio = CopyOptions::default();
    admitted_audio.streams = vec![1];
    admitted_audio.max_packet_bytes = 2000;
    let plan = media::plan_loudness(&mkv, &admitted_audio).unwrap();
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].index, 1);

    let measured = media::measure_loudness(&mkv, &admitted_audio).unwrap();
    assert_eq!(measured.sample_frames, 24_000);

    // Also verify ADTS loudness with max_controlled_bytes reports encoded ADTS packet count/bytes:
    let adts =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let default_events = Arc::new(Mutex::new(Vec::new()));
    let default_events_clone = Arc::clone(&default_events);
    let mut default_adts_opts = CopyOptions::default();
    default_adts_opts.progress = Some(ProgressHook::new(move |ev| {
        default_events_clone.lock().unwrap().push(ev);
    }));
    let expected_adts = media::measure_loudness(&adts, &default_adts_opts).unwrap();
    let expected_final = *default_events.lock().unwrap().last().unwrap();

    let controlled_events = Arc::new(Mutex::new(Vec::new()));
    let controlled_events_clone = Arc::clone(&controlled_events);
    let mut controlled_adts_opts = CopyOptions::default();
    controlled_adts_opts.max_controlled_bytes = Some(16 * 1024 * 1024);
    controlled_adts_opts.progress = Some(ProgressHook::new(move |ev| {
        controlled_events_clone.lock().unwrap().push(ev);
    }));
    let actual_adts = media::measure_loudness(&adts, &controlled_adts_opts).unwrap();
    let actual_final = *controlled_events.lock().unwrap().last().unwrap();
    assert_eq!(actual_adts.sample_frames, expected_adts.sample_frames);
    assert_eq!(actual_adts.integrated_lufs, expected_adts.integrated_lufs);
    assert_eq!(actual_final.packets, expected_final.packets);
    assert_eq!(actual_final.payload_bytes, expected_final.payload_bytes);
    assert!(actual_final.done);
}

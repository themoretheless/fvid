use fvid_control::{CopyOptions, ProgressHook};
use fvid_media::{
    apply_loudnorm, apply_loudnorm_dual, burn_subtitles, concat, convert_subtitles_with_controls,
    decode_video_transformed, plan_burn_subtitles, plan_concat, plan_convert_subtitles_with_controls,
    plan_loudnorm, plan_remux, plan_transcode_lossless, remux, transcode_lossless, DecodeTransform,
    LosslessTransform,
};
use fvid_media_info::{SubtitleCodec, SubtitleConvertOptions};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("standalone-loudnorm-gray-shuffleplanes-and-subtitles")
}

fn temp_path(label: &str, ext: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch588-{label}-{}-{}.{ext}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn standalone_audio_containers_execute_loudnorm_and_loudnorm_dual_cleanly() {
    let flac = fixture_dir().join("flac-48k-1pkt.flac");
    let g711 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/container-stage2-frames-and-spool-budget/g711-alaw-8k-nometa.wav");

    for (source, label, expected_frames) in [(&flac, "flac", 768u64), (&g711, "g711", 49152u64)] {
        let options = CopyOptions {
            max_packets: Some(1),
            ..Default::default()
        };
        let plan = plan_loudnorm(source, None, false, &options).unwrap();
        assert_eq!(plan.command, "loudnorm");

        let out_single = temp_path(&format!("{label}-single"), "wav");
        let _ = std::fs::remove_file(&out_single);
        let single_stats = apply_loudnorm(source, &out_single, None, &options).unwrap();
        let _ = std::fs::remove_file(&out_single);
        assert!(single_stats.backend.starts_with("fvid"));
        assert_eq!(single_stats.sample_frames, expected_frames);

        let out_dual = temp_path(&format!("{label}-dual"), "wav");
        let _ = std::fs::remove_file(&out_dual);
        let dual_stats = apply_loudnorm_dual(source, &out_dual, None, &options).unwrap();
        let _ = std::fs::remove_file(&out_dual);
        assert!(dual_stats.backend.starts_with("fvid"));
        assert_eq!(dual_stats.sample_frames, expected_frames);
    }
}

#[test]
fn monochrome_ffv1_refuses_shuffleplanes_consistently_across_decode_and_lossless() {
    let gray = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let transform = DecodeTransform {
        shuffleplanes: Some("0:0:0".into()),
        ..Default::default()
    };
    let decode_err = decode_video_transformed(&gray, transform.clone()).unwrap_err();
    assert!(
        !decode_err.contains("Y4M frame size mismatch"),
        "monochrome shuffleplanes must not fail with Y4M frame size mismatch: {decode_err}"
    );
    let lossless = LosslessTransform {
        shuffleplanes: Some("0:0:0".into()),
        ..Default::default()
    };
    let plan_err =
        plan_transcode_lossless(&gray, &lossless, &CopyOptions::default(), None).unwrap_err();
    assert!(
        !plan_err.contains("Y4M frame size mismatch"),
        "monochrome shuffleplanes plan must not fail with Y4M frame size mismatch: {plan_err}"
    );
    let out = temp_path("gray-shuffleplanes", "mkv");
    let _ = std::fs::remove_file(&out);
    let transcode_err =
        transcode_lossless(&gray, &out, lossless, &CopyOptions::default()).unwrap_err();
    assert!(!out.exists());
    assert!(
        !transcode_err.contains("Y4M frame size mismatch"),
        "monochrome shuffleplanes transcode must not fail with Y4M frame size mismatch: {transcode_err}"
    );
}

#[test]
fn subtitle_burn_plan_charges_alpha_and_crop_working_memory_and_checks_existing_output_first() {
    let mkv = fixture_dir().join("ffv1-alpha-gray-8-crop.mkv");
    let subs = fixture_dir().join("cue.srt");

    // 16x16 monochrome+alpha with PixelCropLeft=2 -> display 14x16 (672 bytes working planes, 224 bytes alpha).
    // Display geometry retains alpha working set: view.alpha = 224 bytes, view.extra_working_bytes = 448 bytes
    // (total 672 extra bytes).
    // With container metadata added (+1488 bytes), uncorrected plan_burn_subtitles omitted view.alpha +
    // view.extra_working_bytes (672 bytes) from frame_working_bytes (~21_147 bytes instead of ~21_819 bytes),
    // whereas burn_subtitles charged them in export_processed (~21_819 bytes).
    // With max_controlled_bytes = 21_500, both plan_burn_subtitles and burn_subtitles refuse at the frame
    // working set boundary!
    let tight = CopyOptions {
        max_controlled_bytes: Some(21_500),
        metadata_set: vec![("COMMENT".into(), "x".repeat(150))],
        ..Default::default()
    };
    let plan_err = plan_burn_subtitles(&mkv, &subs, &tight).unwrap_err();
    assert!(
        plan_err.contains("subtitle burn frame exceeds controlled memory budget"),
        "expected plan_burn_subtitles budget refusal for alpha+crop working set, got: {plan_err}"
    );
    let out = temp_path("burn-tight", "mkv");
    let _ = std::fs::remove_file(&out);
    let burn_err = burn_subtitles(&mkv, &out, &subs, &tight).unwrap_err();
    assert!(!out.exists());
    assert!(
        burn_err.contains("subtitle burn frame exceeds controlled memory budget"),
        "expected burn_subtitles budget refusal for alpha+crop working set, got: {burn_err}"
    );

    // With sufficient budget, both plan and burn succeed:
    let ample = CopyOptions {
        max_controlled_bytes: Some(64_000),
        ..Default::default()
    };
    plan_burn_subtitles(&mkv, &subs, &ample).unwrap();
    let stats = burn_subtitles(&mkv, &out, &subs, &ample).unwrap();
    assert_eq!(stats.video_frames, 1);

    // Calling burn_subtitles when destination already exists must fail immediately with
    // "output already exists" BEFORE running prepare() (so even a missing subtitle path is not opened):
    let missing_subs = fixture_dir().join("nonexistent-subtitle-file.srt");
    let exists_err = burn_subtitles(&mkv, &out, &missing_subs, &CopyOptions::default()).unwrap_err();
    let _ = std::fs::remove_file(&out);
    assert_eq!(exists_err, "output already exists");
}

#[test]
fn matroska_subtitle_conversion_filters_overlong_source_tags_and_validates_plan_metadata() {
    let source = fixture_dir().join("mkv-sub-overlong-writing-app.mkv");
    let sub_opts = SubtitleConvertOptions {
        streams: Vec::new(),
        codec: SubtitleCodec::Ass,
    };
    // Both plan_convert_subtitles_with_controls and convert_subtitles_with_controls must succeed
    // after filtering the overlong (>1024-byte) source WritingApp and container tag:
    let plan =
        plan_convert_subtitles_with_controls(&source, &sub_opts, &CopyOptions::default()).unwrap();
    assert_eq!(plan.command, "convert-subtitles");

    let out = temp_path("sub-overlong-tag", "mkv");
    let _ = std::fs::remove_file(&out);
    let stats =
        convert_subtitles_with_controls(&source, &out, &sub_opts, &CopyOptions::default()).unwrap();
    let _ = std::fs::remove_file(&out);
    assert_eq!(stats.cues, 1);
}

#[test]
fn adts_remux_and_concat_enforce_tag_count_and_defer_zero_progress_until_packet_admission() {
    let adts = fixture_dir().join("adts-8k-2pkt.aac");

    // 1. Tight controlled memory budget where base_bytes (16384) fits, but packet 1
    //    (16384 + 32 + pkt.len()) exceeds 16400: remux and concat must fail WITHOUT emitting
    //    any ProgressEvent!
    let events: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_events = Arc::clone(&events);
    let tight = CopyOptions {
        max_controlled_bytes: Some(16_400),
        progress: Some(ProgressHook::new(move |ev| {
            hook_events.lock().unwrap().push(ev.packets);
        })),
        ..Default::default()
    };
    let plan_err = plan_remux(&adts, &tight).unwrap_err();
    assert!(
        plan_err.contains("controlled memory budget exceeded"),
        "expected ADTS plan_remux budget refusal, got: {plan_err}"
    );
    let out_remux = temp_path("adts-remux-tight", "mka");
    let _ = std::fs::remove_file(&out_remux);
    let remux_err = remux(&adts, &out_remux, &tight).unwrap_err();
    assert!(!out_remux.exists());
    assert!(remux_err.contains("controlled memory budget exceeded"));
    assert!(
        events.lock().unwrap().is_empty(),
        "no progress events may be emitted when packet 1 fails the memory budget"
    );

    let concat_sources = vec![adts.clone(), adts.clone()];
    let concat_tight = CopyOptions {
        max_controlled_bytes: Some(32_780),
        progress: Some(ProgressHook::new({
            let hook_events = Arc::clone(&events);
            move |ev| {
                hook_events.lock().unwrap().push(ev.packets);
            }
        })),
        ..Default::default()
    };
    let concat_plan_err = plan_concat(&concat_sources, &concat_tight).unwrap_err();
    assert!(concat_plan_err.contains("controlled memory budget exceeded"));
    let out_concat = temp_path("adts-concat-tight", "mka");
    let _ = std::fs::remove_file(&out_concat);
    let concat_err = concat(&concat_sources, &out_concat, &concat_tight).unwrap_err();
    assert!(!out_concat.exists());
    assert!(concat_err.contains("controlled memory budget exceeded"));
    assert!(
        events.lock().unwrap().is_empty(),
        "no progress events may be emitted when concat packet 1 fails the memory budget"
    );
}

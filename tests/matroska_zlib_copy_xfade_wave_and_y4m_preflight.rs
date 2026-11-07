use fvid::media::{
    concat, plan_remux, plan_transcode_lossless, plan_xfade, remux, transcode_lossless, trim,
    xfade_video,
};
use fvid::media_control::{CopyOptions, ProgressEvent, ProgressHook};
use fvid::media_info::LosslessTransform;
use fvid::native_export::{concat_y4m_with_options, export_y4m_pipeline_with_options};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join(name)
}

fn temp_path(label: &str, ext: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "fvid-batch589-{label}-{}-{}.{ext}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn matroska_copy_charges_decompressed_zlib_payload_and_defers_zero_progress_until_preflight() {
    let zlib_mkv = fixture_path("matroska_copy_zlib_video.mkv");
    let data = std::fs::read(&zlib_mkv).unwrap();

    // Compressed block is ~20 bytes, decompressed payload is 4096 bytes (+ ~20 bytes read overhead).
    // Reader index + 64 KiB copy buffer is ~78 KiB (~79_924 bytes).
    // With max_controlled_bytes = 81_000, compressed size (79_924 + 20 < 81_000) would pass if
    // decompressed payload were ignored, whereas decompressed payload + overhead (79_924 + 4116 > 81_000)
    // is properly rejected and emits 0 progress events.
    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_events = Arc::clone(&events);
    let hook = ProgressHook::new(move |ev| {
        hook_events.lock().unwrap().push(ev);
    });

    let mut input = Cursor::new(&data);
    let mut output = Vec::new();
    let err = fvid_media::owned_matroska_copy::copy_with_packet_and_memory_limits(
        &mut input,
        &mut output,
        false,
        None,
        Some(&hook),
        64 << 20,
        Some(81_000),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("controlled memory budget exceeded"),
        "unexpected error: {err}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "failed Matroska copy preflight must not emit a 0-packet progress event"
    );

    // Also verify plan_remux and remux agree on the same budget.
    let opts = CopyOptions {
        max_controlled_bytes: Some(81_000),
        progress: Some(hook),
        ..Default::default()
    };
    let plan_err = plan_remux(&zlib_mkv, &opts).unwrap_err();
    assert!(plan_err.to_string().contains("controlled memory budget exceeded"));
    let out_mkv = temp_path("zlib-copy", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    let remux_err = remux(&zlib_mkv, &out_mkv, &opts).unwrap_err();
    assert!(remux_err.to_string().contains("controlled memory budget exceeded"));
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn xfade_plan_validates_colorrange_and_metadata_and_checks_existing_output_first() {
    let bad_y4m = fixture_path("xfade_bad_colorrange.y4m");
    let opts = CopyOptions::default();

    let plan_err = plan_xfade(&bad_y4m, &bad_y4m, "fade", 40_000, 0, &opts).unwrap_err();
    assert!(
        plan_err.to_string().contains("invalid Y4M colour range"),
        "expected plan_xfade to reject invalid XCOLORRANGE, got: {plan_err}"
    );

    // Existing destination must be reported before opening missing inputs.
    let existing = temp_path("xfade-existing", "mkv");
    std::fs::write(&existing, b"already-here").unwrap();
    let missing = temp_path("xfade-missing", "y4m");
    let _ = std::fs::remove_file(&missing);
    let xfade_err = xfade_video(
        &missing,
        &missing,
        &existing,
        "fade",
        40_000,
        0,
        &opts,
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&existing);
    assert!(
        xfade_err.to_string().contains("output already exists"),
        "expected output already exists before opening missing sources, got: {xfade_err}"
    );
}

#[test]
fn wave_remux_and_concat_defer_initial_progress_until_preflight_succeeds() {
    let wav = fixture_path("batch589_pcm16_8k.wav");
    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_events = Arc::clone(&events);
    let hook = ProgressHook::new(move |ev| {
        hook_events.lock().unwrap().push(ev);
    });

    // Packet limit smaller than one sample frame must fail without emitting progress.
    let tiny_packet_opts = CopyOptions {
        max_packet_bytes: 1,
        progress: Some(hook.clone()),
        ..Default::default()
    };
    let out_wav = temp_path("wave-remux-fail", "wav");
    let _ = std::fs::remove_file(&out_wav);
    let err = remux(&wav, &out_wav, &tiny_packet_opts).unwrap_err();
    assert!(
        err.to_string()
            .contains("PCM packet limit cannot hold one sample frame"),
        "unexpected error: {err}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "failed WAVE remux preflight must not emit a 0-packet progress event"
    );

    // Non-representable microsecond trim boundary must also fail without emitting progress.
    let bad_trim_opts = CopyOptions {
        progress: Some(hook),
        ..Default::default()
    };
    let err2 = trim(&wav, &out_wav, 1, 100, &bad_trim_opts).unwrap_err();
    assert!(
        err2.to_string()
            .contains("PCM time boundary is not exactly representable"),
        "unexpected error: {err2}"
    );
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn mp4_multitrack_lossless_qualify_charges_source_mp4_metadata_and_scoped_stream_tags() {
    let mp4 = fixture_path("multitrack-avc-mp4-stream-metadata/avc-dual-aac-tracks.mp4");
    let transform = LosslessTransform::default();

    // Set 3 stream tags on track 0 and 3 stream tags on track 1 with the SAME tag keys.
    // Unscoped deduplication in options_matroska_metadata_working_bytes would collapse them
    // into 3 tags (~17 KiB) instead of 6 tags across two tracks (~35 KiB).
    // With max_controlled_bytes = 25_000, uncorrected qualify under-counted index_and_meta_bytes
    // (~17.6 KiB <= 25_000) and entered the video bridge, whereas corrected qualify charges all
    // 6 scoped tags across both tracks (~35.3 KiB > 25_000) and rejects immediately with
    // "controlled memory budget exceeded: MP4 index and metadata".
    let mut stream_metadata_set = Vec::new();
    for i in 0..3 {
        stream_metadata_set.push((0usize, format!("CUSTOM_{i}"), "a".repeat(200)));
        stream_metadata_set.push((1usize, format!("CUSTOM_{i}"), "b".repeat(200)));
    }
    let opts = CopyOptions {
        max_controlled_bytes: Some(25_000),
        stream_metadata_set,
        ..Default::default()
    };
    let plan_err = plan_transcode_lossless(&mp4, &transform, &opts, None).unwrap_err();
    assert!(
        plan_err
            .to_string()
            .contains("controlled memory budget exceeded: MP4 index and metadata"),
        "expected MP4 index and metadata budget error in plan, got: {plan_err}"
    );
    let out_mkv = temp_path("mp4-multitrack-meta", "mkv");
    let _ = std::fs::remove_file(&out_mkv);
    let export_err = transcode_lossless(&mp4, &out_mkv, transform, &opts).unwrap_err();
    assert!(
        export_err
            .to_string()
            .contains("controlled memory budget exceeded: MP4 index and metadata"),
        "expected MP4 index and metadata budget error in export, got: {export_err}"
    );
}

#[test]
fn native_y4m_export_checks_existing_output_and_defers_zero_progress_until_preflight() {
    let y4m = fixture_path("batch589_valid_25fps.y4m");
    let existing = temp_path("y4m-existing", "y4m");
    std::fs::write(&existing, b"existing-y4m").unwrap();

    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let hook_events = Arc::clone(&events);
    let hook = ProgressHook::new(move |ev| {
        hook_events.lock().unwrap().push(ev);
    });

    // 1. Existing output must be rejected upfront with "output already exists" and 0 progress events.
    let opts_existing = CopyOptions {
        progress: Some(hook.clone()),
        ..Default::default()
    };
    let err_existing = export_y4m_pipeline_with_options(
        &y4m,
        &existing,
        None,
        &Default::default(),
        &Default::default(),
        &opts_existing,
    )
    .unwrap_err();
    let _ = std::fs::remove_file(&existing);
    assert!(
        err_existing.to_string().contains("output already exists"),
        "expected output already exists, got: {err_existing}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "existing output refusal must not emit a 0-packet progress event"
    );

    // 2. Budget refusal during preflight must not emit a 0-packet progress event.
    let out_y4m = temp_path("y4m-budget-fail", "y4m");
    let _ = std::fs::remove_file(&out_y4m);
    let opts_tight = CopyOptions {
        max_packet_bytes: 2,
        progress: Some(hook.clone()),
        ..Default::default()
    };
    let err_tight = export_y4m_pipeline_with_options(
        &y4m,
        &out_y4m,
        None,
        &Default::default(),
        &Default::default(),
        &opts_tight,
    )
    .unwrap_err();
    assert!(
        err_tight.to_string().contains("Y4M frame exceeds byte limit"),
        "expected Y4M frame exceeds byte limit, got: {err_tight}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "failed Y4M export preflight must not emit a 0-packet progress event"
    );

    // 3. Incompatible concat inputs must also fail before emitting a 0-packet progress event.
    let other_y4m = fixture_path("batch589_valid_30fps.y4m");
    let err_concat = concat_y4m_with_options(
        &[y4m.clone(), other_y4m.clone()],
        &out_y4m,
        &CopyOptions {
            progress: Some(hook.clone()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        err_concat.to_string().contains("Y4M"),
        "unexpected concat error: {err_concat}"
    );
    assert!(
        events.lock().unwrap().is_empty(),
        "failed Y4M concat preflight must not emit a 0-packet progress event"
    );
    let _ = concat(&[y4m, other_y4m], &out_y4m, &CopyOptions { progress: Some(hook), ..Default::default() });
    assert!(events.lock().unwrap().is_empty());
}

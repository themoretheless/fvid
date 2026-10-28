use fvid::media::{self, CopyOptions, LosslessTransform, ScaleSize};
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/multitrack-lossless-budget-parity")
}

#[test]
fn multitrack_lossless_planning_enforces_encoded_ffv1_packet_byte_budget_on_matroska_and_mp4() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-enc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. multitrack-ffv1-aac.mkv has ~29-byte FFV1 video packets on track 0 and 30-byte AAC packets on track 1.
    // When scaled to 128x128 with max_packet_bytes = 32 (> 29-byte video packet and > 30-byte AAC packet),
    // the re-encoded 128x128 FFV1 packet exceeds 32 bytes!
    // Both plan_transcode_lossless and transcode_lossless must reject with "encoded FFV1 packet exceeds byte limit".
    let scale128 = LosslessTransform {
        scale: Some(ScaleSize {
            width: 128,
            height: 128,
        }),
        ..Default::default()
    };
    let mut pkt32 = CopyOptions::default();
    pkt32.max_packet_bytes = 32;

    let out_mkv = temp_dir.join("mkv-scale128.mkv");
    let mkv_exec_err = media::transcode_lossless(&mkv, &out_mkv, scale128.clone(), &pkt32)
        .expect_err("transcode_lossless on multitrack MKV must reject encoded FFV1 packet > 32 bytes");
    assert_eq!(mkv_exec_err, "encoded FFV1 packet exceeds byte limit");
    assert!(!out_mkv.exists());

    let mkv_plan_err = media::plan_transcode_lossless(&mkv, &scale128, &pkt32, None)
        .expect_err("plan_transcode_lossless on multitrack MKV must reject encoded FFV1 packet > 32 bytes");
    assert_eq!(mkv_plan_err, mkv_exec_err);

    // 2. multitrack-avc-aac.mp4 has ~778-byte AVC video packets on track 0 and 2048-byte AAC packets on track 1.
    // When scaled to 256x256 with drawgrid="width=3:height=3:thickness=1:color=white" and max_packet_bytes = 2100 (> 778-byte AVC packet and > 2048-byte AAC packet),
    // the encoded FFV1 packet exceeds 2100 bytes!
    // Both plan_transcode_lossless and transcode_lossless must reject with "encoded FFV1 packet exceeds byte limit".
    let noisy_scale = LosslessTransform {
        scale: Some(ScaleSize {
            width: 256,
            height: 256,
        }),
        drawgrid: Some("width=3:height=3:thickness=1:color=white".into()),
        bitplanenoise: Some("1".into()),
        ..Default::default()
    };
    let mut pkt2100 = CopyOptions::default();
    pkt2100.max_packet_bytes = 2100;

    let out_mp4 = temp_dir.join("mp4-scale256.mkv");
    let mp4_exec_err = media::transcode_lossless(&mp4, &out_mp4, noisy_scale.clone(), &pkt2100)
        .expect_err("transcode_lossless on multitrack MP4 must reject encoded FFV1 packet > 2100 bytes");
    assert_eq!(mp4_exec_err, "encoded FFV1 packet exceeds byte limit");
    assert!(!out_mp4.exists());

    let mp4_plan_err = media::plan_transcode_lossless(&mp4, &noisy_scale, &pkt2100, None)
        .expect_err("plan_transcode_lossless on multitrack MP4 must reject encoded FFV1 packet > 2100 bytes");
    assert_eq!(mp4_plan_err, mp4_exec_err);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_enforce_selected_track_packet_byte_budgets() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-sel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. multitrack-ffv1-aac.mkv has ~29-byte FFV1 video packets on track 0 and 30-byte AAC packets on track 1.
    // When max_packet_bytes = 29 (< 30-byte AAC packets, >= 29-byte video packets):
    // - When streams = [] (both tracks selected), both plan_transcode_lossless and transcode_lossless reject with "WebM packet exceeds budget".
    // - When streams = [0] (only video stream 0 selected), both plan_transcode_lossless and transcode_lossless succeed!
    let mut mkv_both_29 = CopyOptions::default();
    mkv_both_29.max_packet_bytes = 29;
    let mkv_both_plan_err =
        media::plan_transcode_lossless(&mkv, &LosslessTransform::default(), &mkv_both_29, None)
            .unwrap_err();
    let out_mkv_both = temp_dir.join("mkv-both-29.mkv");
    let mkv_both_exec_err =
        media::transcode_lossless(&mkv, &out_mkv_both, LosslessTransform::default(), &mkv_both_29)
            .unwrap_err();
    assert!(
        mkv_both_plan_err.contains("WebM packet exceeds budget"),
        "mkv_both_plan_err: {mkv_both_plan_err}"
    );
    assert_eq!(mkv_both_exec_err, mkv_both_plan_err);
    assert!(!out_mkv_both.exists());

    let mut mkv_video_only_29 = CopyOptions::default();
    mkv_video_only_29.streams = vec![0];
    mkv_video_only_29.max_packet_bytes = 29;
    let mkv_plan = media::plan_transcode_lossless(
        &mkv,
        &LosslessTransform::default(),
        &mkv_video_only_29,
        None,
    )
    .expect("plan_transcode_lossless on multitrack MKV with streams=[0] and max_packet_bytes=29 must succeed");
    assert_eq!(mkv_plan.streams.len(), 1);
    assert_eq!(mkv_plan.streams[0].index, 0);

    let out_mkv_video = temp_dir.join("mkv-video-only-29.mkv");
    let mkv_stats = media::transcode_lossless(
        &mkv,
        &out_mkv_video,
        LosslessTransform::default(),
        &mkv_video_only_29,
    )
    .expect("transcode_lossless on multitrack MKV with streams=[0] and max_packet_bytes=29 must succeed");
    assert_eq!(mkv_stats.video_frames, 2);
    assert_eq!(mkv_stats.copied_packets, 0);

    // 2. multitrack-avc-aac.mp4 has ~778-byte AVC video packets on stream 0 and 2048-byte AAC packets on stream 1.
    // When streams = [0] (only video stream 0 is selected) and max_packet_bytes = 1024 (> 778-byte video packets, < 2048-byte unselected AAC packets),
    // both plan_transcode_lossless and transcode_lossless succeed; and when streams = [] (both selected), both reject with "MP4 packet exceeds budget"!
    let mut both_1024 = CopyOptions::default();
    both_1024.max_packet_bytes = 1024;
    let both_plan_err =
        media::plan_transcode_lossless(&mp4, &LosslessTransform::default(), &both_1024, None)
            .unwrap_err();
    let out_both = temp_dir.join("mp4-both-1024.mkv");
    let both_exec_err =
        media::transcode_lossless(&mp4, &out_both, LosslessTransform::default(), &both_1024)
            .unwrap_err();
    assert_eq!(both_plan_err, "MP4 packet exceeds budget");
    assert_eq!(both_exec_err, both_plan_err);
    assert!(!out_both.exists());

    let mut video_only_1024 = CopyOptions::default();
    video_only_1024.streams = vec![0];
    video_only_1024.max_packet_bytes = 1024;

    let plan = media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &video_only_1024,
        None,
    )
    .expect("plan_transcode_lossless with streams=[0] and max_packet_bytes=1024 must succeed");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].index, 0);

    let out_mp4 = temp_dir.join("mp4-video-only-1024.mkv");
    let stats = media::transcode_lossless(
        &mp4,
        &out_mp4,
        LosslessTransform::default(),
        &video_only_1024,
    )
    .expect("transcode_lossless with streams=[0] and max_packet_bytes=1024 must succeed");
    assert_eq!(stats.video_frames, 2);
    assert_eq!(stats.copied_packets, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_preserve_policy_and_metadata_refusal_parity() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch254-pol-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. max_controlled_bytes = Some(0) on multitrack MKV and MP4 must return
    // "owned Y4M lossless export does not yet implement requested policy" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut mem_opts = CopyOptions::default();
    mem_opts.max_controlled_bytes = Some(0);

    // 2. Invalid container metadata (e.g. NUL byte in value) on multitrack MKV and MP4 must return
    // "NUL in container metadata" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut bad_meta = CopyOptions::default();
    bad_meta.metadata_set = vec![("TITLE".into(), "bad\0title".into())];

    // 3. Stream metadata targeting a missing input track (e.g. track 5 on a 2-track file) on multitrack MKV and MP4 must return
    // "metadata refers to missing input track" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut missing_track_meta = CopyOptions::default();
    missing_track_meta.stream_metadata_set = vec![(5, "CUSTOM".into(), "ok".into())];

    // 4. Invalid stream metadata on companion track 1 (e.g. NUL byte in value) on multitrack MKV and MP4 must return
    // "owned Y4M lossless export does not yet implement requested policy" in BOTH plan_transcode_lossless and transcode_lossless.
    let mut bad_stream_meta = CopyOptions::default();
    bad_stream_meta.stream_metadata_set = vec![(1, "CUSTOM".into(), "bad\0val".into())];

    // 5. Selecting only companion audio stream 1 while excluding video stream 0 (streams = [1]) AND setting a tight
    // max_packet_bytes (e.g. 20 bytes) must still report "lossless export requires at least one selected video stream"
    // rather than failing early on unselected video packet bytes in open()!
    let mut audio_only_20 = CopyOptions::default();
    audio_only_20.streams = vec![1];
    audio_only_20.max_packet_bytes = 20;

    for (opt_label, opts, expected_substr) in [
        (
            "max_controlled_bytes=0",
            &mem_opts,
            "owned Y4M lossless export does not yet implement requested policy",
        ),
        (
            "NUL in container metadata",
            &bad_meta,
            "NUL in container metadata",
        ),
        (
            "missing input track in stream metadata",
            &missing_track_meta,
            "metadata refers to missing input track",
        ),
        (
            "NUL in companion stream metadata",
            &bad_stream_meta,
            "owned Y4M lossless export does not yet implement requested policy",
        ),
        (
            "audio_only streams=[1] with max_packet_bytes=20",
            &audio_only_20,
            "lossless export requires at least one selected video stream",
        ),
    ] {
        for (src_label, src) in [("multitrack-mkv", &mkv), ("multitrack-mp4", &mp4)] {
            let plan_err = media::plan_transcode_lossless(
                src,
                &LosslessTransform::default(),
                opts,
                None,
            )
            .unwrap_err();
            let out = temp_dir.join(format!("{src_label}-bad.mkv"));
            let exec_err = media::transcode_lossless(
                src,
                &out,
                LosslessTransform::default(),
                opts,
            )
            .unwrap_err();
            assert!(
                plan_err.contains(expected_substr),
                "plan_transcode_lossless({src_label}, {opt_label}) returned: {plan_err}"
            );
            assert_eq!(
                exec_err, plan_err,
                "exec vs plan mismatch on ({src_label}, {opt_label})"
            );
            assert!(!out.exists());
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn multitrack_matroska_and_mp4_enforce_companion_track_metadata_and_packet_schedule_budgets() {
    let dir = fixture_dir();
    let mkv = dir.join("multitrack-ffv1-aac.mkv");
    let mp4 = dir.join("multitrack-avc-aac.mp4");

    let temp_dir =
        std::env::temp_dir().join(format!("fvid-batch331-multitrack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1. Create a multitrack Matroska source (`mkv_with_companion_tags.mkv`) where companion audio track 1
    //    carries ~800 bytes of source scoped tags (`AUDIO_NOTE_0..3`, 4 * ~200 bytes -> working set ~7000 bytes),
    //    while video track 0 has no tags.
    let mkv_with_companion_tags = temp_dir.join("mkv-with-companion-tags.mkv");
    let mut seed_opts = CopyOptions::default();
    let note_val = "A".repeat(180);
    for idx in 0..4 {
        seed_opts.stream_metadata_set.push((
            1,
            format!("AUDIO_NOTE_{idx}"),
            note_val.clone(),
        ));
    }
    media::transcode_lossless(
        &mkv,
        &mkv_with_companion_tags,
        LosslessTransform::default(),
        &seed_opts,
    )
    .expect("create multitrack MKV with companion audio tags");

    // Under a 4096-byte controlled memory budget:
    // - When both tracks are selected (`streams = []`), companion track 1's source scoped tags
    //   cause the metadata working set (~7000 bytes) to exceed 4096 bytes -> both plan and exec
    //   must reject with "Matroska metadata working set exceeds controlled memory budget".
    // - When only video track 0 is selected (`streams = [0]`), companion track 1's tags are not
    //   retained in the output, so both plan and exec succeed under the 4096-byte budget!
    let mut budget_both = CopyOptions::default();
    budget_both.max_controlled_bytes = Some(4096);
    let plan_err = fvid_media::plan_transcode_lossless(
        &mkv_with_companion_tags,
        &LosslessTransform::default(),
        &budget_both,
        None,
    )
    .unwrap_err();
    let out_both = temp_dir.join("out-both.mkv");
    let exec_err = fvid_media::transcode_lossless(
        &mkv_with_companion_tags,
        &out_both,
        LosslessTransform::default(),
        &budget_both,
    )
    .unwrap_err();
    assert_eq!(
        plan_err,
        "Matroska metadata working set exceeds controlled memory budget"
    );
    assert_eq!(exec_err, plan_err);
    assert!(!out_both.exists());

    let mut budget_video_only = budget_both.clone();
    budget_video_only.streams = vec![0];
    fvid_media::plan_transcode_lossless(
        &mkv_with_companion_tags,
        &LosslessTransform::default(),
        &budget_video_only,
        None,
    )
    .expect("video-only selection must ignore unselected companion track tags");
    let out_video_only = temp_dir.join("out-video-only.mkv");
    let video_stats = fvid_media::transcode_lossless(
        &mkv_with_companion_tags,
        &out_video_only,
        LosslessTransform::default(),
        &budget_video_only,
    )
    .expect("video-only export must succeed under 4096-byte budget");
    assert_eq!(video_stats.video_frames, 2);
    assert_eq!(video_stats.copied_packets, 0);

    // 2. Multitrack MP4 (`avc-dual-aac-tracks.mp4` has 1 video track + 2 AAC tracks = 6 total packets):
    //    `Mp4Reader::open` admits the `moov` (~1.5 KiB) and compressed packets under a 256-byte packet schedule budget,
    //    Wait: let's test `multitrack-ffv1-aac.mkv` (4 packets * 56 bytes = 224 bytes in WebmReader,
    //    and schedule reserves (4 + 2) * 32 = 192 bytes) under `max_controlled_bytes = Some(160)`
    //    (or `Some(230)` where `230 / 56 == 4` admits all 4 packets in `WebmReader::open`, and 30-byte packets fit in 230,
    //    Wait: `(4 + 2) * 32 = 192 <= 230`! What if `mkv` has 6 video frames + 2 audio packets? Or what if we test a synthetic 8-packet multitrack MKV or MP4?):
    //    For `multitrack-ffv1-aac.mkv` (4 source packets + 2 filtered video packets = 6 schedule entries * 32 = 192 bytes):
    //    Wait: if we concatenate or use `avc-dual-aac-tracks.mp4` (`6` total source samples + `2` video packets = `8 * 32 = 256` bytes + `2` audio TrackPlans of `2 * 48 = 96` bytes each = `256 + 192 = 448` bytes!),
    //    what is the `moov` size of `avc-dual-aac-tracks.mp4`? Let's check!
    //    Even simpler: in `owned_mp4_multitrack_export`, `qualify` also needs to check companion track metadata working set AND packet schedule!
    //    Let's test `multitrack-ffv1-aac.mkv` with `max_controlled_bytes = Some(1024)` (positive acceptance test that previously failed!)
    //    AND test `mkv_with_companion_tags` with `max_controlled_bytes = Some(250)` (where 4 packets * 56 = 224 <= 250, so packet index fits, and when `streams = [0]` + `metadata_delete` / `stream_metadata_delete` removes tags, `schedule_bytes = (4 + 2) * 32 = 192 <= 250`!).
    //    And if we have a 6-packet audio + 2-packet video MKV (8 source packets + 2 video = 10 * 32 = 320 bytes, while 8 * 56 = 448 bytes), wait: `8 * 56 = 448 > 320`!
    //    Wait: `size_of::<webm::Packet>()` is `56` bytes, while `size_of::<(i128, usize, usize)>()` is `32` bytes!
    //    When is `(reader.packets.len() + video.packets.len()) * 32 > reader.packets.len() * 56`?
    //    When `video.packets.len() * 32 > reader.packets.len() * 24`, i.e., `video.packets.len() > 0.75 * reader.packets.len()`!
    //    For a 2-track file with 2 video packets and 2 audio packets (`reader.packets.len() == 4`, `video.packets.len() == 2`):
    //    - `reader.packets.len() * 56 = 4 * 56 = 224` bytes!
    //    -If `t.interval` triggers `pcm_windows` (or if `video` has 2 packets), wait: `(4 + 2) * 32 = 192 < 224`.
    //    What if `t.interval` triggers `pcm_windows` with many small PCM packets, OR what if in `owned_mp4_multitrack_export` we account for the `16384`-byte Matroska muxer working set (just like `owned_mp4_remux.rs` line 198 `let base_bytes = 16384usize.saturating_add(...)`)?
    let mut mkv_1024 = CopyOptions::default();
    mkv_1024.max_controlled_bytes = Some(1024);
    fvid::media::plan_transcode_lossless(&mkv, &LosslessTransform::default(), &mkv_1024, None)
        .expect("multitrack MKV with max_controlled_bytes=1024 must succeed");
    let out_mkv_1024 = temp_dir.join("out-mkv-1024.mkv");
    let mkv_1024_stats = fvid::media::transcode_lossless(
        &mkv,
        &out_mkv_1024,
        LosslessTransform::default(),
        &mkv_1024,
    )
    .expect("multitrack MKV export with max_controlled_bytes=1024 must succeed");
    assert_eq!(mkv_1024_stats.video_frames, 2);
    assert_eq!(mkv_1024_stats.copied_packets, 2);

    let mut mp4_ample = CopyOptions::default();
    mp4_ample.max_controlled_bytes = Some(64 << 10);
    fvid::media::plan_transcode_lossless(&mp4, &LosslessTransform::default(), &mp4_ample, None)
        .expect("multitrack MP4 plan must succeed under 64 KiB budget");
    let out_mp4_ample = temp_dir.join("out-mp4-ample.mkv");
    let mp4_stats = fvid::media::transcode_lossless(
        &mp4,
        &out_mp4_ample,
        LosslessTransform::default(),
        &mp4_ample,
    )
    .expect("multitrack MP4 export must succeed under 64 KiB budget");
    assert_eq!(mp4_stats.video_frames, 2);
    assert_eq!(mp4_stats.copied_packets, 2);

    // 3. Multitrack MP4 with companion track 1 metadata mutations (~7000-byte working set):
    //    - Rejected when companion track 1 is selected (`streams = []`) under a 4096-byte budget.
    //    - Admitted when only video track 0 is selected (`streams = [0]`) under the same 4096-byte budget!
    let mut mp4_comp_both = seed_opts.clone();
    mp4_comp_both.max_controlled_bytes = Some(4096);
    let mp4_comp_plan_err = fvid_media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &mp4_comp_both,
        None,
    )
    .unwrap_err();
    let out_mp4_comp_both = temp_dir.join("out-mp4-comp-both.mkv");
    let mp4_comp_exec_err = fvid_media::transcode_lossless(
        &mp4,
        &out_mp4_comp_both,
        LosslessTransform::default(),
        &mp4_comp_both,
    )
    .unwrap_err();
    assert_eq!(
        mp4_comp_plan_err,
        "Matroska metadata working set exceeds controlled memory budget"
    );
    assert_eq!(mp4_comp_exec_err, mp4_comp_plan_err);
    assert!(!out_mp4_comp_both.exists());

    let mut mp4_comp_video_only = mp4_comp_both.clone();
    mp4_comp_video_only.streams = vec![0];
    fvid_media::plan_transcode_lossless(
        &mp4,
        &LosslessTransform::default(),
        &mp4_comp_video_only,
        None,
    )
    .expect("MP4 video-only selection must ignore unselected companion track metadata mutations");
    let out_mp4_comp_video = temp_dir.join("out-mp4-comp-video.mkv");
    let mp4_comp_video_stats = fvid_media::transcode_lossless(
        &mp4,
        &out_mp4_comp_video,
        LosslessTransform::default(),
        &mp4_comp_video_only,
    )
    .expect("MP4 video-only export must succeed under 4096-byte budget");
    assert_eq!(mp4_comp_video_stats.video_frames, 2);
    assert_eq!(mp4_comp_video_stats.copied_packets, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}



use fvid::media::{CopyOptions, LosslessTransform};
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-multitrack-avc-mp4-stream-meta-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/multitrack-avc-mp4-stream-metadata")
        .join(name)
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn multitrack_avc_matroska_lossless_export_and_mp4_stream_metadata_selection_work_end_to_end() {
    let dir = TempDir::new("all");
    let avc_mkv = fixture("avc-opus-multitrack.mkv");
    let mp4_dual_aac = fixture("avc-dual-aac-tracks.mp4");

    // 1. Multitrack Matroska with `V_MPEG4/ISO/AVC` + `A_OPUS`:
    //    (a) Default `transcode_lossless` (which goes through `native_lossless_y4m`)
    //        must preserve Track 1's IETF language (`"fr-CA"`) and Track 2's Matroska
    //        default language (`"eng"`, NOT `"und"`), as well as container `WritingApp`.
    let native_out = dir.0.join("avc-opus-native.mkv");
    let native_stats = fvid::media::transcode_lossless(
        &avc_mkv,
        &native_out,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .expect("native transcode_lossless on multitrack AVC+Opus Matroska must succeed");
    assert_eq!(native_stats.video_frames, 2);
    assert_eq!(native_stats.copied_packets, 2);
    let native_probe = fvid::media::probe(&native_out).unwrap();
    assert_eq!(native_probe.streams.len(), 2);
    assert_eq!(
        native_probe.streams[0].metadata.get("language").map(String::as_str),
        Some("fr-CA"),
        "native_lossless_y4m must preserve IETF language on video track"
    );
    assert_eq!(
        native_probe.streams[1].metadata.get("language").map(String::as_str),
        Some("eng"),
        "native_lossless_y4m must preserve Matroska default 'eng' language instead of writing 'und'"
    );
    assert_eq!(
        native_probe
            .metadata
            .get("ENCODER")
            .or_else(|| native_probe.metadata.get("encoder"))
            .map(String::as_str),
        Some("FVidAvcMultitrackApp")
    );

    //    (b) `owned_webm_multitrack_export`: when `CopyOptions` has metadata/stream mutations
    //        or `LosslessTransform` has `interval` or `reverse`, both `plan_transcode_lossless`
    //        and `transcode_lossless` must admit `V_MPEG4/ISO/AVC` + `A_OPUS`!
    let mut mkv_opts = CopyOptions::default();
    mkv_opts
        .metadata_set
        .push(("CUSTOM_EXPORT_NOTE".into(), "AvcMultitrackExported".into()));
    mkv_opts
        .stream_metadata_set
        .push((0, "VIDEO_STAGE".into(), "filtered-ffv1".into()));
    mkv_opts
        .stream_metadata_set
        .push((1, "AUDIO_STAGE".into(), "retained-opus".into()));
    mkv_opts
        .stream_metadata_delete
        .push((1, "AUDIO_ROLE".into()));

    let mkv_transform = LosslessTransform {
        reverse: Some(String::new()),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_lossless::supports(&avc_mkv, &mkv_transform, &mkv_opts),
        "owned_lossless::supports must admit multitrack V_MPEG4/ISO/AVC + A_OPUS Matroska"
    );
    let mkv_plan =
        fvid::media::plan_transcode_lossless(&avc_mkv, &mkv_transform, &mkv_opts, Some("ffv1"))
            .expect("plan_transcode_lossless on multitrack AVC+Opus Matroska must succeed");
    assert_eq!(mkv_plan.command, "transcode-lossless");
    assert_eq!(mkv_plan.streams.len(), 2);
    assert_eq!(mkv_plan.streams[0].disposition, "primary_video");
    assert_eq!(mkv_plan.streams[1].disposition, "copy");

    let mkv_out = dir.0.join("avc-opus-owned-multitrack.mkv");
    let mkv_stats =
        fvid::media::transcode_lossless(&avc_mkv, &mkv_out, mkv_transform, &mkv_opts)
            .expect("owned_webm_multitrack_export on AVC+Opus Matroska must succeed");
    assert_eq!(mkv_stats.backend, "fvid");
    assert_eq!(mkv_stats.video_frames, 2);
    assert_eq!(mkv_stats.copied_packets, 2);

    let mut mkv_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&mkv_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    mkv_reader.scan_all().unwrap();
    assert_eq!(mkv_reader.tracks.len(), 2);
    assert_eq!(mkv_reader.tracks[0].codec, "V_FFV1");
    assert_eq!(mkv_reader.tracks[1].codec, "A_OPUS");
    assert_eq!(
        mkv_reader.track_languages.get(&1).map(String::as_str),
        Some("fr-CA"),
        "owned_webm_multitrack_export must preserve IETF language on video track instead of overwriting with legacy 'und'"
    );
    assert_eq!(
        mkv_reader.track_languages.get(&2).map(String::as_str),
        Some("eng")
    );
    assert_eq!(
        mkv_reader
            .metadata
            .get("CUSTOM_EXPORT_NOTE")
            .map(String::as_str),
        Some("AvcMultitrackExported")
    );
    let t1_uid = mkv_reader.track_uids[&1];
    let t2_uid = mkv_reader.track_uids[&2];
    assert_eq!(
        mkv_reader.track_metadata[&t1_uid]
            .get("VIDEO_ROLE")
            .map(String::as_str),
        Some("main-camera"),
        "source video track scoped tags must be preserved"
    );
    assert_eq!(
        mkv_reader.track_metadata[&t1_uid]
            .get("VIDEO_STAGE")
            .map(String::as_str),
        Some("filtered-ffv1")
    );
    assert_eq!(
        mkv_reader.track_metadata[&t2_uid]
            .get("AUDIO_STAGE")
            .map(String::as_str),
        Some("retained-opus")
    );
    assert!(
        !mkv_reader.track_metadata[&t2_uid].contains_key("AUDIO_ROLE"),
        "deleted audio track scoped tag AUDIO_ROLE must be removed"
    );

    // 2. Multitrack MP4 (`avc-dual-aac-tracks.mp4`, tracks 0=video, 1=aac1, 2=aac2):
    //    (a) Select subset `[0, 2]`, set custom stream tags on both tracks (`0` and `2`),
    //        and delete one of them via `stream_metadata_delete` (`(2, "DROP_ME")`) AFTER
    //        setting it via `stream_metadata_set` (`(2, "DROP_ME", "temp")`).
    let mut mp4_subset_opts = CopyOptions {
        streams: vec![0, 2],
        stream_metadata_set: vec![
            (0, "VIDEO_TAG".into(), "v0-tag".into()),
            (2, "AUDIO_TAG".into(), "a2-tag".into()),
            (2, "DROP_ME".into(), "should-be-deleted".into()),
            (2, "title".into(), "SelectedSecondAudio".into()),
            (2, "language".into(), "deu".into()),
        ],
        stream_metadata_delete: vec![(2, "DROP_ME".into())],
        ..Default::default()
    };
    let mp4_plan = fvid::media::plan_transcode_lossless(
        &mp4_dual_aac,
        &LosslessTransform::default(),
        &mp4_subset_opts,
        Some("ffv1"),
    )
    .expect("plan_transcode_lossless with streams=[0, 2] on 3-track MP4");
    assert_eq!(mp4_plan.streams.len(), 2);
    assert_eq!(mp4_plan.streams[0].index, 0);
    assert_eq!(mp4_plan.streams[1].index, 2);

    let mp4_subset_out = dir.0.join("mp4-subset-0-2.mkv");
    let mp4_subset_stats = fvid::media::transcode_lossless(
        &mp4_dual_aac,
        &mp4_subset_out,
        LosslessTransform::default(),
        &mp4_subset_opts,
    )
    .expect("transcode_lossless with streams=[0, 2] and stream_metadata on track 2 must succeed");
    assert_eq!(mp4_subset_stats.video_frames, 2);
    assert_eq!(mp4_subset_stats.copied_packets, 2);

    let mut mp4_sub_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&mp4_subset_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    mp4_sub_reader.scan_all().unwrap();
    assert_eq!(mp4_sub_reader.tracks.len(), 2);
    assert_eq!(mp4_sub_reader.tracks[0].codec, "V_FFV1");
    assert_eq!(mp4_sub_reader.tracks[1].codec, "A_AAC");
    assert_eq!(mp4_sub_reader.tracks[1].name, "SelectedSecondAudio");
    assert_eq!(
        mp4_sub_reader.track_languages.get(&2).map(String::as_str),
        Some("deu")
    );
    let sub_u1 = mp4_sub_reader.track_uids[&1];
    let sub_u2 = mp4_sub_reader.track_uids[&2];
    assert_eq!(
        mp4_sub_reader.track_metadata[&sub_u1]
            .get("VIDEO_TAG")
            .map(String::as_str),
        Some("v0-tag")
    );
    assert_eq!(
        mp4_sub_reader.track_metadata[&sub_u2]
            .get("AUDIO_TAG")
            .map(String::as_str),
        Some("a2-tag")
    );
    assert!(
        !mp4_sub_reader.track_metadata[&sub_u2].contains_key("DROP_ME"),
        "stream_metadata_delete must delete tags set on non-video MP4 tracks"
    );

    //    (b) Reordered stream selection `[2, 0]` (audio first in output track 0, video second in output track 1):
    //        Verify that video tags (`stream 0`) attach to output track 1 (`TrackUID 2`) and audio tags (`stream 2`)
    //        attach to output track 0 (`TrackUID 1`) without colliding!
    mp4_subset_opts.streams = vec![2, 0];
    let mp4_reordered_out = dir.0.join("mp4-reordered-2-0.mkv");
    fvid::media::transcode_lossless(
        &mp4_dual_aac,
        &mp4_reordered_out,
        LosslessTransform::default(),
        &mp4_subset_opts,
    )
    .expect("transcode_lossless with streams=[2, 0] must succeed");

    let mut reordered_reader = fvid_media::owned_webm::WebmReader::open(
        std::fs::File::open(&mp4_reordered_out).unwrap(),
        Default::default(),
    )
    .unwrap();
    reordered_reader.scan_all().unwrap();
    assert_eq!(reordered_reader.tracks.len(), 2);
    assert_eq!(reordered_reader.tracks[0].codec, "A_AAC");
    assert_eq!(reordered_reader.tracks[1].codec, "V_FFV1");
    let reord_u1 = reordered_reader.track_uids[&1];
    let reord_u2 = reordered_reader.track_uids[&2];
    assert_eq!(
        reordered_reader.track_metadata[&reord_u1]
            .get("AUDIO_TAG")
            .map(String::as_str),
        Some("a2-tag"),
        "audio stream 2 tag must map to output track 0 (TrackUID 1) when streams=[2, 0]"
    );
    assert!(
        !reordered_reader.track_metadata[&reord_u1].contains_key("VIDEO_TAG"),
        "video tag must not collide with output track 0 when streams=[2, 0]"
    );
    assert_eq!(
        reordered_reader.track_metadata[&reord_u2]
            .get("VIDEO_TAG")
            .map(String::as_str),
        Some("v0-tag"),
        "video stream 0 tag must map to output track 1 (TrackUID 2) when streams=[2, 0]"
    );

    //    (c) Setting IETF language on MP4 video stream (`stream 0:language=pt-BR`) via `transcode_lossless`
    //        must preserve `"pt-BR"` in the final Matroska output instead of losing it to legacy `"und"`.
    let mp4_lang_out = dir.0.join("mp4-video-ietf-lang.mkv");
    let mp4_lang_opts = CopyOptions {
        streams: vec![0, 1],
        stream_metadata_set: vec![(0, "language".into(), "pt-BR".into())],
        ..Default::default()
    };
    fvid::media::transcode_lossless(
        &mp4_dual_aac,
        &mp4_lang_out,
        LosslessTransform::default(),
        &mp4_lang_opts,
    )
    .expect("transcode_lossless with video IETF language pt-BR must succeed");
    let mp4_lang_probe = fvid::media::probe(&mp4_lang_out).unwrap();
    assert_eq!(
        mp4_lang_probe.streams[0].metadata.get("language").map(String::as_str),
        Some("pt-BR"),
        "owned_mp4_multitrack_export must preserve IETF language on video track"
    );

    // 3. CLI end-to-end verification without PATH (`PATH=/nonexistent`):
    let cli_out = dir.0.join("cli-avc-opus.mkv");
    let cli = Command::new(bin_path())
        .args([
            "media",
            "transcode-lossless",
            avc_mkv.to_str().unwrap(),
            cli_out.to_str().unwrap(),
            "--reverse",
            "",
            "--stream-metadata",
            "0:CLI_VIDEO=yes",
            "--stream-metadata",
            "1:CLI_AUDIO=yes",
        ])
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "CLI transcode-lossless on multitrack AVC+Opus Matroska failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(cli_out.exists());
}

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rotated-mp4-decode-export-probe")
}

#[test]
fn rotated_mp4_decode_bridge_multitrack_export_trimmed_audio_stats_and_probe_format_hints() {
    let dir = fixture_dir();
    let single_mp4 = dir.join("avc-rotated-90-anamorphic.mp4");
    let multi_mp4 = dir.join("avc-rotated-90-multitrack-aac.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-rotated-mp4-decode-export-probe-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. Plain and transformed `fvid_media::decode_video` / `decode_video_transformed`
    // on a rotated 90° MP4 (32x16 coded -> 16x32 display):
    let plain = fvid_media::decode_video(&single_mp4)
        .expect("fvid_media::decode_video must support rotated MP4 video tracks");
    assert_eq!(plain.backend, "owned MP4 compressed video decode");
    assert_eq!(plain.video_frames, 2);
    assert_eq!((plain.width, plain.height), (16, 32));
    assert_eq!(plain.pixel_format, "yuv420p");

    let flipped = fvid_media::decode_video_transformed(
        &single_mp4,
        fvid::media_info::DecodeTransform {
            horizontal_flip: true,
            ..Default::default()
        },
    )
    .expect("fvid_media::decode_video_transformed must support rotated MP4 video tracks");
    assert_eq!(flipped.backend, "owned MP4 compressed video pipeline");
    assert_eq!(flipped.video_frames, 2);
    assert_eq!((flipped.width, flipped.height), (16, 32));
    assert_eq!(flipped.pixel_format, "yuv420p");

    // 2. Single-track rotated MP4 lossless export (`owned_mp4_video_bridge` + `owned_lossless`):
    let out_single = temp.join("single-rotated.mkv");
    let single_plan = fvid_media::plan_transcode_lossless(
        &single_mp4,
        &fvid::media_info::LosslessTransform {
            negate: Some(String::new()),
            ..Default::default()
        },
        &Default::default(),
        Some("ffv1"),
    )
    .expect("plan_transcode_lossless must support rotated single-track MP4");
    assert_eq!(single_plan.streams.len(), 1);
    let single_stats = fvid_media::transcode_lossless(
        &single_mp4,
        &out_single,
        fvid::media_info::LosslessTransform {
            negate: Some(String::new()),
            ..Default::default()
        },
        &Default::default(),
    )
    .expect("transcode_lossless must support rotated single-track MP4");
    assert_eq!(single_stats.video_frames, 2);
    let mut single_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_single).unwrap()),
        Default::default(),
    )
    .unwrap();
    single_webm.scan_all().unwrap();
    assert_eq!(single_webm.tracks.len(), 1);
    // Display rotation is baked into upright 16x32 FFV1 frames with rotation=0 and preserved
    // post-rotation pixel_aspect=(1, 2) (since pasp=(2, 1) inverted by 90° turn is (1, 2)):
    assert_eq!(single_webm.tracks[0].width, 16);
    assert_eq!(single_webm.tracks[0].height, 32);
    assert_eq!(single_webm.tracks[0].rotation, 0);
    assert_eq!(single_webm.tracks[0].pixel_aspect(), (1, 2));
    let mut ffv1 = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        16,
        32,
        &single_webm.tracks[0].codec_private,
        32 << 20,
    )
    .unwrap();
    let frame0 = ffv1.decode(&single_webm.read_packet(0).unwrap()).unwrap();
    // Before negate: coded left 16x16 had Y=40, right 16x16 had Y=200.
    // After 90° clockwise turn, top 16x16 has Y=40 (negated: 255-40=215) and bottom 16x16 has Y=200 (negated: 255-200=55).
    assert_eq!(frame0.frame.data[0], 215);
    assert_eq!(frame0.frame.data[16 * 16], 55);

    // 3. Multitrack rotated MP4 lossless export with stream selection and interval trimming
    // (`owned_mp4_multitrack_export`):
    let out_multi = temp.join("multi-rotated-trimmed.mkv");
    let mut multi_opts = fvid::media_control::CopyOptions::default();
    multi_opts.streams = vec![0, 1, 2];
    multi_opts
        .metadata_set
        .push(("TITLE".into(), "RotatedMultitrack".into()));
    let multi_transform = fvid::media_info::LosslessTransform {
        interval: Some((0, 20_000)), // retains video frame 0 (t=0) and 960 samples per AAC track, trimming 2048 - 960 = 1088 samples per track
        ..Default::default()
    };
    let multi_plan =
        fvid::media::plan_transcode_lossless(&multi_mp4, &multi_transform, &multi_opts, Some("ffv1"))
            .expect("plan_transcode_lossless must support rotated multitrack MP4 with interval");
    assert_eq!(multi_plan.streams.len(), 3);
    let multi_stats =
        fvid::media::transcode_lossless(&multi_mp4, &out_multi, multi_transform, &multi_opts)
            .expect("transcode_lossless must support rotated multitrack MP4 with interval");
    assert_eq!(multi_stats.video_frames, 1);
    assert_eq!(multi_stats.copied_packets, 2);
    // Each of the 2 AAC tracks has 2048 total samples; interval (0, 20_000) retains 960 samples
    // (20000 * 48000 / 1_000_000 = 960), trimming 2048 - 960 = 1088 samples per track -> 2176 total!
    assert_eq!(
        multi_stats.trimmed_audio_sample_frames,
        2176,
        "owned_mp4_multitrack_export must report trimmed_audio_sample_frames across selected AAC tracks"
    );
    let mut multi_webm = fvid_media::owned_webm::WebmReader::open(
        BufReader::new(File::open(&out_multi).unwrap()),
        Default::default(),
    )
    .unwrap();
    multi_webm.scan_all().unwrap();
    assert_eq!(multi_webm.tracks.len(), 3);
    assert_eq!(multi_webm.tracks[0].width, 16);
    assert_eq!(multi_webm.tracks[0].height, 32);
    assert_eq!(multi_webm.tracks[0].rotation, 0);
    assert_eq!(multi_webm.tracks[0].pixel_aspect(), (1, 2));
    assert!(multi_webm.track_enabled(multi_webm.tracks[1].number));
    assert!(!multi_webm.track_enabled(multi_webm.tracks[2].number));

    // 4. `fvid::media::probe_as` / `fvid::media_probe::try_probe_as` format hint parity with `fvid_media::probe_as`:
    for hint in ["3gp", "3g2", "mj2", "mov,mp4,m4a,3gp,3g2,mj2"] {
        let info_top = fvid::media::probe_as(&single_mp4, Some(hint))
            .unwrap_or_else(|e| panic!("fvid::media::probe_as rejected MP4 format hint {hint:?}: {e}"));
        let info_media = fvid_media::probe_as(&single_mp4, Some(hint)).unwrap();
        assert_eq!(info_top.format, info_media.format);
        assert_eq!(info_top.streams.len(), 1);
    }
    let info_mkv_top = fvid::media::probe_as(&out_single, Some("matroska,webm"))
        .expect("fvid::media::probe_as must accept 'matroska,webm' format hint");
    let info_mkv_media = fvid_media::probe_as(&out_single, Some("matroska,webm")).unwrap();
    assert_eq!(info_mkv_top.format, info_mkv_media.format);

    let _ = std::fs::remove_dir_all(&temp);
}

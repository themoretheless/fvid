use fvid_media::{
    CopyOptions, LosslessTransform, decode_video, plan_transcode_lossless, transcode_lossless,
};
use std::fs;
use std::path::PathBuf;

#[test]
fn lossless_ffv1_compressed_video_and_multitrack_reuse_handles() {
    let temp = std::env::temp_dir().join(format!(
        "fvid-lossless-ffv1-handle-reuse-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    // 1. Y4M lossless plan & transcode (with and without interval)
    let y4m_in = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rotate-grid.y4m");
    let mkv_out1 = temp.join("out1.mkv");
    let plan1 = plan_transcode_lossless(
        &y4m_in,
        &LosslessTransform::default(),
        &CopyOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(plan1.command, "transcode-lossless");

    let interval_tx = LosslessTransform {
        interval: Some((0, 500_000)),
        ..Default::default()
    };
    let plan2 =
        plan_transcode_lossless(&y4m_in, &interval_tx, &CopyOptions::default(), None).unwrap();
    assert_eq!(plan2.command, "transcode-lossless");

    let stats1 =
        transcode_lossless(&y4m_in, &mkv_out1, interval_tx, &CopyOptions::default()).unwrap();
    assert_eq!(stats1.video_frames, 1);

    // 2. Matroska FFV1 lossless re-export (tests decode_webm_from_scanned_reader)
    let mkv_out2 = temp.join("out2.mkv");
    let mut controlled_opts = CopyOptions::default();
    controlled_opts.max_controlled_bytes = Some(16 << 20);
    let plan_mkv = plan_transcode_lossless(
        &mkv_out1,
        &LosslessTransform::default(),
        &controlled_opts,
        None,
    )
    .unwrap();
    assert_eq!(plan_mkv.command, "transcode-lossless");

    let stats2 = transcode_lossless(
        &mkv_out1,
        &mkv_out2,
        LosslessTransform::default(),
        &controlled_opts,
    )
    .unwrap();
    assert_eq!(stats2.video_frames, 1);

    // 3. Video decode across Matroska FFV1 and Y4M
    let dec_mkv = decode_video(&mkv_out1).unwrap();
    assert_eq!(dec_mkv.video_frames, 1);
    let dec_y4m = decode_video(&y4m_in).unwrap();
    assert_eq!(dec_y4m.video_frames, 3);

    let _ = fs::remove_dir_all(&temp);
}

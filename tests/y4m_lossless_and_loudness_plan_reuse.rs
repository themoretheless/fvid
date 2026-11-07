use std::fs;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fvid-batch628-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn y4m_concat_and_interval_export_reuse_layout_and_header_bounds() {
    let dir = temp_dir("y4m");
    let src1 = dir.join("seg1.y4m");
    let src2 = dir.join("seg2.y4m");
    let mut bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n".to_vec();
    for val in [32u8, 64] {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&[val, val, val, val, 128, 128]);
    }
    fs::write(&src1, &bytes).unwrap();
    fs::write(&src2, &bytes).unwrap();

    let options = fvid::media::CopyOptions::default();
    let plan = fvid::native_plan::concat_y4m_with_options(
        &[src1.clone(), src2.clone()],
        &options,
    )
    .unwrap();
    assert_eq!(plan.command, "concat");

    let dst_concat = dir.join("concat.y4m");
    let ev = fvid::native_export::concat_y4m_with_options(
        &[src1.clone(), src2.clone()],
        &dst_concat,
        &options,
    )
    .unwrap();
    assert_eq!(ev.packets, 4);

    let dst_trim = dir.join("trim.y4m");
    let ev_trim = fvid::native_export::export_y4m_pipeline_with_options(
        &src1,
        &dst_trim,
        Some((
            std::time::Duration::from_millis(0),
            std::time::Duration::from_millis(500),
        )),
        &Default::default(),
        &Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!(ev_trim.packets, 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lossless_plan_and_export_reuse_eligibility_and_loudness_plan_reuses_geometry() {
    let dir = temp_dir("lossless");
    let y4m = dir.join("src.y4m");
    let mut bytes = b"YUV4MPEG2 W2 H2 F1:1 Ip A1:1 C420\nFRAME\n".to_vec();
    bytes.extend_from_slice(&[64, 64, 64, 64, 128, 128]);
    fs::write(&y4m, &bytes).unwrap();

    let plan = fvid::native_plan::transcode_lossless(&y4m, &Default::default()).unwrap();
    assert_eq!(plan.command, "transcode-lossless");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].codec, "rawvideo");

    let mkv = dir.join("out.mkv");
    let stats = fvid::native_export::transcode_ffv1_selected(
        &y4m,
        &mkv,
        &Default::default(),
        &Default::default(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(stats.video_frames, 1);

    let plan_mkv = fvid::native_plan::transcode_lossless(&mkv, &Default::default()).unwrap();
    assert_eq!(plan_mkv.command, "transcode-lossless");
    assert_eq!(plan_mkv.streams.len(), 1);
    assert_eq!(plan_mkv.streams[0].codec, "ffv1");

    let mkv2 = dir.join("out2.mkv");
    let stats2 = fvid::native_export::transcode_ffv1_selected(
        &mkv,
        &mkv2,
        &Default::default(),
        &Default::default(),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(stats2.video_frames, 1);

    let ogg = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-opus-48k-stereo.ogg");
    if ogg.exists() {
        let mut opts = fvid::media::CopyOptions::default();
        opts.max_controlled_bytes = Some(64 * 1024 * 1024);
        let lplan = fvid::native_plan::loudness_with_options(&ogg, &opts, None).unwrap();
        assert_eq!(lplan.command, "loudness");
    }

    let _ = fs::remove_dir_all(&dir);
}

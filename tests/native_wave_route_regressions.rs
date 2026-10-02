use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn implicit_stereo_layout_rematrixes_without_legacy() {
    let video = fvid::native_probe::probe(&fixture("wave-rematrix-implicit.y4m")).unwrap();
    assert_eq!(video.duration_us, Some(100000));
    let source = fixture("wave-rematrix-implicit.wav");
    let destination = std::env::temp_dir().join(format!(
        "fvid-implicit-rematrix-{}.f32le",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&destination);
    let stats = fvid::native_export::export_audio_pcm_selected(
        &source,
        &destination,
        None,
        1.,
        Some(1),
        None,
        Some(0),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        (stats.sample_frames, stats.channels, stats.sample_rate),
        (4800, 1, 48000)
    );
    let bytes = std::fs::read(&destination).unwrap();
    assert_eq!(bytes.len(), 4800 * 4);
    assert!(
        bytes
            .chunks_exact(4)
            .all(|sample| f32::from_le_bytes(sample.try_into().unwrap()) == 0.25)
    );
    std::fs::remove_file(destination).unwrap();
}
fn assert_audio_copy_plan(name: &str) {
    let source = fixture(name);
    let paths = vec![source.clone(), source.clone()];
    let expected = if name.ends_with(".wav") {
        fvid::native_plan::concat_wave(&paths).unwrap()
    } else {
        let video = fvid::native_probe::probe(&fixture("aac-concat-route.y4m")).unwrap();
        assert_eq!(video.streams[0].duration, Some(3));
        fvid::native_plan::concat_adts(&paths).unwrap()
    };
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "concat"])
        .args(&paths)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    #[cfg(feature = "media")]
    assert_eq!(
        fvid::media::plan_concat(&paths, &Default::default()).unwrap(),
        expected
    );
    for format in ["mkv", "mka"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "concat"])
            .args(&paths)
            .args(["--output-format", format])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(plan["streams"][0]["disposition"], "decode_to_pcm");
        assert!(
            plan["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step["action"] == "write")
        );
    }
}

#[test]
fn wave_concat_plan_matches_copy_route_unless_matroska_is_explicit() {
    assert_audio_copy_plan("wave-probe-info.wav");
}
#[test]
fn adts_concat_plan_matches_copy_route_unless_matroska_is_explicit() {
    assert_audio_copy_plan("aac-concat-route.aac");
}

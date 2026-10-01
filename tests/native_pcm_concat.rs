use fvid::{
    media_control::{CancelFlag, ProgressHook},
    native_audio_mix, native_export,
};
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-pcm-concat-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn export(source: &Path, output: &Path) {
    native_export::export_audio_pcm_selected(
        source, output, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
}
#[test]
fn independent_aac_edits_and_mixed_containers_concatenate_exact_samples() {
    let d = dir("samples");
    let source = fixture("audio/aac-native-edit.m4a");
    let raw = d.0.join("source.f32le");
    let wave = d.0.join("source.wav");
    export(&source, &raw);
    export(&source, &wave);
    let sources = vec![source.clone(), wave, source];
    assert!(native_audio_mix::concat_eligible(&sources).unwrap());
    let output = d.0.join("joined.mka");
    let decoded = d.0.join("decoded.f32le");
    let stats = native_audio_mix::concat_audio(&sources, &output, None, None).unwrap();
    export(&output, &decoded);
    let cli = d.0.join("cli.mka");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "concat"])
        .arg(&cli)
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(cli).unwrap(), std::fs::read(&output).unwrap());
    let plan = fvid::native_plan::concat_mp4_matroska(&sources)
        .unwrap()
        .unwrap();
    assert!(plan.steps.iter().any(|s| s.action == "decode"));
    let original = std::fs::read(raw).unwrap();
    assert_eq!(std::fs::read(decoded).unwrap(), original.repeat(3));
    assert_eq!(
        stats.sample_frames * u64::from(stats.channels) * 4,
        (original.len() * 3) as u64
    );
    #[cfg(feature = "media")]
    {
        let public = d.0.join("public.mkv");
        let result = fvid::media::concat(&sources, &public, &Default::default()).unwrap();
        assert_eq!(result.backend, "fvid");
        assert_eq!(
            std::fs::read(public).unwrap(),
            std::fs::read(&output).unwrap()
        );
    }
    if let Some(ffmpeg) = std::env::var_os("FVID_REFERENCE_FFMPEG") {
        let result = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, original.repeat(3));
    }
}
#[test]
fn concat_cancellation_no_overwrite_and_done_follow_publication() {
    let d = dir("control");
    let source = fixture("audio/aac-native-edit.m4a");
    let sources = vec![source.clone(), source];
    let output = d.0.join("out.mka");
    let cancel = CancelFlag::new();
    let captured = cancel.clone();
    let hook = ProgressHook::new(move |event| {
        assert!(!event.done);
        captured.cancel();
    });
    assert!(native_audio_mix::concat_audio(&sources, &output, Some(&cancel), Some(&hook)).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
    let published = output.clone();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = done.clone();
    let hook = ProgressHook::new(move |e| {
        if e.done {
            assert!(published.exists());
            seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
    native_audio_mix::concat_audio(&sources, &output, None, Some(&hook)).unwrap();
    assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), 1);
    let bytes = std::fs::read(&output).unwrap();
    assert!(native_audio_mix::concat_audio(&sources, &output, None, None).is_err());
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
}
#[test]
fn incompatible_geometry_and_video_tracks_are_not_silently_omitted() {
    let sources = vec![
        fixture("audio/aac-stereo.aac"),
        fixture("audio/aac-51-active.aac"),
    ];
    assert!(!native_audio_mix::concat_eligible(&sources).unwrap());
    let video = fixture("short/avc-baseline.mp4");
    assert!(!native_audio_mix::concat_eligible(&[video.clone(), video]).unwrap());
}

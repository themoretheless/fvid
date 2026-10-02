//! Explicit mixed-container PCM concat sample comparison.
use fvid::{
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
    let plan = fvid::native_plan::concat_mp4_matroska(&sources)
        .unwrap()
        .unwrap();
    assert!(plan.steps.iter().any(|s| s.action == "decode"));
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].disposition, "decode_to_pcm");
    assert!(
        fvid::native_plan::concat_matroska_to(&sources, &output)
            .unwrap()
            .is_some()
    );
    assert!(
        fvid::native_plan::concat_matroska_to(&sources, &d.0.join("out.mp4"))
            .unwrap()
            .is_none()
    );
    let original = std::fs::read(raw).unwrap();
    assert_eq!(std::fs::read(decoded).unwrap(), original.repeat(3));
    assert_eq!(
        stats.sample_frames * u64::from(stats.channels) * 4,
        (original.len() * 3) as u64
    );
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

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    independent_aac_edits_and_mixed_containers_concatenate_exact_samples();
    println!("Mixed-container PCM concat reference comparison passed");
}

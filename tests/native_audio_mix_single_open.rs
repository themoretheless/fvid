use fvid::media::{self, CopyOptions, MixAudioOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-audio-mix-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn audio_mix_and_merge_single_open_and_blockwise_interleave() {
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");
    let sources = vec![wav.to_path_buf(), wav.to_path_buf()];
    let controls = CopyOptions::default();

    assert!(fvid::native_audio_mix::eligible_with_options(&sources, &controls).unwrap());
    assert!(fvid::native_audio_mix::concat_eligible_with_options(&sources, &controls).unwrap());

    let mix_plan = media::plan_mix_audio_with_controls(&sources, &MixAudioOptions::default(), &controls)
        .expect("WAVE mix-audio plan should succeed");
    assert_eq!(mix_plan.command, "mix-audio");

    let merge_plan = media::plan_merge_audio_with_controls(&sources, &controls)
        .expect("WAVE merge-audio plan should succeed");
    assert_eq!(merge_plan.command, "merge-audio");

    let dir = TempDir::new();
    let merged_wav = dir.0.join("merged.wav");
    let merge_stats = media::merge_audio_with_controls(&sources, &merged_wav, &controls)
        .expect("WAVE merge-audio execution should succeed");
    assert_eq!(merge_stats.inputs, 2);
    assert_eq!(merge_stats.channels, 2);
    assert!(merge_stats.sample_frames > 0);
}

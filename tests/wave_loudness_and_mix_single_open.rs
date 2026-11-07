use fvid::media::{self, CopyOptions, MixAudioOptions};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "fvid-{prefix}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn wave_loudness_loudnorm_and_audio_mix_single_open() {
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");

    let mut opts = CopyOptions::default();
    opts.max_controlled_bytes = Some(256 * 1024 * 1024);

    let wav_loudness = media::measure_loudness(wav, &opts).unwrap();
    assert!(wav_loudness.sample_frames > 0);

    let wav_plan = media::plan_loudness(wav, &opts).unwrap();
    assert_eq!(wav_plan.command, "loudness");

    let mp3_sources = vec![mp3.to_path_buf(), mp3.to_path_buf()];
    let mix_plan = media::plan_mix_audio_with_controls(&mp3_sources, &MixAudioOptions::default(), &opts).unwrap();
    assert_eq!(mix_plan.command, "mix-audio");

    let tmp = TempDir::new("wave-loudness-mix-single-open");
    let out_mix = tmp.path().join("mix.wav");
    let mix_stats = media::mix_audio_with_controls(&mp3_sources, &out_mix, &MixAudioOptions::default(), &opts).unwrap();
    assert!(mix_stats.sample_frames > 0);
    assert!(out_mix.exists());
}

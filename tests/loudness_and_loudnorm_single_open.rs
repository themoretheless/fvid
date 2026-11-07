use fvid::media::{self, CopyOptions};
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
fn container_loudness_and_wave_loudnorm_single_open() {
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");

    let mut opts = CopyOptions::default();
    opts.max_controlled_bytes = Some(256 * 1024 * 1024);

    let mp3_loudness_plan = media::plan_loudness(mp3, &opts).unwrap();
    assert_eq!(mp3_loudness_plan.command, "loudness");
    assert_eq!(mp3_loudness_plan.streams[0].codec, "mp3");

    let mka_loudness_plan = media::plan_loudness(mka, &opts).unwrap();
    assert_eq!(mka_loudness_plan.command, "loudness");
    assert_eq!(mka_loudness_plan.streams[0].codec, "alac");

    let wav_loudnorm_plan = media::plan_loudnorm(wav, None, false, &opts).unwrap();
    assert_eq!(wav_loudnorm_plan.command, "loudnorm");
    assert_eq!(wav_loudnorm_plan.streams[0].codec, "pcm_s32le");

    let tmp = TempDir::new("loudness-loudnorm-single-open");
    let out_wav = tmp.path().join("norm.wav");
    let stats = media::apply_loudnorm(mka, &out_wav, None, &opts).unwrap();
    assert!(stats.sample_frames > 0);
    assert!(out_wav.exists());
}

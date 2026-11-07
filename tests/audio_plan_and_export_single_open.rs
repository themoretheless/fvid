use fvid::media::{self, AudioDecodeTransform, CopyOptions};
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
fn audio_plan_and_export_dispatch_from_single_prefix_read() {
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let mka = Path::new("tests/fixtures/playback-errors/alac-resample-window.mka");
    let wav = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/wave-24in32-valid.wav");

    let mut opts = CopyOptions::default();
    opts.max_controlled_bytes = Some(256 * 1024 * 1024);
    let transform = AudioDecodeTransform::default();

    let flac_plan = media::plan_decode_audio(flac, &transform, &opts).unwrap();
    assert_eq!(flac_plan.streams[0].codec, "flac");

    let mp3_plan = media::plan_decode_audio(mp3, &transform, &opts).unwrap();
    assert_eq!(mp3_plan.streams[0].codec, "mp3");

    let mka_plan = media::plan_decode_audio(mka, &transform, &opts).unwrap();
    assert_eq!(mka_plan.streams[0].codec, "alac");

    let wav_plan = media::plan_decode_audio(wav, &transform, &opts).unwrap();
    assert_eq!(wav_plan.streams[0].codec, "pcm_s32le");

    let tmp = TempDir::new("audio-plan-export-single-open");
    let out_wav = tmp.path().join("out.wav");
    let stats = media::decode_audio_transformed(wav, &out_wav, transform, &opts).unwrap();
    assert!(stats.sample_frames > 0);
    assert!(out_wav.exists());
}

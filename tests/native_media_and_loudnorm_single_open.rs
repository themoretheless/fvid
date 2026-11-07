use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-{}-{}-{}",
            prefix,
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
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
fn native_media_audio_source_info_and_loudnorm_single_open() {
    let dir = TempDir::new("native-media-loudnorm");
    let source = dir.path().join("tone.wav");
    let sample_rate = 48_000u32;
    let frames = 24_000usize; // 500 ms >= 400 ms
    let data_bytes = (frames * 4) as u32;
    let mut wav = Vec::with_capacity(44 + frames * 4);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&32u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let sample = ((i as f32 * 440.0 * std::f32::consts::TAU) / sample_rate as f32).sin() * 0.25;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&source, &wav).unwrap();

    let options = fvid::media::CopyOptions::default();
    let plan = fvid::native_loudnorm::try_plan(&source, None, false, &options)
        .expect("try_plan should succeed")
        .expect("WAVE loudnorm plan");
    assert_eq!(plan.command, "loudnorm");

    // Export PCM to Matroska (.mka) and verify single-open inspect_audio_source_selected + is_owned_audio_source
    let mka = dir.path().join("source.mka");
    fvid::native_export::export_audio_pcm_transformed_with_controls(
        &source,
        &mka,
        None,
        1.0,
        None,
        None,
        &options,
    )
    .expect("export to mka");

    assert!(fvid::native_media::is_owned_audio_source_with_options(&mka, &options).expect("owned audio source"));
    assert!(fvid::native_media::is_owned_audio_trim_source_with_options(&mka, &options).expect("owned audio trim source"));
    let info = fvid::native_media::constant_audio_source_info_selected_with_options(&mka, None, &options)
        .expect("constant audio source info");
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(info.channels, 1);

    // Apply loudnorm on Matroska audio source to .wav via cached native_info
    let out_wav = dir.path().join("norm.wav");
    let stats = fvid::native_loudnorm::try_apply(&mka, &out_wav, None, false, &options)
        .expect("try_apply should succeed")
        .expect("stats");
    assert_eq!(stats.channels, 1);
    assert!(out_wav.is_file());
}

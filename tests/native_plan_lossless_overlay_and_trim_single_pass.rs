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
fn native_plan_transcode_lossless_and_overlay_single_pass() {
    let dir = TempDir::new("plan-lossless-overlay");
    let main = dir.path().join("main.y4m");
    let fg = dir.path().join("fg.y4m");
    let mut main_bytes = b"YUV4MPEG2 W4 H4 F2:1 Ip A1:1 C420\n".to_vec();
    let mut fg_bytes = b"YUV4MPEG2 W2 H2 F2:1 Ip A1:1 C420\n".to_vec();
    for val in [16u8, 64u8] {
        main_bytes.extend_from_slice(b"FRAME\n");
        main_bytes.extend_from_slice(&[val; 16]);
        main_bytes.extend_from_slice(&[128; 8]);
        fg_bytes.extend_from_slice(b"FRAME\n");
        fg_bytes.extend_from_slice(&[200, 200, 200, 200, 128, 128]);
    }
    std::fs::write(&main, &main_bytes).expect("write main");
    std::fs::write(&fg, &fg_bytes).expect("write fg");

    let lossless_plan = fvid::native_plan::transcode_lossless(&main, &Default::default())
        .expect("transcode_lossless plan");
    assert_eq!(lossless_plan.command, "transcode-lossless");

    let overlay_plan = fvid::native_plan::overlay(&main, &fg, 0, 0).expect("overlay plan");
    assert_eq!(overlay_plan.command, "overlay");

    let decode_overlay_plan =
        fvid::native_plan::decode_overlay(&main, &fg, 0, 0).expect("decode_overlay plan");
    assert_eq!(decode_overlay_plan.command, "decode");
}

#[test]
fn native_plan_loudness_and_trim_audio_single_pass_on_matroska_pcm() {
    let dir = TempDir::new("plan-loudness-trim");
    let source = dir.path().join("tone.wav");
    let sample_rate = 48_000u32;
    let frames = 24_000usize;
    let data_bytes = (frames * 4) as u32;
    let mut wav = Vec::with_capacity(44 + frames * 4);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&3u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
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
    .expect("export mka");

    let loud_plan = fvid::native_plan::loudness_with_options(&mka, &options, None)
        .expect("loudness_with_options on mka");
    assert_eq!(loud_plan.command, "loudness");

    let trim_plan = fvid::native_plan::trim_audio_with_options(&mka, 0, 100_000, &options)
        .expect("trim_audio_with_options on mka");
    assert_eq!(trim_plan.command, "trim");

    let trimmed_wav = dir.path().join("trimmed.wav");
    let stats = fvid::native_export::trim_audio_pcm_with_controls(&mka, &trimmed_wav, 0, 100_000, &options)
        .expect("trim_audio_pcm_with_controls on mka");
    assert_eq!(stats.sample_frames, 4_800);
    assert!(trimmed_wav.is_file());
}

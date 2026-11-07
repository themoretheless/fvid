use fvid::media::CopyOptions;
use fvid::native_audio_mix::MixAudioOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "fvid-test-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_wav(path: &std::path::Path, sample_rate: u32, samples: &[i16]) {
    let data_bytes = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn wave_plan_concat_and_audio_mix_decode_spool_reuse_inspected_metadata() {
    let dir = TempDir::new("batch-636");
    let wav = dir.path().join("tone.wav");
    write_wav(&wav, 48_000, &vec![4000i16; 12_000]);

    // FVID-0910: media::plan_concat reuses concat_info_with_cancel result
    let concat_plan =
        fvid::media::plan_concat(&[wav.clone(), wav.clone()], &CopyOptions::default()).unwrap();
    assert_eq!(concat_plan.command, "concat");
    assert_eq!(concat_plan.streams.len(), 1);
    assert_eq!(concat_plan.streams[0].codec, "pcm_s16le");

    let mka = dir.path().join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(
        &wav, &mka, None, 1.0, None, None, None, None, None,
    )
    .unwrap();

    // FVID-0912: native_audio_mix::eligible_with_options checks constant_audio_source_info_selected_with_options first
    let eligible = fvid::native_audio_mix::eligible_with_options(
        &[mka.clone(), mka.clone()],
        &CopyOptions::default(),
    )
    .unwrap();
    assert!(eligible);

    // FVID-0913: decode_source_to_f32le_with_options checks constant_audio_source_info_selected_with_options first
    let out_mix = dir.path().join("mixed.wav");
    let mix_stats = fvid::media::mix_audio_with_controls(
        &[mka.clone(), mka],
        &out_mix,
        &MixAudioOptions::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(mix_stats.backend, "fvid");
    assert_eq!(mix_stats.sample_frames, 12_000);
}

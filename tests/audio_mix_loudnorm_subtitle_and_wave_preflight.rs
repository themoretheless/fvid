use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(name: &str) -> TempDir {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let path = std::env::temp_dir().join(format!(
        "fvid-batch592-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    TempDir(path)
}

fn write_wav_s16le_mono(path: &std::path::Path, sample_rate: u32, samples: &[i16]) {
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36 + data_bytes;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn fvid_0690_audio_mix_and_merge_reject_existing_destination_upfront() {
    let dir = temp_dir("mix-merge-preflight");
    let existing = dir.0.join("existing.wav");
    std::fs::write(&existing, b"original").unwrap();
    let missing_a = dir.0.join("missing-a.wav");
    let missing_b = dir.0.join("missing-b.wav");
    let inputs = vec![missing_a.clone(), missing_b.clone()];

    let mix_err = fvid::media::mix_audio_with_controls(
        &inputs,
        &existing,
        &fvid::media::MixAudioOptions::default(),
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        mix_err.contains("already exists"),
        "expected existing destination refusal before probing inputs, got: {mix_err}"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");

    let merge_err = fvid::media::merge_audio_with_controls(
        &inputs,
        &existing,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        merge_err.contains("already exists"),
        "expected existing destination refusal before probing inputs, got: {merge_err}"
    );

    let wav_a = dir.0.join("a.wav");
    let wav_b = dir.0.join("b.wav");
    write_wav_s16le_mono(&wav_a, 48000, &[4096; 240]);
    write_wav_s16le_mono(&wav_b, 48000, &[-4096; 240]);
    let merged = dir.0.join("merged.wav");
    let stats = fvid::media::merge_audio_with_controls(
        &[wav_a, wav_b],
        &merged,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(stats.channels, 2);
    assert_eq!(stats.sample_frames, 240);
}

#[test]
fn fvid_0691_loudnorm_and_normalize_reject_existing_destination_without_progress() {
    let dir = temp_dir("loudnorm-normalize-preflight");
    let source = dir.0.join("source.wav");
    let samples: Vec<i16> = (0..24000)
        .map(|i| if (i / 100) % 2 == 0 { 8000 } else { -8000 })
        .collect();
    write_wav_s16le_mono(&source, 48000, &samples);

    let existing = dir.0.join("existing.wav");
    std::fs::write(&existing, b"keep").unwrap();
    let events = Arc::new(AtomicUsize::new(0));
    let events_clone = events.clone();
    let mut controls = fvid::media::CopyOptions::default();
    controls.progress = Some(fvid::media_control::ProgressHook::new(move |_| {
        events_clone.fetch_add(1, Ordering::Relaxed);
    }));

    let err = fvid::media::apply_loudnorm(
        &source,
        &existing,
        Some("I=-16:TP=-1.5:LRA=11"),
        &controls,
    )
    .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected upfront existing destination refusal for loudnorm, got: {err}"
    );
    assert_eq!(events.load(Ordering::Relaxed), 0);
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep");

    let norm_events = Arc::new(AtomicUsize::new(0));
    let norm_events_clone = norm_events.clone();
    let hook = fvid::native_pcm::NormalizationProgressHook::new(move |_| {
        norm_events_clone.fetch_add(1, Ordering::Relaxed);
    });
    let norm_err = fvid::native_pcm::normalize_loudness_file_with_options(
        &source,
        &existing,
        &[1.0],
        fvid::native_pcm::NormalizeTarget {
            integrated_lufs: -16.0,
            sample_peak_dbfs: -1.0,
        },
        &fvid::media::CopyOptions::default(),
        Some(&hook),
    )
    .unwrap_err();
    assert!(
        norm_err.to_string().contains("already exists"),
        "expected upfront existing destination refusal before pass-1 measurement, got: {norm_err}"
    );
    assert_eq!(norm_events.load(Ordering::Relaxed), 0);
}

#[test]
fn fvid_0692_wave_loudness_meter_and_measurement_guard_cancel_before_progress() {
    let dir = temp_dir("wave-loudness-cancel");
    let source = dir.0.join("source.wav");
    write_wav_s16le_mono(&source, 48000, &[4000; 4800]);

    let cancel = fvid::media_control::CancelFlag::default();
    cancel.cancel();
    let events = Arc::new(AtomicUsize::new(0));
    let events_clone = events.clone();
    let mut controls = fvid::media::CopyOptions::default();
    controls.cancel = Some(cancel);
    controls.progress = Some(fvid::media_control::ProgressHook::new(move |_| {
        events_clone.fetch_add(1, Ordering::Relaxed);
    }));

    let err = fvid_media::measure_loudness(&source, &controls).unwrap_err();
    assert!(err.contains("cancelled"), "unexpected error: {err}");
    assert_eq!(events.load(Ordering::Relaxed), 0);

    let mut meter = fvid::native_pcm::LoudnessMeter::new(48000, &[1.0]).unwrap();
    meter.push(&vec![0.25; 4800]).unwrap();
    let stats = meter.report();
    assert_eq!(stats.sample_frames, 4800);
}

#[test]
fn fvid_0693_subtitle_conversion_rejects_existing_destination_upfront() {
    let dir = temp_dir("subtitle-preflight");
    let existing = dir.0.join("existing.mkv");
    std::fs::write(&existing, b"original-mkv").unwrap();

    let srt = dir.0.join("sub.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
    let err = fvid::media::convert_subtitles_with_controls(
        &srt,
        &existing,
        &fvid::media::SubtitleConvertOptions::default(),
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected SRT subtitle conversion to reject existing destination upfront, got: {err}"
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original-mkv");

    let ass = dir.0.join("sub.ass");
    std::fs::write(
        &ass,
        "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,Hello\n",
    )
    .unwrap();
    let ass_err = fvid::media::convert_subtitles_with_controls(
        &ass,
        &existing,
        &fvid::media::SubtitleConvertOptions::default(),
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        ass_err.contains("already exists"),
        "expected ASS subtitle conversion to reject existing destination upfront, got: {ass_err}"
    );

    let valid_mkv = dir.0.join("valid-sub.mkv");
    fvid::media::convert_subtitles_with_controls(
        &srt,
        &valid_mkv,
        &fvid::media::SubtitleConvertOptions::default(),
        &fvid::media::CopyOptions::default(),
    )
    .unwrap();
    let mkv_err = fvid::media::convert_subtitles_with_controls(
        &valid_mkv,
        &existing,
        &fvid::media::SubtitleConvertOptions::default(),
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        mkv_err.contains("already exists"),
        "expected Matroska subtitle conversion to reject existing destination upfront, got: {mkv_err}"
    );
}

#[test]
fn fvid_0694_wave_and_adts_remux_concat_trim_reject_existing_destination_upfront() {
    let dir = temp_dir("wave-adts-preflight");
    let existing_wav = dir.0.join("existing.wav");
    std::fs::write(&existing_wav, b"keep-wav").unwrap();
    let missing_wav = dir.0.join("missing.wav");

    let trim_err =
        fvid::native_pcm::trim_wave(&missing_wav, &existing_wav, 0, 100_000, None, None)
            .unwrap_err();
    assert!(trim_err.to_string().contains("already exists"));

    let concat_err = fvid::native_pcm::concat_wave(
        &[missing_wav.clone(), missing_wav.clone()],
        &existing_wav,
        None,
        None,
    )
    .unwrap_err();
    assert!(concat_err.to_string().contains("already exists"));

    let existing_m4a = dir.0.join("existing.m4a");
    std::fs::write(&existing_m4a, b"keep-m4a").unwrap();
    let missing_aac = dir.0.join("missing.aac");

    let adts_remux_err = fvid::native_export::remux_adts_aac_with_options(
        &missing_aac,
        &existing_m4a,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(adts_remux_err.to_string().contains("already exists"));

    let adts_concat_err = fvid::native_export::concat_adts_aac_with_options(
        &[missing_aac.clone(), missing_aac],
        &existing_m4a,
        &fvid::media::CopyOptions::default(),
    )
    .unwrap_err();
    assert!(adts_concat_err.to_string().contains("already exists"));
}

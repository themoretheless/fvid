use fvid::{container::adts::Limits, native_media::decode_aac_pcm};

#[test]
fn headless_aac_export_matches_saved_pcm_reference() {
    let mut pcm = Vec::new();
    let stats = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut pcm, &Limits::default()).unwrap();
    assert_eq!((stats.sample_rate, stats.channels, stats.decoded_frames, stats.sample_frames),
        (44100, 1, 7, 7168));
    let oracle = include_bytes!("fixtures/audio/aac-mono-reference.f32le");
    assert_eq!(pcm.len(), oracle.len());
    let mut squared = 0.0f64;
    let mut peak = 0.0f64;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        let delta = f64::from(f32::from_le_bytes(a.try_into().unwrap()))
            - f64::from(f32::from_le_bytes(b.try_into().unwrap()));
        squared += delta * delta;
        peak = peak.max(delta.abs());
    }
    assert!((squared / stats.sample_frames as f64).sqrt() < 4e-5);
    assert!(peak < 3e-4);
}

#[test]
fn destination_errors_are_propagated() {
    struct Fails;
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("destination failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let error = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut Fails, &Limits::default()).unwrap_err();
    assert!(error.to_string().contains("destination failed"));
}

#[test]
fn cli_exports_complete_pcm_without_overwriting_or_partial_files() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(dir.clone());
    let source = dir.join("source.aac");
    let output = dir.join("out.f32le");
    let fixture = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    std::fs::write(&source, fixture).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"sample_frames\":7168"));
    let mut expected = Vec::new();
    decode_aac_pcm(fixture, &mut expected, &Limits::default()).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    assert!(fvid::native_export::export_aac_pcm(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    let frames = fvid::container::adts::Aac::parse(fixture, &Limits::default()).unwrap();
    let second = frames.frames[1];
    let mut corrupt = fixture.to_vec();
    corrupt[second.start + second.header_bytes..second.start + second.size].fill(0xff);
    std::fs::write(&source, corrupt).unwrap();
    let failed = dir.join("failed.f32le");
    assert!(fvid::native_export::export_aac_pcm(&source, &failed).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}

#[test]
fn float_wav_preserves_surround_samples_and_channel_layout() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-wav-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let source = dir.join("source.aac");
    let output = dir.join("out.wav");
    let input = include_bytes!("fixtures/audio/aac-51-active.aac");
    std::fs::write(&source, input).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let wave = std::fs::read(&output).unwrap();
    let read32 = |at| u32::from_le_bytes(wave[at..at+4].try_into().unwrap());
    assert_eq!(&wave[..4], b"RIFF");
    assert_eq!(read32(4) as usize + 8, wave.len());
    assert_eq!(&wave[8..16], b"WAVEfmt ");
    assert_eq!(read32(16), 40);
    assert_eq!(&wave[20..24], &[0xfe, 0xff, 6, 0]);
    assert_eq!(read32(24), 48000);
    assert_eq!(read32(28), 48000 * 24);
    assert_eq!(read32(40), 0x3f);
    assert_eq!(&wave[44..60], &[3,0,0,0,0,0,16,0,128,0,0,170,0,56,155,113]);
    assert_eq!(&wave[60..64], b"fact");
    assert_eq!(&wave[72..76], b"data");
    let mut pcm = Vec::new();
    let stats = decode_aac_pcm(input, &mut pcm, &Limits::default()).unwrap();
    assert_eq!(u64::from(read32(68)), stats.sample_frames);
    assert_eq!(read32(76) as usize, pcm.len());
    assert_eq!(&wave[80..], pcm);
    assert!(fvid::native_export::export_aac_pcm(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), wave);
    let clip = dir.join("clip.wav");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&clip)
        .args(["--from", "0.030001", "--to", "0.070001", "--quiet"])
        .output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(run.stdout.is_empty());
    let clipped = std::fs::read(clip).unwrap();
    assert_eq!(u32::from_le_bytes(clipped[68..72].try_into().unwrap()), 1920);
    assert_eq!(&clipped[80..], &pcm[1441*24..3361*24]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn interval_pcm_equals_exact_slice_including_decoder_preroll() {
    use std::time::Duration;
    let data = include_bytes!("fixtures/audio/aac-stereo.aac");
    let mut whole = Vec::new();
    decode_aac_pcm(data, &mut whole, &Limits::default()).unwrap();
    let mut part = Vec::new();
    let stats = fvid::native_media::decode_aac_pcm_interval(data, &mut part, &Limits::default(),
        Some((Duration::from_micros(30001), Duration::from_micros(70001)))).unwrap();
    // ceil(30001*48000/1e6)=1441; ceil(70001*48000/1e6)=3361.
    assert_eq!(stats.sample_frames, 1920);
    assert_eq!(stats.decoded_frames, 4);
    assert_eq!(part, whole[1441*8..3361*8]);
    for (from, to) in [(2, 3), (1, 1), (2, 1)] {
        let mut out = Vec::new();
        assert!(fvid::native_media::decode_aac_pcm_interval(data, &mut out, &Limits::default(),
            Some((Duration::from_secs(from), Duration::from_secs(to)))).is_err());
        assert!(out.is_empty());
    }
}

#[test]
fn mp4_aac_edit_removes_priming_and_encoder_tail() {
    let source = include_bytes!("fixtures/audio/aac-native-edit.m4a");
    let mut pcm = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(source, &mut pcm).unwrap();
    assert_eq!((stats.sample_rate, stats.channels, stats.sample_frames), (44100, 1, 5645));
    let oracle = include_bytes!("fixtures/audio/aac-native-edit-reference.f32le");
    assert!(oracle.len() >= pcm.len());
    let mut peak = 0.0f32;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        peak = peak.max((f32::from_le_bytes(a.try_into().unwrap())
            - f32::from_le_bytes(b.try_into().unwrap())).abs());
    }
    assert!(peak < 1e-6, "peak error {peak}");
}

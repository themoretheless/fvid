//! Separate deterministic AAC reconstruction from stochastic noise synthesis.
use std::process::Command;
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn non_noise_aac_reconstruction_matches_independent_decoder() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let directory =
        std::env::temp_dir().join(format!("fvid-aac-pns-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    for (rate, channels) in [(44100, 1), (48000, 1), (48000, 2), (96000, 1)] {
        let source = directory.join(format!("{rate}-{channels}.aac"));
        let result = Command::new(&binary)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=1000:sample_rate={rate}:duration=2"),
                "-ac",
                &channels.to_string(),
                "-c:a",
                "aac",
                "-aac_pns",
                "0",
            ])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = source.with_extension("f32le");
        let stats = fvid::native_export::export_audio_pcm_selected(
            &source, &output, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        let reference = Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(["-f", "f32le", "-"])
            .output()
            .unwrap();
        assert!(reference.status.success());
        let actual = std::fs::read(&output).unwrap();
        assert_eq!(
            actual.len(),
            reference.stdout.len(),
            "{rate}/{channels} samples"
        );
        assert_eq!(
            actual.len() as u64,
            stats.sample_frames * u64::from(stats.channels) * 4
        );
        let mut maximum = 0f64;
        let mut squared = 0f64;
        for (actual, reference) in actual.chunks_exact(4).zip(reference.stdout.chunks_exact(4)) {
            let difference = f64::from(
                f32::from_le_bytes(actual.try_into().unwrap())
                    - f32::from_le_bytes(reference.try_into().unwrap()),
            );
            maximum = maximum.max(difference.abs());
            squared += difference * difference;
        }
        let rms = (squared / (actual.len() / 4) as f64).sqrt();
        eprintln!("AAC no-PNS {rate}/{channels}: max={maximum}, rms={rms}");
        assert!(
            maximum < 1e-6 && rms < 1e-7,
            "{rate}/{channels}: max={maximum}, rms={rms}"
        );
    }
}

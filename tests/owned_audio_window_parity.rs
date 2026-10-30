use std::{io::Cursor, path::Path};
#[test]
fn synthetic_container_windows_preserve_exact_pcm_and_language_info() {
    let dir = std::env::temp_dir().join(format!("fvid-window-diagnostics-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let mut failures = Vec::new();
    for (name, precise) in [
        ("alac-resample-window.m4a", false),
        ("alac-resample-window.mka", false),
        ("pcm64-resample-window.mka", true),
    ] {
        let raw: Vec<u8> = (0..512)
            .flat_map(|i| {
                let v = (f64::from((i * 119) % 24001) - 12000.0) / 32768.0
                    + if precise {
                        f64::from(i % 5) * 1e-10
                    } else {
                        0.0
                    };
                if precise {
                    v.to_le_bytes().to_vec()
                } else {
                    (v as f32).to_le_bytes().to_vec()
                }
            })
            .collect();
        let mut reference = fvid_media::owned_wav::float_wav_header_with_precision(
            48000,
            1,
            512,
            4,
            if precise { 64 } else { 32 },
        )
        .unwrap();
        reference.extend_from_slice(&raw);
        let input = dir.join("reference.wav");
        std::fs::write(&input, reference).unwrap();
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((2000, 6000)),
            sample_rate: Some(44100),
            ..Default::default()
        };
        let expected_path = dir.join(format!("{name}-expected.wav"));
        fvid_media::owned_audio_export::decode_audio_transformed(
            &input,
            &expected_path,
            transform,
            &Default::default(),
        )
        .unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name);
        let actual_path = dir.join(format!("{name}-actual.wav"));
        fvid_media::owned_audio_export::decode_audio_transformed(
            &source,
            &actual_path,
            transform,
            &Default::default(),
        )
        .unwrap();
        let a = std::fs::read(&actual_path).unwrap();
        let b = std::fs::read(&expected_path).unwrap();
        let ai = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&a), None).unwrap();
        let bi = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&b), None).unwrap();
        let ap = &a[ai.data_offset as usize..][..ai.data_bytes as usize];
        let bp = &b[bi.data_offset as usize..][..bi.data_bytes as usize];
        println!(
            "{name}: whole_bytes={}/{}; PCM_bytes={}/{}; bits={}/{}; rate={}/{}; channels={}/{}; offsets={}/{}; first_PCM_difference={:?}",
            a.len(),
            b.len(),
            ap.len(),
            bp.len(),
            ai.bits_per_sample,
            bi.bits_per_sample,
            ai.sample_rate,
            bi.sample_rate,
            ai.channels,
            bi.channels,
            ai.data_offset,
            bi.data_offset,
            ap.iter().zip(bp).position(|(a, b)| a != b)
        );
        if ap != bp
            || ai.bits_per_sample != bi.bits_per_sample
            || ai.sample_rate != bi.sample_rate
            || ai.channels != bi.channels
        {
            failures.push(name);
        }
        let metadata =
            fvid_media::owned_wave_inspect::info_chunks(&mut Cursor::new(&a), &ai, None).unwrap();
        let expected_metadata: &[u8] = if name.ends_with(".mka") {
            b"LIST\x10\0\0\0INFOILNG\x04\0\0\0eng\0"
        } else {
            &[]
        };
        assert_eq!(
            metadata, expected_metadata,
            "{name}: preserve the authored/default language metadata exactly"
        );
        assert!(!ap.is_empty());
    }
    assert!(
        failures.is_empty(),
        "sample-domain parity failures: {failures:?}"
    );
}

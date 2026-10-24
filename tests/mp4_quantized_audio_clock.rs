use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-quantized-audio-clock")
}
fn compare(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(a.is_finite() && (a - b).abs() < 2e-6, "{a} != {b}");
    }
}
#[test]
fn coarse_cumulative_ticks_recover_the_native_clock_without_packet_drift() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let input =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let oracle = std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for interval in [
            None,
            Some((
                Duration::from_nanos(22001001),
                Duration::from_nanos(43007001),
            )),
        ] {
            let mut output = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            let expected = if let Some((a, b)) = interval {
                let at = |d: Duration| {
                    (d.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * stride
                };
                oracle[at(a)..at(b).min(oracle.len())].to_vec()
            } else {
                oracle.clone()
            };
            compare(&output, &expected);
            if c["name"].as_str().unwrap().starts_with("ps-edits-") {
                for (a, b) in output.chunks_exact(4).zip(expected.chunks_exact(4)) {
                    let a = f32::from_le_bytes(a.try_into().unwrap());
                    let b = f32::from_le_bytes(b.try_into().unwrap());
                    assert!(
                        (a - b).abs() <= 2. * f32::EPSILON * b.abs() + 2e-16,
                        "PS clock: {a} != {b}"
                    );
                }
            }
            assert_eq!(stats.sample_frames, expected.len() as u64 / stride as u64);
        }
    }
}
#[test]
fn an_extra_tick_at_an_exact_coded_endpoint_is_not_rounding() {
    let input = std::fs::read(root().join("false-exact-end-gap.mp4")).unwrap();
    let mut output = Vec::new();
    let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(input),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("packet duration disagrees"), "{e}");
}
#[test]
fn precise_pcm_coarse_clock_does_not_narrow_sub_float32_samples() {
    let expected = std::fs::read(root().join("pcm64.f64le")).unwrap();
    for mode in ["floor", "ceil", "nearest"] {
        let source = std::fs::read(root().join(format!("pcm64-900-{mode}.mp4"))).unwrap();
        let mut output = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
            Cursor::new(source),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(output.len(), expected.len());
        for (a, b) in output.chunks_exact(8).zip(expected.chunks_exact(8)) {
            let a = f64::from_le_bytes(a.try_into().unwrap());
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 1e-13, "{a} != {b}");
        }
    }
}
#[test]
fn native_and_library_files_keep_quantized_edit_waveforms_and_packet_prefixes() {
    for (name, oracle, rate) in [
        ("aac-edits-900-floor", "aac-edits.f32le", 48000),
        ("ssr-edits-1000-ceil", "ssr-edits.f32le", 24000),
        ("pcm-900-nearest", "pcm.f32le", 48000),
    ] {
        let source = root().join(format!("{name}.mp4"));
        let expected = std::fs::read(root().join(oracle)).unwrap();
        for native in [false, true] {
            let output = std::env::temp_dir().join(format!(
                "fvid-quantized-{}-{name}-{native}.wav",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            if native {
                fvid::native_export::export_audio_pcm_selected(
                    &source, &output, None, 1., None, None, None, None, None,
                )
                .unwrap();
            } else {
                fvid_media::decode_audio_transformed(
                    &source,
                    &output,
                    Default::default(),
                    &Default::default(),
                )
                .unwrap();
            }
            let mut f = std::fs::File::open(&output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
            assert_eq!(info.sample_rate, rate);
            let data = std::fs::read(&output).unwrap();
            let payload = &data[info.data_offset as usize
                ..(info.data_offset + u64::from(info.data_bytes)) as usize];
            let pcm = if info.bits_per_sample == 64 {
                payload
                    .chunks_exact(8)
                    .flat_map(|b| (f64::from_le_bytes(b.try_into().unwrap()) as f32).to_le_bytes())
                    .collect::<Vec<_>>()
            } else {
                payload.to_vec()
            };
            compare(&pcm, &expected);
            std::fs::remove_file(output).unwrap();
        }
    }
    let input = std::fs::read(root().join("pcm-900-floor.mp4")).unwrap();
    let mut output = Vec::new();
    let options = fvid_media::CopyOptions {
        max_packets: Some(2),
        ..Default::default()
    };
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(input),
        &mut output,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 1959);
    compare(
        &output,
        &std::fs::read(root().join("pcm.f32le")).unwrap()[..1959 * 8],
    );
}

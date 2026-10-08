use fvid::{
    codec::aac_native::NativeAacDecoder,
    container::adts::{Aac, Limits},
};
macro_rules! case {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(
                "fixtures/playback-errors/aac-pulse-band-",
                $name,
                ".aac"
            ))
            .as_slice(),
            include_bytes!(concat!(
                "fixtures/playback-errors/aac-pulse-band-",
                $name,
                ".f32le"
            ))
            .as_slice(),
        )
    };
}
#[test]
fn pulses_on_spectral_and_uncoded_bands_match_independent_pcm() {
    for (name, data, reference) in [
        case!("outside"),
        case!("outside-four"),
        case!("outside-far"),
        case!("cross-boundary"),
        case!("repeated-spectral"),
        case!("zero-spectral"),
        case!("zero-band"),
    ] {
        let reader = Aac::parse(data, &Limits::default()).unwrap();
        let mut decoder = NativeAacDecoder::new(&reader.configuration).unwrap();
        let mut pcm = Vec::new();
        for index in 0..reader.packets() {
            pcm.extend(
                decoder
                    .decode(reader.packet(index))
                    .unwrap_or_else(|e| panic!("{name}: {e}")),
            );
        }
        assert_eq!(pcm.len() * 4, reference.len());
        assert!(pcm.iter().any(|v| v.abs() > 0.00001), "{name}");
        for (i, (&a, b)) in pcm.iter().zip(reference.as_chunks::<4>().0).enumerate() {
            let delta = (a - f32::from_le_bytes(*b)).abs();
            assert!(delta < 1e-6, "{name}/{i}: {delta}");
        }
        decoder.reset();
        assert_eq!(decoder.decode(reader.packet(0)).unwrap(), pcm[..1024]);
    }
}

#[test]
fn pulses_do_not_modify_zero_noise_or_intensity_tools_or_random_state() {
    for (data, baseline) in [
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-outside.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-outside-baseline.aac")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-zero-band.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-zero-band-baseline.aac")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-noise-band.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-noise-band-baseline.aac")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-intensity14.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-intensity14-baseline.aac")
                .as_slice(),
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-intensity15.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-intensity15-baseline.aac")
                .as_slice(),
        ),
    ] {
        let reader = Aac::parse(data, &Limits::default()).unwrap();
        let baseline = Aac::parse(baseline, &Limits::default()).unwrap();
        let mut actual = NativeAacDecoder::new(&reader.configuration).unwrap();
        let mut expected = NativeAacDecoder::new(&baseline.configuration).unwrap();
        for index in 0..reader.packets() {
            let reference = expected.decode(baseline.packet(index)).unwrap();
            assert!(reference.iter().any(|v| v.abs() > 0.00001));
            assert_eq!(actual.decode(reader.packet(index)).unwrap(), reference);
        }
    }
}

#[test]
fn invalid_pulse_band_and_cumulative_offset_do_not_change_audio_state() {
    for (data, baseline, message) in [
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-invalid-start.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-invalid-start-baseline.aac")
                .as_slice(),
            "start band exceeds table",
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-pulse-band-invalid-offset.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-pulse-band-invalid-offset-baseline.aac")
                .as_slice(),
            "pulse exceeds spectrum",
        ),
    ] {
        let reader = Aac::parse(data, &Limits::default()).unwrap();
        let baseline = Aac::parse(baseline, &Limits::default()).unwrap();
        let mut actual = NativeAacDecoder::new(&reader.configuration).unwrap();
        let mut expected = NativeAacDecoder::new(&baseline.configuration).unwrap();
        actual.decode(baseline.packet(0)).unwrap();
        expected.decode(baseline.packet(0)).unwrap();
        assert!(
            actual
                .decode(reader.packet(1))
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        assert_eq!(
            actual.decode(baseline.packet(1)).unwrap(),
            expected.decode(baseline.packet(1)).unwrap()
        );
    }
}

#[test]
fn every_truncated_pulse_packet_preserves_the_next_frame_and_checkpoint() {
    let data = include_bytes!("fixtures/playback-errors/aac-pulse-band-outside-far.aac");
    let reader = Aac::parse(data, &Limits::default()).unwrap();
    for length in 0..reader.packet(1).len() {
        let mut actual = NativeAacDecoder::new(&reader.configuration).unwrap();
        let mut expected = NativeAacDecoder::new(&reader.configuration).unwrap();
        actual.decode(reader.packet(0)).unwrap();
        expected.decode(reader.packet(0)).unwrap();
        let state = actual.checkpoint();
        assert!(
            actual.decode(&reader.packet(1)[..length]).is_err(),
            "length {length}"
        );
        let wanted = expected.decode(reader.packet(1)).unwrap();
        assert_eq!(actual.decode(reader.packet(1)).unwrap(), wanted);
        actual.restore(&state).unwrap();
        assert_eq!(actual.decode(reader.packet(1)).unwrap(), wanted);
    }
}

#[test]
fn owned_pcm_wave_exports_and_synthetic_video_match_decoded_intervals() {
    let directory =
        std::env::temp_dir().join(format!("fvid-pulse-band-export-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    for name in ["cross-boundary", "noise-band", "intensity15"] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/aac-pulse-band-{name}.aac"
        ));
        let output = directory.join(format!("{name}.wav"));
        let data = std::fs::read(&source).unwrap();
        let reader = Aac::parse(&data, &Limits::default()).unwrap();
        let mut decoder = NativeAacDecoder::new(&reader.configuration).unwrap();
        let mut expected = Vec::new();
        for index in 0..reader.packets() {
            for v in decoder.decode(reader.packet(index)).unwrap() {
                expected.extend_from_slice(&v.to_le_bytes());
            }
        }
        fvid::native_export::export_aac_pcm(&source, &output).unwrap();
        let wave =
            fvid::native_pcm::inspect(&mut std::fs::File::open(&output).unwrap(), None).unwrap();
        assert_eq!((wave.channels, wave.sample_frames), (reader.channels, 6144));
        assert!(std::fs::read(output).unwrap().ends_with(&expected));
    }
    let data = include_bytes!("fixtures/playback-errors/aac-pulse-band-companion.y4m");
    let mut output = Vec::new();
    let stats = fvid::y4m::process(
        std::io::Cursor::new(data),
        &mut output,
        Default::default(),
        1 << 20,
    )
    .unwrap();
    assert_eq!(stats.frames, 6);
    assert_eq!(output, data);
}

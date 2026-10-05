use fvid::native_media::decode_matroska_aac_pcm_interval;
use std::time::Duration;

#[test]
fn short_frame_aac_matches_independent_pcm_across_window_transitions() {
    for (rate, source, reference) in [
        (
            48000,
            include_bytes!("fixtures/audio/aac-960-48000.mka").as_slice(),
            include_bytes!("fixtures/audio/aac-960-48000-reference.f32le").as_slice(),
        ),
        (
            96000,
            include_bytes!("fixtures/audio/aac-960-96000.mka").as_slice(),
            include_bytes!("fixtures/audio/aac-960-96000-reference.f32le").as_slice(),
        ),
        (
            8000,
            include_bytes!("fixtures/audio/aac-960-8000.mka").as_slice(),
            include_bytes!("fixtures/audio/aac-960-8000-reference.f32le").as_slice(),
        ),
    ] {
        let mut actual = Vec::new();
        let stats = decode_matroska_aac_pcm_interval(source, &mut actual, None).unwrap();
        assert_eq!(
            (
                stats.sample_rate,
                stats.channels,
                stats.decoded_frames,
                stats.sample_frames
            ),
            (rate, 1, 7, 7 * 960)
        );
        assert_eq!(actual.len(), reference.len());
        let mut peak = 0.0f64;
        let mut energy = 0.0f64;
        for (a, b) in actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let a = f32::from_le_bytes(*a);
            let b = f32::from_le_bytes(*b);
            assert!(a.is_finite());
            peak = peak.max(f64::from((a - b).abs()));
            energy += f64::from(b) * f64::from(b);
        }
        assert!(energy > 0.0001, "silent oracle");
        assert!(peak < 1e-6, "rate={rate}, peak={peak}");
        let from = Duration::from_micros(30001);
        let to = Duration::from_micros(60001);
        let mut part = Vec::new();
        decode_matroska_aac_pcm_interval(source, &mut part, Some((from, to))).unwrap();
        let first = (from.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize;
        let last = (to.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize;
        assert_eq!(part, actual[first * 4..last * 4]);
    }
}

#[test]
fn mp4_and_cli_preserve_960_sample_frames() {
    let mut pcm = Vec::new();
    let stats = fvid::native_media::decode_mp4_aac_pcm(
        include_bytes!("fixtures/audio/aac-960-48000.m4a"),
        &mut pcm,
    )
    .unwrap();
    assert_eq!((stats.sample_frames, stats.decoded_frames), (7 * 960, 7));
    let mut from_mka = Vec::new();
    decode_matroska_aac_pcm_interval(
        include_bytes!("fixtures/audio/aac-960-48000.mka"),
        &mut from_mka,
        None,
    )
    .unwrap();
    assert_eq!(pcm, from_mka);
    let dir = std::env::temp_dir().join(format!("fvid-aac960-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio/aac-960-48000.m4a");
    let destination = dir.join("out.wav");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(source)
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let wav = std::fs::read(destination).unwrap();
    assert_eq!(wav[80..], pcm);
}

#[cfg(feature = "player")]
#[test]
fn playback_adapter_outputs_960_and_reset_replays_first_packet() {
    let mut source = fvid::container::webm::WebmReader::open(
        std::io::Cursor::new(include_bytes!("fixtures/audio/aac-960-48000.mka")),
        Default::default(),
    )
    .unwrap();
    let esds = fvid::container::adts::esds_for(&source.tracks[0].codec_private).unwrap();
    let mut decoder = fvid::codec::aac_decoder::AacDecoder::new(&esds, 48000, 1).unwrap();
    let packet = source.read_packet(0).unwrap();
    let first = decoder.decode(&packet, 0, 960).unwrap().unwrap().data;
    assert_eq!(first.len(), 960 * 4);
    decoder
        .decode(&source.read_packet(1).unwrap(), 960, 960)
        .unwrap();
    decoder.reset();
    assert_eq!(
        decoder.decode(&packet, 0, 960).unwrap().unwrap().data,
        first
    );
}

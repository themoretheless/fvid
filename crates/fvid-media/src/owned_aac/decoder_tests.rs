use super::{NativeAacDecoder, adts};
use std::path::Path;
fn stream(name: &str) -> adts::Aac {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name);
    adts::Aac::parse(&std::fs::read(path).unwrap(), &Default::default()).unwrap()
}
#[test]
fn standalone_decoder_matches_saved_mono_pcm_and_exact_clock() {
    let source = stream("aac-mono-44k.aac");
    let mut decoder = NativeAacDecoder::new(&source.configuration).unwrap();
    assert_eq!(
        (
            decoder.sample_rate(),
            decoder.channels(),
            decoder.channel_mask()
        ),
        (44100, 1, 4)
    );
    let mut pcm = Vec::new();
    for i in 0..source.packets() {
        pcm.extend(decoder.decode(source.packet(i)).unwrap());
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio/aac-mono-reference.f32le");
    let reference = std::fs::read(path).unwrap();
    assert_eq!(pcm.len(), source.samples() as usize);
    assert_eq!(reference.len(), pcm.len() * 4);
    let mut squared = 0.;
    let mut peak = 0f64;
    for (&actual, bytes) in pcm.iter().zip(reference.chunks_exact(4)) {
        let error = f64::from(actual) - f64::from(f32::from_le_bytes(bytes.try_into().unwrap()));
        squared += error * error;
        peak = peak.max(error.abs());
    }
    assert!((squared / pcm.len() as f64).sqrt() < 4e-5);
    assert!(peak < 3e-4);
}
#[test]
fn standalone_checkpoint_restores_noise_overlap_and_configuration() {
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-pce-wide8.aac",
        "aac-tns.aac",
    ] {
        let source = stream(name);
        assert!(source.packets() >= 2);
        let mut decoder = NativeAacDecoder::new(&source.configuration).unwrap();
        decoder.decode(source.packet(0)).unwrap();
        let state = decoder.checkpoint();
        let expected = decoder.decode(source.packet(1)).unwrap();
        decoder.reset();
        decoder.restore(&state).unwrap();
        assert_eq!(decoder.decode(source.packet(1)).unwrap(), expected);
        assert!(expected.iter().all(|x| x.is_finite()));
        let other = if decoder.channels() == 1 {
            [0x11, 0x90]
        } else {
            [0x12, 0x08]
        };
        assert!(
            NativeAacDecoder::new(&other)
                .unwrap()
                .restore(&state)
                .is_err()
        );
    }
}

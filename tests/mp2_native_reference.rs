#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
#[test]
fn own_mp2_matches_saved_synthetic_tone_reference() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mp3/layer-ii.mp2");
    let bytes = std::fs::read(path).unwrap();
    let mut stream =
        fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
    let mut owned =
        fvid_codecs::mp2::Decoder::new(stream.sample_rate(), stream.channels()).unwrap();
    let mut reference = fvid::codec::symphonia_decoder::SymphoniaDecoder::new(
        "MP2 reference",
        symphonia::core::codecs::audio::well_known::CODEC_ID_MP2,
        &[],
        stream.sample_rate(),
        stream.channels(),
    )
    .unwrap();
    let mut count = 0;
    let mut squared = 0f64;
    let mut peak = 0f64;
    while let Some(packet) = stream.next_packet().unwrap() {
        let actual = owned.decode(&packet.data).unwrap();
        let expected = reference
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .unwrap()
            .unwrap();
        let expected: Vec<f32> = expected
            .data
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            let error = f64::from(a - b);
            squared += error * error;
            peak = peak.max(error.abs());
            count += 1;
        }
    }
    assert!(count > 1152);
    let rms = (squared / count as f64).sqrt();
    eprintln!("MP2 reference: {count} samples, peak error {peak}, RMS {rms}");
    assert!(
        peak < 5e-5 && rms < 5e-6,
        "PCM difference: peak {peak}, rms {rms}"
    );
}

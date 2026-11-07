use fvid_control::CopyOptions;
use fvid_media_info::LosslessTransform;
use std::fs;
use std::path::{Path, PathBuf};

fn unique_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch594-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn pcm_decoder_single_pass_finite_check_and_f64_decode() {
    use fvid_media::owned_pcm_decoder::{PcmDecoder, PcmFormat};
    let decoder = PcmDecoder::new(PcmFormat::Float { bits: 32 }, 48_000, 2).unwrap();
    let mut packet = Vec::new();
    packet.extend_from_slice(&0.25f32.to_le_bytes());
    packet.extend_from_slice(&(-0.5f32).to_le_bytes());
    let f32_samples = decoder.decode_pcm(&packet).unwrap();
    assert_eq!(f32_samples, vec![0.25, -0.5]);
    let f64_samples = decoder.decode_pcm_f64(&packet).unwrap();
    assert_eq!(f64_samples, vec![0.25, -0.5]);

    let mut nan_packet = packet.clone();
    nan_packet[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
    let err32 = decoder.decode_pcm(&nan_packet).unwrap_err().to_string();
    assert!(err32.contains("non-finite"), "unexpected error: {err32}");
    let err64 = decoder.decode_pcm_f64(&nan_packet).unwrap_err().to_string();
    assert!(err64.contains("non-finite"), "unexpected error: {err64}");
}

#[test]
fn mace_decoder_fallible_planar_and_output_reservation() {
    use fvid_media::owned_mace::{MaceCoding, MaceDecoder};
    let mut dec3 = MaceDecoder::new(MaceCoding::ThreeToOne, 22_050, 2).unwrap();
    let samples3 = dec3.decode_pcm(&[0x12, 0x34, 0x56, 0x78]).unwrap();
    assert_eq!(samples3.len(), 12);

    let mut dec6 = MaceDecoder::new(MaceCoding::SixToOne, 22_050, 1).unwrap();
    let samples6 = dec6.decode_pcm(&[0x5a, 0xa5]).unwrap();
    assert_eq!(samples6.len(), 12);
}

#[test]
fn webm_multitrack_export_rejects_bad_extension_and_existing_dest_before_video_decode() {
    let dir = unique_temp_dir("webm-multitrack-dest");
    let source = Path::new("tests/fixtures/playback-errors/av1-vp9-webm/vp9-opus-2frames.webm");
    let bad_ext = dir.join("out.mp4");
    let transform = LosslessTransform {
        horizontal_flip: true,
        ..Default::default()
    };
    let err = fvid_media::transcode_lossless(
        source,
        &bad_ext,
        transform.clone(),
        &CopyOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains(".mkv"),
        "expected .mkv extension rejection, got: {err}"
    );

    let existing = dir.join("existing.mkv");
    fs::write(&existing, b"keep").unwrap();
    let err_existing = fvid_media::transcode_lossless(
        source,
        &existing,
        transform,
        &CopyOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(
        err_existing.contains("already exists"),
        "expected existing output rejection, got: {err_existing}"
    );
    assert_eq!(fs::read(&existing).unwrap(), b"keep");
    let _ = fs::remove_dir_all(&dir);
}

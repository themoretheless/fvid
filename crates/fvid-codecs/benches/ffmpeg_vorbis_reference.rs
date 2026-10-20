//! Opt-in reference comparison only. Ordinary tests never execute FFmpeg.
use fvid_codecs::vorbis::{Identification, Setup, SetupLimits};
use fvid_codecs::vorbis_audio::Decoder;
fn main() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ogg/vorbis-stereo.ogg");
    let bytes = std::fs::read(&source).unwrap();
    let ogg = fvid_media::owned_ogg::Ogg::parse(&bytes, &Default::default()).unwrap();
    let chain = &ogg.chains()[0];
    let id = Identification::parse(&chain.packets[0].data).unwrap();
    let setup = Setup::parse(
        &chain.packets[2].data,
        id.channels,
        SetupLimits {
            entries: 1_000_000,
            nodes: 2_000_000,
            multiplicands: 1_000_000,
        },
    )
    .unwrap();
    let mut decoder = Decoder::new(id, setup, 64 * 1024 * 1024).unwrap();
    let mut owned = Vec::new();
    for packet in &chain.packets[3..] {
        owned.extend(decoder.decode(&packet.data).unwrap());
    }
    let raw_samples = owned.len();
    // This authored tone starts at zero; its EOS granule trims the overlap tail.
    // General Ogg initial offsets/chained timelines require the transport adapter.
    let end = chain
        .packets
        .iter()
        .rev()
        .find_map(|p| p.end_granule)
        .unwrap();
    let end = usize::try_from(end).unwrap() * usize::from(id.channels);
    assert!(end <= owned.len());
    owned.truncate(end);
    let tool = std::env::var_os("FVID_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let reference = std::process::Command::new(&tool)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:a:0",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .output()
        .expect("FFmpeg is required only for this explicit reference benchmark");
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    assert_eq!(reference.stdout.len() % 4, 0);
    let reference: Vec<_> = reference
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        owned.len(),
        reference.len(),
        "EOS-granule-qualified sample count"
    );
    let (mut max_error, mut squared_error, mut power) = (0f64, 0f64, 0f64);
    for (&a, &b) in owned.iter().zip(&reference) {
        assert!(a.is_finite() && b.is_finite());
        let error = f64::from(a) - f64::from(b);
        max_error = max_error.max(error.abs());
        squared_error += error * error;
        power += f64::from(b).powi(2);
    }
    let rmse = (squared_error / owned.len() as f64).sqrt();
    let snr = 10. * (power / squared_error).log10();
    println!(
        "{{\"sample_rate\":{},\"channels\":{},\"raw_owned_samples\":{},\"trimmed_samples\":{},\"max_error\":{},\"rmse\":{},\"snr_db\":{}}}",
        id.sample_rate,
        id.channels,
        raw_samples,
        owned.len(),
        max_error,
        rmse,
        snr
    );
    assert!(
        max_error < 0.000001,
        "Vorbis reference PCM error: {max_error}"
    );
}

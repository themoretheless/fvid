use fvid_media::owned_aac::{
    aac_bands::BandTables,
    aac_ltp_channel::LtpChannel,
    aac_ltp_syntax::{LtpData, Usage},
    aac_synthesis::{WindowSequence, WindowShape},
};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn authored_video_ltp_channel_matches_external_pcm_reference() {
    let offsets = BandTables::new(24000, 1024).unwrap().long;
    for name in ["inactive", "active"] {
        let gold = bytes(&format!("aac-ltp-{name}-external-reference.f32le"));
        assert_eq!(gold.len(), 12288 * 4);
        let mut channel = LtpChannel::new(1024).unwrap();
        let mut maxima = [0f64; 2];
        for frame in 0..12 {
            let mut residual = vec![0.; 1024];
            residual[..8]
                .copy_from_slice(&[1024., -1024., 1024., -1024., 0., 1024., -1024., 1024.]);
            let predictor = LtpData {
                lag: 1024,
                coefficient_index: frame % 8,
                usage: Usage::Bands(vec![true, true]),
            };
            let active = name == "active" && frame >= 3;
            let actual = channel
                .process(
                    residual,
                    active.then_some(&predictor),
                    WindowSequence::OnlyLong,
                    WindowShape::Sine,
                    offsets,
                    2,
                    None,
                )
                .unwrap();
            for (i, chunk) in gold[frame as usize * 4096..(frame as usize + 1) * 4096]
                .chunks_exact(4)
                .enumerate()
            {
                let expected = f32::from_le_bytes(chunk.try_into().unwrap()) as f64;
                maxima[usize::from(active)] =
                    maxima[usize::from(active)].max((actual[i] - expected).abs());
            }
        }
        println!(
            "{name}: inactive peak={} active peak={}",
            maxima[0], maxima[1]
        );
        assert!(
            maxima[0] < 1e-7,
            "inactive normalization mismatch: {}",
            maxima[0]
        );
        assert!(
            maxima[1] < 1e-7,
            "active prediction normalization mismatch: {}",
            maxima[1]
        );
    }
}

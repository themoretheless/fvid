use fvid_media::owned_aac::{
    aac_bands::BandTables, aac_channel::ChannelData, aac_ltp_channel::LtpChannel, bits::BitReader,
    config::AacConfig,
};
use serde_json::Value;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn ltp_packet_parse_reconstruction_and_channel_match_external_pcm() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-syntax.json")).unwrap();
    let blob = bytes("aac-ltp-packets.bin");
    let config = AacConfig {
        object_type: 4,
        sample_rate: 24000,
        channels: 1,
        channel_configuration: 1,
        frame_samples: 1024,
        core_coder_delay: None,
        section_data_resilience: false,
        scalefactor_data_resilience: false,
    };
    let offsets = BandTables::new(24000, 1024).unwrap().long;
    for video in m["videos"].as_array().unwrap() {
        let name = video["name"].as_str().unwrap();
        let reference = bytes(&format!("aac-ltp-{name}-external-reference.f32le"));
        let mut state = LtpChannel::new(1024).unwrap();
        for (frame, row) in video["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut bits = BitReader::new(raw);
            assert_eq!(bits.read(3).unwrap(), 0);
            assert_eq!(bits.read(4).unwrap(), 0);
            let (channel, prediction) = ChannelData::read_ltp(&mut bits, &config).unwrap();
            assert_eq!(
                bits.read(3).unwrap(),
                7,
                "packet end must follow parsed spectral payload"
            );
            assert_eq!(prediction.is_some(), name == "active" && frame >= 3);
            let spectrum = channel.ordinary_spectrum(&config).unwrap();
            let pcm = state
                .process(
                    spectrum,
                    prediction.as_ref(),
                    channel.info.sequence,
                    channel.info.shape,
                    offsets,
                    2,
                    channel.tns.as_ref(),
                )
                .unwrap();
            for (i, chunk) in reference[frame * 4096..(frame + 1) * 4096]
                .chunks_exact(4)
                .enumerate()
            {
                let gold = f32::from_le_bytes(chunk.try_into().unwrap()) as f64;
                assert!(
                    (pcm[i] - gold).abs() < 1e-7,
                    "{name} frame={frame} sample={i}: {} vs {gold}",
                    pcm[i]
                );
            }
            // Truncation inside the individual channel stream must not consume any bits.
            for len in 1..raw.len() - 1 {
                let mut bad = BitReader::new(&raw[..len]);
                bad.skip(7).unwrap();
                let before = bad.position();
                assert!(ChannelData::read_ltp(&mut bad, &config).is_err());
                assert_eq!(bad.position(), before);
            }
        }
    }
    let mut bad_profile = config.clone();
    bad_profile.object_type = 2;
    let mut bits = BitReader::new(&[0; 32]);
    assert!(ChannelData::read_ltp(&mut bits, &bad_profile).is_err());
    assert_eq!(bits.position(), 0);
}

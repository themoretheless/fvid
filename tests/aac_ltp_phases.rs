use fvid_media::owned_aac::{
    aac_bands::BandTables, aac_channel::ChannelData, aac_coupling_syntax::Coupling,
    aac_ltp_channel::LtpChannel, bits::BitReader, config::AacConfig,
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
fn parsed_ltp_tns_coupling_phases_match_independent_scalar_pcm() {
    let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-phase.json")).unwrap();
    let blob = bytes("aac-ltp-phase-packets.bin");
    let gold = bytes("aac-ltp-phase-reference.f32le");
    let config = AacConfig {
        object_type: 4,
        sample_rate: 24000,
        channels: 1,
        channel_configuration: 0,
        frame_samples: 1024,
        core_coder_delay: None,
        section_data_resilience: false,
        scalefactor_data_resilience: false,
    };
    let tables = BandTables::new(config.sample_rate, config.frame_samples as usize).unwrap();
    let mut wrong_point1_peak = 0f64;
    for case in manifest["cases"].as_array().unwrap() {
        let point = case["point"].as_u64().unwrap() as u8;
        let mut target = LtpChannel::new(1024).unwrap();
        let mut source = LtpChannel::new(1024).unwrap();
        for row in case["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut bits = BitReader::new(packet);
            let mut audio = None;
            let mut cce = None;
            loop {
                match bits.read(3).unwrap() {
                    0 => {
                        assert_eq!(bits.read(4).unwrap(), 0);
                        audio = Some(ChannelData::read_ltp(&mut bits, &config).unwrap());
                    }
                    2 => {
                        cce = Some(Coupling::read_ltp(&mut bits, &config).unwrap());
                    }
                    7 => break,
                    value => panic!("unexpected element {value}"),
                }
            }
            assert!(bits.remaining() < 8);
            let (channel, prediction) = audio.unwrap();
            let (coupling, source_prediction) = cce.unwrap();
            assert_eq!(coupling.point, point);
            assert_eq!(coupling.targets.len(), 1);
            assert_eq!(coupling.targets[0].gain, 1.);
            assert_eq!(
                prediction.is_some(),
                row["target"]["active"].as_bool().unwrap()
            );
            assert_eq!(
                source_prediction.is_some(),
                row["source"]["active"].as_bool().unwrap()
            );
            let raw = channel.ordinary_spectrum(&config).unwrap();
            let source_raw = coupling.channel.ordinary_spectrum(&config).unwrap();
            let prepared_source = source
                .prepare_spectrum(
                    source_raw.clone(),
                    source_prediction.as_ref(),
                    coupling.channel.info.sequence,
                    coupling.channel.info.shape,
                    tables.long,
                    2,
                    coupling.channel.tns.as_ref(),
                )
                .unwrap();
            // Preparation can be repeated without advancing source history/window shape.
            assert_eq!(
                prepared_source,
                source
                    .prepare_spectrum(
                        source_raw,
                        source_prediction.as_ref(),
                        coupling.channel.info.sequence,
                        coupling.channel.info.shape,
                        tables.long,
                        2,
                        coupling.channel.tns.as_ref()
                    )
                    .unwrap()
            );
            let mut residual = raw.clone();
            if point == 0 {
                for i in 0..8 {
                    residual[i] += prepared_source[i];
                }
            }
            let checkpoint = target.checkpoint();
            let before = target.retained_payload_bytes().unwrap();
            let mut prepared = target
                .prepare_spectrum(
                    residual.clone(),
                    prediction.as_ref(),
                    channel.info.sequence,
                    channel.info.shape,
                    tables.long,
                    2,
                    channel.tns.as_ref(),
                )
                .unwrap();
            assert_eq!(
                prepared,
                target
                    .prepare_spectrum(
                        residual,
                        prediction.as_ref(),
                        channel.info.sequence,
                        channel.info.shape,
                        tables.long,
                        2,
                        channel.tns.as_ref()
                    )
                    .unwrap()
            );
            if point == 1 {
                for i in 0..8 {
                    prepared[i] += prepared_source[i];
                }
            }
            // A refused synthesis must not commit the prior preparation's window shape or history.
            assert!(
                target
                    .synthesize_spectrum(
                        &prepared[..1023],
                        channel.info.sequence,
                        channel.info.shape
                    )
                    .is_err()
            );
            let mut invalid = prepared.clone();
            invalid[0] = f32::NAN;
            assert!(
                target
                    .synthesize_spectrum(&invalid, channel.info.sequence, channel.info.shape)
                    .is_err()
            );
            assert!(
                target
                    .prepare_spectrum(
                        invalid,
                        None,
                        channel.info.sequence,
                        channel.info.shape,
                        tables.long,
                        2,
                        None
                    )
                    .is_err()
            );
            let pcm = target
                .synthesize_spectrum(&prepared, channel.info.sequence, channel.info.shape)
                .unwrap();
            target.restore(&checkpoint).unwrap();
            assert_eq!(
                pcm,
                target
                    .synthesize_spectrum(&prepared, channel.info.sequence, channel.info.shape)
                    .unwrap()
            );
            assert_eq!(before, target.retained_payload_bytes().unwrap());
            let pcm: Vec<f64> = if point == 3 {
                let other = source
                    .synthesize_spectrum(
                        &prepared_source,
                        coupling.channel.info.sequence,
                        coupling.channel.info.shape,
                    )
                    .unwrap();
                pcm.iter()
                    .zip(other)
                    .map(|(&a, b)| f64::from(a as f32 + b as f32))
                    .collect()
            } else {
                pcm
            };
            let at = row["reference_offset"].as_u64().unwrap() as usize;
            for (i, &sample) in pcm.iter().enumerate() {
                let expected =
                    f32::from_le_bytes(gold[at + i * 4..at + i * 4 + 4].try_into().unwrap()) as f64;
                assert!(
                    (sample - expected).abs() < 1e-7,
                    "point={point} sample={i}: {sample} vs {expected}"
                );
            }
            if point == 1 {
                // Control: treating post-TNS coupling as pre-TNS changes this very fixture's PCM.
                let mut wrong = target.clone();
                wrong.restore(&checkpoint).unwrap();
                let mut mixed = raw;
                for i in 0..8 {
                    mixed[i] += prepared_source[i];
                }
                let wrong = wrong
                    .process(
                        mixed,
                        prediction.as_ref(),
                        channel.info.sequence,
                        channel.info.shape,
                        tables.long,
                        2,
                        channel.tns.as_ref(),
                    )
                    .unwrap();
                for (a, b) in wrong.iter().zip(&pcm) {
                    wrong_point1_peak = wrong_point1_peak.max((a - b).abs());
                }
            }
        }
    }
    assert!(
        wrong_point1_peak > 1e-5,
        "fixture must detect wrong coupling/TNS ordering: {wrong_point1_peak}"
    );
}

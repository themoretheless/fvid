use fvid_media::owned_aac::{
    aac_channel::ChannelData, aac_coupling_syntax::Coupling, aac_ltp_syntax::Usage,
    aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
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
fn owned_cce_ltp_syntax_gains_and_boundaries_match_authored_packets() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
    let blob = bytes("aac-ltp-coupling-packets.bin");
    assert_eq!(m["cases"].as_array().unwrap().len(), 17);
    for c in m["cases"].as_array().unwrap() {
        let channels = c["channels"].as_u64().unwrap() as u8;
        let mut config = AacConfig {
            object_type: 4,
            sample_rate: 24000,
            channels,
            channel_configuration: 0,
            frame_samples: 1024,
            core_coder_delay: None,
            section_data_resilience: false,
            scalefactor_data_resilience: false,
        };
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut bits = BitReader::new(raw);
            let mut seen = 0;
            loop {
                match bits.read(3).unwrap() {
                    0 => {
                        assert_eq!(bits.read(4).unwrap(), 0);
                        let (_, data) = ChannelData::read_ltp(&mut bits, &config).unwrap();
                        assert!(data.is_none());
                    }
                    1 => {
                        assert_eq!(bits.read(4).unwrap(), 0);
                        let (_, data) = ChannelPair::read_ltp(&mut bits, &config).unwrap();
                        assert!(data.iter().all(Option::is_none));
                    }
                    2 => {
                        assert_eq!(bits.position(), row["cce_start"].as_u64().unwrap() as usize);
                        let start = bits.position();
                        if row["active"].as_bool().unwrap() {
                            // The legacy ICS path rejects this very LTP presence bit;
                            // the new reader must accept the same complete CCE.
                            let mut legacy = BitReader::new(raw);
                            legacy.skip(start).unwrap();
                            let error = Coupling::read(&mut legacy, &config).err().unwrap();
                            assert!(
                                error
                                    .to_string()
                                    .contains("prediction is not allowed in AAC-LC"),
                                "{error}"
                            );
                            assert_eq!(legacy.position(), start);
                        }

                        let (source, data) = Coupling::read_ltp(&mut bits, &config).unwrap();
                        seen += 1;
                        assert_eq!(source.tag, 1);
                        assert_eq!(source.point, c["point"].as_u64().unwrap() as u8);
                        assert_eq!(data.is_some(), row["active"].as_bool().unwrap());
                        if let Some(data) = data {
                            assert_eq!(data.lag, row["lag"].as_u64().unwrap() as u16);
                            assert_eq!(
                                data.coefficient_index,
                                row["coefficient"].as_u64().unwrap() as u8
                            );
                            let Usage::Bands(used) = data.usage else {
                                panic!("expected long band use")
                            };
                            assert_eq!(
                                used,
                                row["used"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|v| v.as_bool().unwrap())
                                    .collect::<Vec<_>>()
                            );
                        }
                        let spectrum = source.channel.ordinary_spectrum(&config).unwrap();
                        for (i, &value) in spectrum.iter().enumerate() {
                            let expected = if i < 8 {
                                row["quantized"][i].as_i64().unwrap() as f32 * 1024.
                            } else {
                                0.
                            };
                            assert_eq!(value, expected);
                        }
                        let selection = c["selection"].as_u64().unwrap();
                        let signed = c["signed"].as_bool().unwrap();
                        let point = source.point;
                        let selected: Vec<u8> = if channels == 1 {
                            vec![0]
                        } else {
                            match selection {
                                0 | 3 => vec![0, 1],
                                1 => vec![1],
                                2 => vec![0],
                                _ => unreachable!(),
                            }
                        };
                        assert_eq!(source.targets.len(), selected.len());
                        for (target, &ch) in source.targets.iter().zip(&selected) {
                            assert_eq!(target.pair, channels == 2);
                            assert_eq!(target.tag, 0);
                            assert_eq!(target.channel, ch);
                            let separate_right = selection == 3 && ch == 1;
                            let gains = if signed && separate_right {
                                [-(0.5f32).sqrt(), -0.5]
                            } else {
                                [if separate_right { 0.5 } else { 1. }; 2]
                            };
                            assert!((target.gain - gains[1]).abs() < 1e-7);
                            if point == 3 {
                                assert!(target.bands.is_empty());
                            } else {
                                assert_eq!(target.bands.len(), 1);
                                for (actual, expected) in target.bands[0].iter().zip(gains) {
                                    assert!((*actual - expected).abs() < 1e-7);
                                }
                            }
                        }
                        for len in (start / 8 + 1)..raw.len() {
                            let mut short = BitReader::new(&raw[..len]);
                            short.skip(start).unwrap();
                            if len * 8 < bits.position() {
                                assert!(Coupling::read_ltp(&mut short, &config).is_err());
                                assert_eq!(short.position(), start);
                            }
                        }
                        config.object_type = 2;
                        let mut wrong = BitReader::new(raw);
                        wrong.skip(start).unwrap();
                        assert!(
                            Coupling::read_ltp(&mut wrong, &config)
                                .err()
                                .unwrap()
                                .to_string()
                                .contains("AAC LTP coupling requires AOT4")
                        );
                        assert_eq!(wrong.position(), start);
                        config.object_type = 4;
                    }
                    7 => break,
                    x => panic!("unexpected element {x}"),
                }
            }
            assert_eq!(seen, 1);
            assert!(bits.remaining() < 8);
        }
    }
}

#[test]
fn root_cce_parser_preserves_prediction_and_end_alignment() {
    use fvid::codec::{
        aac_coupling::Coupling, aac_ltp_syntax::Usage, bits::BitReader, config::AacConfig,
    };
    let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
    let blob = bytes("aac-ltp-coupling-packets.bin");
    for case in manifest["cases"].as_array().unwrap() {
        let config = AacConfig {
            object_type: 4,
            sample_rate: 24000,
            channels: case["channels"].as_u64().unwrap() as u8,
            channel_configuration: 0,
            frame_samples: 1024,
            core_coder_delay: None,
            section_data_resilience: false,
            scalefactor_data_resilience: false,
        };
        for row in case["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let mut bits = BitReader::new(packet);
            bits.skip(row["cce_start"].as_u64().unwrap() as usize)
                .unwrap();
            let (source, data) = Coupling::read_ltp(&mut bits, &config).unwrap();
            assert_eq!(source.point, case["point"].as_u64().unwrap() as u8);
            assert_eq!(data.is_some(), row["active"].as_bool().unwrap());
            if let Some(data) = data {
                assert_eq!(data.lag, row["lag"].as_u64().unwrap() as u16);
                let Usage::Bands(flags) = data.usage else {
                    panic!("expected bands")
                };
                assert_eq!(
                    flags,
                    row["used"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_bool().unwrap())
                        .collect::<Vec<_>>()
                );
            }
            // Compare actual root/owned payload ends, not just prediction metadata.
            let mut owned = fvid_media::owned_aac::bits::BitReader::new(packet);
            owned
                .skip(row["cce_start"].as_u64().unwrap() as usize)
                .unwrap();
            let cfg = fvid_media::owned_aac::config::AacConfig {
                object_type: 4,
                sample_rate: 24000,
                channels: config.channels,
                channel_configuration: 0,
                frame_samples: 1024,
                core_coder_delay: None,
                section_data_resilience: false,
                scalefactor_data_resilience: false,
            };
            let (expected, _) =
                fvid_media::owned_aac::aac_coupling_syntax::Coupling::read_ltp(&mut owned, &cfg)
                    .unwrap();
            assert_eq!(bits.position(), owned.position());
            assert_eq!(source.targets.len(), expected.targets.len());
            for (actual, expected) in source.targets.iter().zip(expected.targets) {
                assert_eq!(actual.gain, expected.gain);
                assert_eq!(actual.bands, expected.bands);
                assert_eq!(actual.channel, expected.channel);
            }
        }
    }
}

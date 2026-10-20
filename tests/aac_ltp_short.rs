use fvid_media::owned_aac::{
    aac_ltp_analysis::LtpAnalysis,
    aac_ltp_channel::LtpChannel,
    aac_ltp_history::LtpHistory,
    aac_ltp_syntax::{LtpData, ShortPrediction, Usage},
    aac_synthesis::{WindowSequence, WindowShape},
    aac_tns::{TnsData, TnsFilter},
    bits::BitReader,
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
fn short_window_ltp_history_analysis_and_channel_match_scalar_oracle() {
    let manifest: Value = serde_json::from_slice(&bytes("aac-ltp-short.json")).unwrap();
    let blob = bytes("aac-ltp-short-packets.bin");
    let gold = bytes("aac-ltp-short-reference.f32le");
    let inactive_gold = bytes("aac-ltp-short-inactive-reference.f32le");
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 4);

    // 1. Unit verification of LtpHistory::estimate_short and LtpAnalysis::analyze
    for n in [960usize, 1024] {
        let small = n / 8;
        let mut history = LtpHistory::new_float(n).unwrap();
        let pcm: Vec<f64> = (0..n).map(|i| (i as f64 * 13.5) - 2000.0).collect();
        let overlap: Vec<f64> = (0..n).map(|i| (i as f64 * -7.25) + 1500.0).collect();
        history.update_raw(&pcm, &overlap).unwrap();
        let mut windows = std::array::from_fn(|_| None);
        windows[0] = Some(ShortPrediction { lag_offset: None });
        windows[3] = Some(ShortPrediction {
            lag_offset: Some(12),
        });
        let data = LtpData {
            lag: n as u16,
            coefficient_index: 2,
            usage: Usage::Windows(windows),
        };
        let mut out0 = vec![0.0; 2 * small];
        let mut out3 = vec![0.0; 2 * small];
        let mut out1 = vec![99.0; 2 * small];
        history.estimate_short(&data, 0, &mut out0).unwrap();
        history.estimate_short(&data, 3, &mut out3).unwrap();
        history.estimate_short(&data, 1, &mut out1).unwrap();
        assert_ne!(out0, out3);
        assert!(out1.iter().all(|&x| x == 0.0));
        assert!(history.estimate_short(&data, 8, &mut out0).is_err());

        let mut analysis = LtpAnalysis::new(n).unwrap();
        let mut spec = vec![0.0; n];
        let input: Vec<f64> = (0..2 * n)
            .map(|i| (i as f64 * 0.125).sin() * 1024.0)
            .collect();
        analysis
            .analyze(
                &input,
                WindowSequence::EightShort,
                WindowShape::Kbd,
                WindowShape::Sine,
                &mut spec,
            )
            .unwrap();
        assert!(spec.iter().any(|x| x.abs() > 1.0));
    }

    // 2. End-to-end LtpData::read + LtpChannel trajectory verification across 960/1024 and TNS
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let n = case["n"].as_u64().unwrap() as usize;
        let small = n / 8;
        let long_offsets = [0, 4, 8, n];
        let short_offsets = [0, 4, 8, small];
        let mut channel = LtpChannel::new(n).unwrap();
        let mut diff_from_inactive = 0.0f32;
        for (frame_idx, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let seq = match row["sequence"].as_u64().unwrap() {
                0 => WindowSequence::OnlyLong,
                1 => WindowSequence::LongStart,
                2 => WindowSequence::EightShort,
                _ => WindowSequence::LongStop,
            };
            let shape = if row["shape"].as_u64().unwrap() == 0 {
                WindowShape::Sine
            } else {
                WindowShape::Kbd
            };
            let active = row["active"].as_bool().unwrap();
            let mut residual = vec![0.0f32; n];
            for (w, q_row) in row["q"].as_array().unwrap().iter().enumerate() {
                let base = if seq == WindowSequence::EightShort {
                    w * small
                } else {
                    0
                };
                for (k, v) in q_row.as_array().unwrap().iter().enumerate() {
                    residual[base + k] = (v.as_i64().unwrap() * 1024) as f32;
                }
            }
            let ltp_data = if active {
                if seq == WindowSequence::EightShort {
                    let at = row["ltp_offset"].as_u64().unwrap() as usize;
                    let len = row["ltp_bytes"].as_u64().unwrap() as usize;
                    let mut bits = BitReader::new(&blob[at..at + len]);
                    assert_eq!(bits.read(3).unwrap(), 5);
                    Some(LtpData::read(&mut bits, seq, 2, n as u16).unwrap())
                } else {
                    Some(LtpData {
                        lag: row["lag"].as_u64().unwrap() as u16,
                        coefficient_index: row["coefficient"].as_u64().unwrap() as u8,
                        usage: Usage::Bands(
                            row["used"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| v.as_bool().unwrap())
                                .collect(),
                        ),
                    })
                }
            } else {
                None
            };
            let tns_data = if !row["tns"].is_null() {
                let mut windows = Vec::with_capacity(8);
                for win_spec in row["tns"].as_array().unwrap() {
                    if win_spec.is_null() {
                        windows.push(Vec::new());
                    } else {
                        let arr = win_spec.as_array().unwrap();
                        let reverse = arr[0].as_bool().unwrap();
                        let lpc = arr[1].as_f64().unwrap();
                        windows.push(vec![
                            TnsFilter {
                                length: 1,
                                reverse: false,
                                lpc: Vec::new(),
                            },
                            TnsFilter {
                                length: 2,
                                reverse,
                                lpc: vec![lpc],
                            },
                        ]);
                    }
                }
                Some(TnsData { windows })
            } else {
                None
            };
            let offsets: &[usize] = if seq == WindowSequence::EightShort {
                &short_offsets
            } else {
                &long_offsets
            };
            let saved = channel.checkpoint();
            let pcm = channel
                .process(
                    residual.clone(),
                    ltp_data.as_ref(),
                    seq,
                    shape,
                    offsets,
                    2,
                    tns_data.as_ref(),
                )
                .unwrap_or_else(|e| panic!("{name} frame {frame_idx}: {e}"));
            let ref_at = row["reference_offset"].as_u64().unwrap() as usize;
            for (i, &sample) in pcm.iter().enumerate() {
                let sample = sample as f32;
                let expected =
                    f32::from_le_bytes(gold[ref_at + i * 4..ref_at + i * 4 + 4].try_into().unwrap());
                let inactive = f32::from_le_bytes(
                    inactive_gold[ref_at + i * 4..ref_at + i * 4 + 4]
                        .try_into()
                        .unwrap(),
                );
                diff_from_inactive = diff_from_inactive.max((sample - inactive).abs());
                assert!(
                    (sample - expected).abs() < 1e-6,
                    "{name} frame {frame_idx} sample {i}: {sample} vs {expected}"
                );
            }
            channel.restore(&saved).unwrap();
            assert_eq!(
                channel
                    .process(
                        residual,
                        ltp_data.as_ref(),
                        seq,
                        shape,
                        offsets,
                        2,
                        tns_data.as_ref()
                    )
                    .unwrap(),
                pcm
            );
        }
        assert!(
            diff_from_inactive > 1e-5,
            "{name} short-window LTP prediction had no measurable effect"
        );
    }
}

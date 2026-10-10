use fvid_media::owned_aac::{
    aac_ld_bands::LdBands,
    aac_ld_channel::LdChannel,
    aac_ld_ltp::LdLtpData,
    aac_ld_synthesis::LdWindowShape,
    aac_tns::{TnsData, TnsFilter},
    bits::BitReader,
};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-ld-ltp.json")).unwrap()
}
fn predictor(row: &serde_json::Value) -> Option<LdLtpData> {
    row["active"].as_bool().unwrap().then(|| LdLtpData {
        lag_update: row["lag_update"].as_u64().map(|n| n as u16),
        coefficient_index: row["coefficient"].as_u64().unwrap() as u8,
        used: row["used"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect(),
    })
}
fn shape(row: &serde_json::Value) -> LdWindowShape {
    if row["shape"].as_u64().unwrap() == 0 {
        LdWindowShape::Sine
    } else {
        LdWindowShape::LowOverlap
    }
}
fn tns(reverse: bool) -> TnsData {
    TnsData {
        windows: vec![vec![TnsFilter {
            length: 47,
            reverse,
            lpc: vec![(std::f64::consts::PI / 7.).sin()],
        }]],
    }
}
fn residual(row: &serde_json::Value, n: usize) -> Vec<f32> {
    let mut values = vec![0.; n];
    for (out, v) in values.iter_mut().zip(row["spectrum"].as_array().unwrap()) {
        *out = v.as_f64().unwrap() as f32;
    }
    values
}
#[test]
fn ld_ltp_tns_window_switches_match_scalar_pcm_and_checkpoint_replay() {
    let gold = bytes("aac-ld-ltp-reference.f32le");
    let blob = bytes("aac-ld-ltp-packets.bin");
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 4);
    for case in m["cases"].as_array().unwrap() {
        let n = case["n"].as_u64().unwrap() as usize;
        let mut channel = LdChannel::new(n).unwrap();
        let bands = LdBands::new(24000, n).unwrap();
        let filter = tns(case["reverse"].as_bool().unwrap());
        let retained = channel.retained_bytes(None).unwrap();
        let initial = channel.checkpoint();
        assert_eq!(
            channel.retained_bytes(Some(&initial)).unwrap(),
            retained + (3 * n + 1023) * 8
        );
        let mut first = vec![];
        for pass in 0..2 {
            channel.reset();
            let mut actual_all = vec![];
            for row in case["frames"].as_array().unwrap() {
                let data = predictor(row);
                // Confirm the authored video packet contains exactly the lag
                // update/reuse syntax being exercised by the scalar PCM case.
                let at = row["offset"].as_u64().unwrap() as usize;
                let size = row["bytes"].as_u64().unwrap() as usize;
                let mut bits = BitReader::new(&blob[at..at + size]);
                assert_eq!(bits.read(4).unwrap(), 0); // ER channel tag
                assert_eq!(bits.read(8).unwrap(), 140);
                assert_eq!(bits.read(3).unwrap(), 0); // reserved + ONLY_LONG
                assert_eq!(bits.read(1).unwrap(), row["shape"].as_u64().unwrap() as u32);
                assert_eq!(bits.read(6).unwrap(), 2);
                assert_eq!(bits.bit().unwrap(), data.is_some());
                if let Some(data) = &data {
                    assert!(bits.bit().unwrap());
                    assert_eq!(&LdLtpData::read(&mut bits, 2, n as u16).unwrap(), data);
                }
                let saved = channel.checkpoint();
                let actual = channel
                    .process(
                        residual(row, n),
                        data.as_ref(),
                        shape(row),
                        bands.offsets,
                        2,
                        Some(&filter),
                    )
                    .unwrap();
                channel.restore(&saved).unwrap();
                let replay = channel
                    .process(
                        residual(row, n),
                        data.as_ref(),
                        shape(row),
                        bands.offsets,
                        2,
                        Some(&filter),
                    )
                    .unwrap();
                assert_eq!(actual, replay);
                let offset = row["reference_offset"].as_u64().unwrap() as usize;
                for (i, (value, raw)) in actual
                    .iter()
                    .zip(gold[offset..offset + n * 4].chunks_exact(4))
                    .enumerate()
                {
                    let expected = f32::from_le_bytes(raw.try_into().unwrap()) as f64;
                    assert!(
                        (value - expected).abs() < 2e-9,
                        "case={} frame={} sample={i}: {value} vs {expected}",
                        case["name"],
                        row["offset"]
                    );
                }
                actual_all.extend(actual);
                assert_eq!(channel.retained_bytes(None).unwrap(), retained);
            }
            if pass == 0 {
                first = actual_all;
            } else {
                assert_eq!(first, actual_all);
            }
        }
    }
}
#[test]
fn ld_channel_errors_do_not_advance_lag_pcm_or_window_history() {
    for n in [480, 512] {
        let bands = LdBands::new(24000, n).unwrap();
        let mut channel = LdChannel::new(n).unwrap();
        channel
            .process(
                vec![1.; n],
                None,
                LdWindowShape::LowOverlap,
                bands.offsets,
                2,
                Some(&tns(false)),
            )
            .unwrap();
        let mut control = channel.clone();
        let invalid = LdLtpData {
            lag_update: Some(1024),
            coefficient_index: 0,
            used: vec![true; 2],
        };
        assert!(
            channel
                .process(
                    vec![1.; n],
                    Some(&invalid),
                    LdWindowShape::Sine,
                    bands.offsets,
                    2,
                    None
                )
                .is_err()
        );
        assert!(
            channel
                .synthesize_spectrum(&vec![1.; n], Some(&invalid), LdWindowShape::Sine)
                .is_err()
        );
        assert!(
            channel
                .process(
                    vec![f32::NAN; n],
                    None,
                    LdWindowShape::Sine,
                    bands.offsets,
                    2,
                    None
                )
                .is_err()
        );
        let invalid_tns = TnsData {
            windows: (0..8).map(|_| vec![]).collect(),
        };
        assert!(
            channel
                .process(
                    vec![1.; n],
                    None,
                    LdWindowShape::Sine,
                    bands.offsets,
                    2,
                    Some(&invalid_tns)
                )
                .is_err()
        );
        assert!(
            channel
                .restore(
                    &LdChannel::new(if n == 480 { 512 } else { 480 })
                        .unwrap()
                        .checkpoint()
                )
                .is_err()
        );
        let valid = LdLtpData {
            lag_update: None,
            coefficient_index: 3,
            used: vec![true; 2],
        };
        assert_eq!(
            channel
                .process(
                    vec![1.; n],
                    Some(&valid),
                    LdWindowShape::Sine,
                    bands.offsets,
                    2,
                    Some(&tns(true))
                )
                .unwrap(),
            control
                .process(
                    vec![1.; n],
                    Some(&valid),
                    LdWindowShape::Sine,
                    bands.offsets,
                    2,
                    Some(&tns(true))
                )
                .unwrap()
        );
    }
}
#[test]
fn active_ld_ltp_videos_reproduce_public_profile_gap_until_packet_dispatch() {
    for case in manifest()["cases"].as_array().unwrap() {
        let data = bytes(case["video"]["file"].as_str().unwrap());
        let mut pcm = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "only AAC Main, LC, SSR, LTP, ER-LC and ER-LTP core configurations are implemented"
            ),
            "{error}"
        );
    }
}

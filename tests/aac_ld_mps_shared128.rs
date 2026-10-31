use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_session::{Session, Stamp},
    aac_ld_mps_transport::{Extension, Kind, Message},
    aac_ld_qmf::Analysis,
    aac_sbr_qmf::Complex,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-shared128")
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn message(f: &Value) -> Message {
    let mut payload = hex(&f["spatial"]);
    let header = f["header"].as_bool().unwrap();
    if header {
        let cfg = hex(&f["ssc"]);
        assert!(cfg.len() < 127);
        let mut prefix = vec![cfg.len() as u8];
        prefix.extend(cfg);
        prefix.extend(payload);
        payload = prefix;
    }
    Message {
        extension: Extension::LowDelay,
        kind: if header {
            Kind::HeaderAndFrame
        } else {
            Kind::Frame
        },
        payload,
    }
}
#[test]
fn shared128_matches_pcm_analysis_native_movies_and_transactional_navigation() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let gold = std::fs::read(root().join("core-reference.f32le")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 66);
    let mut values = 0;
    let mut maximum = 0.0f64;
    for c in m["cases"].as_array().unwrap() {
        let mut native = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut native,
        )
        .unwrap();
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let mut pcm = Session::new(&cfg, Mode::Complex).unwrap();
        let mut qmf = Session::new_qmf(&cfg, Mode::Complex).unwrap();
        let fresh = qmf.clone();
        let inputs = n(&c["channels"]);
        let outputs = n(&c["output_channels"]);
        let samples = n(&c["n"]) * n(&c["group"]);
        let slots = n(&c["slots"]);
        assert_eq!(samples, 128 * slots);
        let offset = n(&c["core_reference_offset"]);
        let mut banks = (0..inputs)
            .map(|_| Analysis::<128>::new().unwrap())
            .collect::<Vec<_>>();
        let mut all = Vec::new();
        let mut traces = Vec::new();
        for (i, f) in c["reference"].as_array().unwrap().iter().enumerate() {
            let planar = (0..inputs)
                .map(|ch| {
                    (0..samples)
                        .map(|t| {
                            let at = offset + ((i * samples + t) * inputs + ch) * 4;
                            f64::from(f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut input = vec![vec![vec![Complex::default(); 128]; inputs]; slots];
            for ch in 0..inputs {
                for (t, row) in banks[ch]
                    .process(&planar[ch])
                    .unwrap()
                    .into_iter()
                    .enumerate()
                {
                    input[t][ch] = row.to_vec();
                }
            }
            let msg = message(f);
            let stamp = Stamp {
                pts: (i * samples) as i64,
                duration: samples as u64,
                timescale: n(&c["rate"]) as u32,
            };
            let before = qmf.clone();
            let want = pcm
                .submit(Some((&planar, stamp)), std::slice::from_ref(&msg))
                .unwrap();
            let got = qmf
                .submit_qmf(Some((&input, stamp)), std::slice::from_ref(&msg))
                .unwrap_or_else(|e| panic!("{} shared128 frame{i}: {e}", c["name"]));
            assert_eq!(want.len(), 1);
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].stamp, want[0].stamp);
            assert_eq!(got[0].sources, want[0].sources);
            for (a, b) in got[0].pcm.samples.iter().zip(&want[0].pcm.samples) {
                let d = (a - b).abs();
                maximum = maximum.max(d);
                values += 1;
                assert!(d < 1e-11, "{} shared/PCM {d}", c["name"]);
            }
            let mut retry = before.clone();
            let mut bad = input.clone();
            bad[slots - 1][inputs - 1][127].re = f64::NAN;
            assert!(
                retry
                    .submit_qmf(Some((&bad, stamp)), std::slice::from_ref(&msg))
                    .is_err()
            );
            assert_eq!(retry, before);
            let mut badmsg = msg.clone();
            badmsg.payload.push(0xff);
            assert!(retry.submit_qmf(Some((&input, stamp)), &[badmsg]).is_err());
            assert_eq!(retry, before);
            assert_eq!(
                retry
                    .submit_qmf(Some((&input, stamp)), std::slice::from_ref(&msg))
                    .unwrap(),
                got
            );
            assert_eq!(retry, qmf);
            let split = slots / 2;
            let mut chunks = before;
            let first = Stamp {
                duration: (split * 128) as u64,
                ..stamp
            };
            assert!(
                chunks
                    .submit_qmf(Some((&input[..split], first)), std::slice::from_ref(&msg))
                    .unwrap()
                    .is_empty()
            );
            let second = Stamp {
                pts: stamp.pts + (split * 128) as i64,
                duration: ((slots - split) * 128) as u64,
                ..stamp
            };
            let divided = chunks
                .submit_qmf(Some((&input[split..], second)), &[])
                .unwrap();
            assert_eq!(divided[0].pcm, got[0].pcm);
            assert_eq!(divided[0].stamp, got[0].stamp);
            assert_eq!(divided[0].sources.len(), 2);
            assert_eq!(chunks, qmf);
            for row in got[0].pcm.samples.chunks_exact(outputs) {
                for wave in 0..outputs {
                    let ch = if matches!(n(&c["tree"]), 4 | 6) {
                        [0, 1, 2, 3, 6, 7, 4, 5][wave]
                    } else {
                        wave
                    };
                    all.push(row[ch]);
                }
            }
            traces.push((input, stamp, msg, got));
        }
        pcm.finish().unwrap();
        qmf.finish().unwrap();
        assert_eq!(qmf.queued_samples(), 0);
        assert_eq!(qmf.queued_frames(), 0);
        qmf.reset();
        assert_eq!(qmf, fresh);
        for (input, stamp, msg, want) in traces {
            assert_eq!(qmf.submit_qmf(Some((&input, stamp)), &[msg]).unwrap(), want);
        }
        qmf.finish().unwrap();
        assert_eq!(native.len(), all.len() * 4);
        for (a, b) in all.iter().zip(native.chunks_exact(4)) {
            assert!((a - f64::from(f32::from_le_bytes(b.try_into().unwrap()))).abs() < 1e-9);
        }
    }
    assert!(values > 1_000_000);
    eprintln!("shared128 component PCM values {values}; maximum PCM-analysis difference {maximum}");
}

// Residual plus LD-filter compositions are numerical research,not standard LD-MPS.
use fvid_media::owned_aac::{
    aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm, aac_ld_mps_residual,
    aac_ld_qmf::Analysis, aac_mps_mdct_qmf, aac_sbr_qmf::Complex, bits::BitReader,
};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(CORPUS)
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
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

const PROFILE_ERROR: &str = "residual coding is not permitted in LD-MPS";
#[test]
fn bare_core_and_ordinary_controls_remain_valid_while_ld_residual_profiles_refuse() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 6);
    for c in m["controls"].as_array().unwrap() {
        let mut p = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut p,
        )
        .unwrap();
        assert_eq!(stats.sample_frames as usize, CORE_FRAMES * n(&c["n"]));
        let gold = std::fs::read(root().join(c["gold"].as_str().unwrap())).unwrap();
        assert_eq!(p.len(), gold.len());
        for (a, b) in p.chunks_exact(4).zip(gold.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), PROGRAMMES);
    for c in m["cases"].as_array().unwrap() {
        let mut ordinary = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap(),
            &mut ordinary,
        )
        .unwrap();
        assert_eq!(stats.sample_frames as usize, n(&c["samples"]));
        assert!(ordinary.iter().any(|&v| v != 0));
        let mut output = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(PROFILE_ERROR),
            "{} {error}",
            c["name"]
        );
        assert!(output.is_empty());
    }
}
fn reference(c: &Value, signal: bool, corrected: bool) -> Vec<f64> {
    let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
    let ctl = Config::parse(&hex(&c["control_ssc"])).unwrap();
    let extension = &cfg.extensions[0].value;
    let slots = n(&c["slots"]);
    let inputs = n(&c["channels"]);
    let outputs = n(&c["output_channels"]);
    let samples = slots * 64;
    let gold = std::fs::read(root().join(c["core_gold"].as_str().unwrap())).unwrap();
    let mut analysis = (0..inputs)
        .map(|_| Analysis::<64>::new().unwrap())
        .collect::<Vec<_>>();
    let mut residual = c["cutoffs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            if n(v) > 0 {
                Some(aac_mps_mdct_qmf::Decoder::new(slots, 1).unwrap())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut pcm = aac_ld_mps_pcm::Decoder::new_qmf(&ctl, Mode::Complex).unwrap();
    let mut result = Vec::new();
    for (i, f) in c["reference"].as_array().unwrap().iter().enumerate() {
        let mut input = vec![vec![vec![Complex::default(); 64]; inputs]; slots];
        for ch in 0..inputs {
            let values = (0..samples)
                .map(|t| {
                    let at = ((i * samples + t) * inputs + ch) * 4;
                    f64::from(f32::from_le_bytes(gold[at..at + 4].try_into().unwrap()))
                })
                .collect::<Vec<_>>();
            for (t, row) in analysis[ch]
                .process(&values)
                .unwrap()
                .into_iter()
                .enumerate()
            {
                input[t][ch] = row.to_vec();
            }
        }
        let wire = hex(&f["residual"]);
        let parsed =
            aac_ld_mps_residual::read(&mut BitReader::new(&wire), &cfg, extension, 2).unwrap();
        let rows = residual
            .iter_mut()
            .zip(&parsed.boxes)
            .map(|(b, data)| match (b, data) {
                (Some(b), Some(d)) => {
                    let mut v = b.process(&d.blocks).unwrap();
                    if !signal {
                        for row in &mut v {
                            row.fill(Complex::default());
                        }
                    }
                    Some(v)
                }
                (None, None) => None,
                _ => panic!("residual mask mismatch"),
            })
            .collect::<Vec<_>>();
        let out = pcm
            .decode_qmf(
                &mut BitReader::new(&hex(&f[if corrected { "control" } else { "uncorrected" }])),
                &input,
                &rows,
            )
            .unwrap();
        for row in out.samples.chunks_exact(outputs) {
            for wave in 0..outputs {
                let ch = if matches!(cfg.tree.code, 4 | 6) {
                    [0, 1, 2, 3, 6, 7, 4, 5][wave]
                } else {
                    wave
                };
                result.push(row[ch]);
            }
        }
    }
    result
}

#[test]
fn controlled_numerical_compositions_keep_signal_and_icc_effects_without_profile_claim() {
    let mut count = 0;
    let mut min_signal = f64::INFINITY;
    let mut min_icc = f64::INFINITY;
    for c in manifest()["cases"].as_array().unwrap() {
        let full = reference(c, true, true);
        let absent = reference(c, false, true);
        let uncorrected = reference(c, true, false);
        assert_eq!(full.len(), n(&c["samples"]) * n(&c["output_channels"]));
        assert!(full.iter().all(|v| v.is_finite()));
        let distance = |other: &[f64]| {
            full.iter()
                .zip(other)
                .fold(0.0f64, |m, (a, b)| m.max((a - b).abs()))
        };
        let signal = distance(&absent);
        let correction = distance(&uncorrected);
        assert!(signal > 1e-9);
        assert!(correction > 1e-9);
        min_signal = min_signal.min(signal);
        min_icc = min_icc.min(correction);
        count += full.len();
    }
    eprintln!(
        "NONCONFORMING LD composition values {count};signal {min_signal};ICC {min_icc};not codec acceptance"
    );
}
#[test]
fn mp4_matroska_exports_and_reader_factories_refuse_nonconforming_profiles_before_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        for mkv in [false, true] {
            let data =
                std::fs::read(root().join(c[if mkv { "file" } else { "video" }].as_str().unwrap()))
                    .unwrap();
            let mut output = Vec::new();
            let error = if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(&data),
                    &mut output,
                    None,
                    &Default::default(),
                )
                .unwrap_err()
            } else {
                fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    Cursor::new(&data),
                    &mut output,
                    None,
                    &Default::default(),
                )
                .unwrap_err()
            };
            assert!(
                error.to_string().contains(PROFILE_ERROR),
                "{} {error}",
                c["name"]
            );
            assert!(output.is_empty());
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(data),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(data),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            for _ in 0..2 {
                let error = match reader.make_decoder() {
                    Err(e) => e,
                    Ok(_) => panic!("invalid LD profile accepted"),
                };
                assert!(error.to_string().contains(PROFILE_ERROR));
                reader.rewind();
            }
        }
    }
}

use fvid_media::owned_aac::{
    aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm, aac_ld_mps_residual,
    aac_ld_qmf::Analysis, aac_mps_mdct_qmf, aac_sbr_qmf::Complex, bits::BitReader,
};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-residual64-multicore")
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
fn native(c: &Value, key: &str) -> Vec<u8> {
    let mut p = Vec::new();
    let s = fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join(c[key].as_str().unwrap())).unwrap(),
        &mut p,
    )
    .unwrap_or_else(|e| panic!("{} {key}: {e}", c["name"]));
    assert_eq!(s.sample_frames as usize, n(&c["samples"]));
    assert_eq!(s.channels as usize, n(&c["output_channels"]));
    assert_eq!(s.sample_rate, 48000);
    p
}
#[test]
fn all_cores_and_residual_movies_accept_two_core_source_windows() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 6);
    for c in m["controls"].as_array().unwrap() {
        let mut p = Vec::new();
        let s = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut p,
        )
        .unwrap();
        assert_eq!(s.sample_frames as usize, 24 * n(&c["n"]));
        let g = std::fs::read(root().join(c["gold"].as_str().unwrap())).unwrap();
        assert_eq!(p.len(), g.len());
        for (a, b) in p.chunks_exact(4).zip(g.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), 132);
    for c in m["cases"].as_array().unwrap() {
        let p = native(c, "video");
        assert!(p.iter().any(|&v| v != 0));
        let control = native(c, "control");
        assert_eq!(p.len(), control.len());
        assert_ne!(p, control, "{} residual had no effect", c["name"]);
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap()),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(p, owned);
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
fn complete_pcm_matches_scalar_core_plus_separate_residual_and_spatial_stages() {
    let mut count = 0;
    let mut max = 0.0f64;
    let mut signal_min = f64::INFINITY;
    let mut correction_min = f64::INFINITY;
    for c in manifest()["cases"].as_array().unwrap() {
        let want = reference(c, true, true);
        let absent = reference(c, false, true);
        let uncorrected = reference(c, true, false);
        let p = native(c, "video");
        assert_eq!(p.len(), want.len() * 4);
        let mut signal = 0.0f64;
        let mut correction = 0.0f64;
        for (((a, b), d), wire) in want
            .iter()
            .zip(absent)
            .zip(uncorrected)
            .zip(p.chunks_exact(4))
        {
            let got = f64::from(f32::from_le_bytes(wire.try_into().unwrap()));
            let delta = (a - got).abs();
            max = max.max(delta);
            assert!(delta < 1e-9, "{} PCM {delta}", c["name"]);
            signal = signal.max((a - b).abs());
            correction = correction.max((a - d).abs());
            count += 1;
        }
        assert!(signal > 1e-9, "{} residual signal missing", c["name"]);
        assert!(correction > 1e-9, "{} ICC correction missing", c["name"]);
        signal_min = signal_min.min(signal);
        correction_min = correction_min.min(correction);
    }
    eprintln!(
        "plain ELD residual64 composition values {count};max {max};signal minimum {signal_min};ICC minimum {correction_min}"
    );
}

fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    while let Some(p) = reader.next_packet()? {
        let stamp = reader.decoder_stamp(&p);
        if let Some(mut f) = reader.decode_source_packet(decoder, &p, stamp)? {
            if let Some(limit) =
                reader.decoded_packet_sample_limit(f.source_pts, f.source_duration)?
            {
                let bytes = limit * usize::from(reader.channels()) * 4;
                assert!(bytes <= f.packet.data.len());
                f.packet.data.truncate(bytes);
            }
            if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts)? {
                data.extend(pcm.data);
            }
        }
    }
    while let Some(mut f) = decoder.finish_packet()? {
        if let Some(limit) = reader.decoded_packet_sample_limit(f.source_pts, f.source_duration)? {
            let bytes = limit * usize::from(reader.channels()) * 4;
            assert!(bytes <= f.packet.data.len());
            f.packet.data.truncate(bytes);
        }
        if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts)? {
            data.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet()?.is_none());
    Ok(data)
}

#[test]
fn both_containers_play_rewind_seek_and_repeat_exact_source_ranges() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let full = native(c, "video");
        let stride = n(&c["output_channels"]) * 4;
        for matroska in [false, true] {
            let key = if matroska { "file" } else { "video" };
            let data = std::fs::read(root().join(c[key].as_str().unwrap())).unwrap();
            let mut p = Vec::new();
            if matroska {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(&data),
                    &mut p,
                    None,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(p, full);
                p.clear();
                fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut p, None).unwrap();
                assert_eq!(p, full);
            }
            let mut r: Box<dyn AudioStream> = if matroska {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(data.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(data.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            let mut d = r.make_decoder().unwrap();
            for _ in 0..2 {
                assert_eq!(
                    play(r.as_mut(), d.as_mut()).unwrap(),
                    full,
                    "{} initial/rewind",
                    c["name"]
                );
                r.rewind();
                d.reset();
            }
            for target in [
                13,
                3 * n(&c["n"]) + 7,
                8 * n(&c["n"]) + 1,
                16 * n(&c["n"]) + 7,
                7,
                full.len() / stride,
            ] {
                let ticks = (target as u128 * u128::from(r.timescale())
                    / u128::from(r.sample_rate())) as i64;
                let at = r.seek_to(ticks);
                d.reset();
                let first = ((at.max(0) as u128 * u128::from(r.sample_rate()))
                    .div_ceil(u128::from(r.timescale())) as usize
                    * stride)
                    .min(full.len());
                assert_eq!(
                    play(r.as_mut(), d.as_mut()).unwrap(),
                    full[first..],
                    "{} seek{target} MKV={matroska}",
                    c["name"]
                );
            }
            let rate = n(&c["rate"]) as u64;
            let a = std::time::Duration::from_nanos((13u64 * 1_000_000_000).div_ceil(rate));
            let b = std::time::Duration::from_nanos(
                ((5 * n(&c["n"]) + 7) as u64 * 1_000_000_000).div_ceil(rate),
            );
            let first = (a.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * stride;
            let last = (b.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * stride;
            for _ in 0..2 {
                let mut p = Vec::new();
                if matroska {
                    fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                        Cursor::new(&data),
                        &mut p,
                        Some((a, b)),
                        &Default::default(),
                    )
                    .unwrap();
                } else {
                    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                        Cursor::new(&data),
                        &mut p,
                        Some((a, b)),
                        &Default::default(),
                    )
                    .unwrap();
                }
                assert_eq!(
                    p,
                    full[first..last],
                    "{} repeated range MKV={matroska}",
                    c["name"]
                );
            }
        }
    }
}

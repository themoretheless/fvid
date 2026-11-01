use fvid_media::owned_aac::{
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_qmf,
    aac_ld_qmf::{Analysis, Synthesis},
    aac_sbr_qmf::Complex,
    bits::BitReader,
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
fn read(c: &Value, key: &str) -> Vec<u8> {
    std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()
}
fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn reference<const B: usize>(c: &Value, mode: Mode) -> Vec<u8> {
    let cfg = Config::parse(&hex(&c["base_ssc"])).unwrap();
    let mut qmf = aac_ld_mps_qmf::Decoder::new(&cfg, mode).unwrap();
    let input_channels = n(&c["input_channels"]);
    let channels = n(&c["channels"]);
    let slots = n(&c["slots"]);
    let samples = n(&c["out_n"]);
    let source = read(c, "source");
    let shared = c["shared_qmf"] == true;
    let mut analysis = (0..input_channels)
        .map(|_| Analysis::<B>::new().unwrap())
        .collect::<Vec<_>>();
    let mut synthesis = (0..channels)
        .map(|_| {
            if shared && B == 32 {
                Synthesis::<B>::new_single_rate_ld_sbr().unwrap()
            } else {
                Synthesis::<B>::new().unwrap()
            }
        })
        .collect::<Vec<_>>();
    let mut previous = [[1.0f64; 23]; 2];
    let mut result = Vec::new();
    for frame in 0..3 {
        let mut input = vec![vec![vec![Complex::default(); B]; input_channels]; slots];
        if shared {
            for t in 0..slots {
                for ch in 0..input_channels {
                    for q in 0..B {
                        let at = (((frame * slots + t) * input_channels + ch) * B + q) * 16;
                        input[t][ch][q] = Complex {
                            re: f64::from_le_bytes(source[at..at + 8].try_into().unwrap()),
                            im: f64::from_le_bytes(source[at + 8..at + 16].try_into().unwrap()),
                        };
                    }
                }
            }
        } else {
            for ch in 0..input_channels {
                let pcm = (0..samples)
                    .map(|t| {
                        let at = ((frame * samples + t) * input_channels + ch) * 4;
                        f64::from(f32::from_le_bytes(source[at..at + 4].try_into().unwrap()))
                    })
                    .collect::<Vec<_>>();
                for (t, row) in analysis[ch].process(&pcm).unwrap().into_iter().enumerate() {
                    input[t][ch] = row.to_vec();
                }
            }
        }
        let bytes = hex(&c["base_reference"][frame]);
        let residuals = vec![None; cfg.tree.ott_lfe.len() + usize::from(cfg.tree.ttt_boxes)];
        let base = qmf
            .decode(&mut BitReader::new(&bytes), &input, &residuals)
            .unwrap();
        let mut signal = vec![vec![vec![Complex::default(); B]; channels]; slots];
        let first = slots.div_ceil(2);
        for t in 0..slots {
            for q in 0..B {
                let band = usize::from(base.prepared.qmf_parameter_bands[q]);
                let ratio = 10.0f64.powf(c["extra_cld"][frame][band].as_f64().unwrap() / 10.0);
                let weights = [(ratio / (1.0 + ratio)).sqrt(), (1.0 / (1.0 + ratio)).sqrt()];
                for out in 0..channels {
                    let src = if out < 2 { 0 } else { out - 1 };
                    let fixed = c["fixed"][src].as_f64().unwrap();
                    let gain = if out < 2 {
                        if t < first {
                            previous[out][band]
                                + (weights[out] - previous[out][band]) * (t + 1) as f64
                                    / first as f64
                        } else {
                            weights[out]
                        }
                    } else {
                        1.0
                    };
                    let z = Complex {
                        re: base.dry[t][src][q].re + base.wet[t][src][q].re,
                        im: base.dry[t][src][q].im + base.wet[t][src][q].im,
                    };
                    let destination = c["positions"][frame][out]
                        .as_u64()
                        .map_or(out, |v| v as usize);
                    signal[t][destination][q] = Complex {
                        re: z.re * gain * fixed,
                        im: z.im * gain * fixed,
                    };
                }
            }
        }
        for band in 0..c["extra_cld"][frame].as_array().unwrap().len() {
            let ratio = 10.0f64.powf(c["extra_cld"][frame][band].as_f64().unwrap() / 10.0);
            previous[0][band] = (ratio / (1.0 + ratio)).sqrt();
            previous[1][band] = (1.0 / (1.0 + ratio)).sqrt();
        }
        let planar = (0..channels)
            .map(|ch| {
                synthesis[ch]
                    .process(
                        &signal
                            .iter()
                            .map(|s| std::array::from_fn(|q| s[ch][q]))
                            .collect::<Vec<[Complex; B]>>(),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for t in 0..samples {
            for p in &planar {
                result.extend_from_slice(&(p[t] as f32).to_le_bytes());
            }
        }
    }
    result
}
fn expected(c: &Value, mode: Mode) -> Vec<u8> {
    match n(&c["bank"]) {
        32 => reference::<32>(c, mode),
        64 => reference::<64>(c, mode),
        128 => reference::<128>(c, mode),
        _ => unreachable!(),
    }
}
fn near(a: &[u8], b: &[u8], name: &Value) -> f32 {
    assert_eq!(a.len(), b.len());
    let maximum = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(a, b)| {
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
        })
        .fold(0.0f32, f32::max);
    assert!(maximum < 1e-6, "{name}: max PCM error {maximum}");
    maximum
}
#[test]
fn all_trees_domains_and_fixed_gains_match_analytical_split_composition() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), PROGRAMMES);
    for c in m["cases"].as_array().unwrap() {
        let want = expected(c, Mode::Complex);
        for mkv in [false, true] {
            let mut out = Vec::new();
            let result = if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(read(c, "file")),
                    &mut out,
                    None,
                    &Default::default(),
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            result.unwrap_or_else(|e| panic!("{} mkv{mkv}: {e}", c["name"]));
            near(&out, &want, &c["name"]);
        }
    }
}
#[test]
fn all_trees_both_modes_preserve_packet_clocks_and_rollback() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        for mode in [Mode::Real, Mode::Complex] {
            let mut d = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(usize::from(d.channels()), n(&c["channels"]));
            assert_eq!(d.channel_mask(), 0);
            let mut out = Vec::new();
            for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = n(&r["offset"]);
                let packet = &blob[at..at + n(&r["bytes"])];
                let pts = (i * n(&c["out_n"])) as i64;
                let duration = n(&c["out_n"]) as u64;
                let state = d.checkpoint();
                let bytes = d.retained_payload_bytes().unwrap();
                assert!(d
                    .decode_timed(&packet[..packet.len() - 1], pts, duration)
                    .is_err());
                assert_eq!(d.retained_payload_bytes().unwrap(), bytes);
                let frame = d.decode_timed(packet, pts, duration).unwrap();
                d.restore(&state).unwrap();
                assert_eq!(d.decode_timed(packet, pts, duration).unwrap(), frame);
                if let Some(f) = frame {
                    assert_eq!(f.pts, pts);
                    assert_eq!(f.duration, duration);
                    out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
                }
            }
            while let Some(f) = d.finish().unwrap() {
                out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            near(&out, &expected(c, mode), &c["name"]);
        }
    }
}

fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&packet);
        if let Some(f) = reader
            .decode_source_packet(decoder.as_mut(), &packet, stamp)
            .unwrap()
        {
            if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(p) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn downscaled_spatial_playback_seek_and_rewind_keep_source_windows() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(c, "file")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(c, "video")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(play(reader.as_mut()), full);
            let core = n(&c["out_n"]);
            let spatial = core * n(&c["group"]);
            for sample in [
                1,
                core - 1,
                core,
                core + 1,
                spatial - 1,
                spatial,
                spatial + 1,
                n(&c["samples"]) / 2,
                n(&c["samples"]),
            ] {
                reader.rewind();
                let ticks = (sample as u128 * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * n(&c["channels"])
                    * 4;
                assert_eq!(
                    play(reader.as_mut()),
                    full[begin.min(full.len())..],
                    "{} seek{sample} mkv{mkv}",
                    c["name"]
                );
            }
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full);
        }
    }
}

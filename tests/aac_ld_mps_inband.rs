use fvid_media::owned_aac::{
    NativeAacDecoder, aac_ld_mps_config::Config, aac_ld_mps_decor::Mode, aac_ld_mps_pcm,
    bits::BitReader,
};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-inband")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn num(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn movie(c: &Value) -> Vec<u8> {
    std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap()
}
#[test]
fn inband_movies_accept_the_header_instead_of_refusing_the_whole_message() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let cfg = Config::parse(&hex(c["configs"][0].as_str().unwrap())).unwrap();
        assert_eq!(
            pcm.len(),
            num(&c["samples"]) * usize::from(cfg.tree.output_channels) * 4
        );
        assert!(pcm.iter().any(|&b| b != 0));
    }
}
fn staged_reference<const B: usize>(c: &Value, qmf: &[u8], reset_everything: bool) -> Vec<f64> {
    use fvid_media::owned_aac::{
        aac_ld_mps_prepare, aac_ld_mps_qmf,
        aac_ld_qmf::{Analysis, Synthesis},
        aac_sbr_qmf::Complex,
    };
    let parent = root().parent().unwrap().to_path_buf();
    let configs: Vec<_> = c["configs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Config::parse(&hex(s.as_str().unwrap())).unwrap())
        .collect();
    assert_ne!(configs[0].downmix_gain, configs[1].downmix_gain);
    let body = hex(c["body"].as_str().unwrap());
    let slots = num(&c["slots"]);
    let channels = num(&c["channels"]);
    let tree = num(&c["tree"]);
    let mut chain = aac_ld_mps_qmf::Decoder::new(&configs[0], Mode::Complex).unwrap();
    let outputs = usize::from(configs[0].tree.output_channels);
    let mut synthesis: Vec<_> = (0..outputs)
        .map(|_| {
            if B == 32 && !c["qmf"].is_null() {
                Synthesis::<B>::new_single_rate_ld_sbr().unwrap()
            } else {
                Synthesis::<B>::new().unwrap()
            }
        })
        .collect();
    let mut analysis: Vec<_> = (0..channels)
        .map(|_| Analysis::<B>::new().unwrap())
        .collect();
    let mut core = NativeAacDecoder::new(
        &std::fs::read(parent.join(format!("aac-eld-mps-core/tree{tree}.asc"))).unwrap(),
    )
    .unwrap();
    let mut current = 0usize;
    let mut expected = Vec::new();
    for i in 0..3 {
        let phase = num(&c["phase"][i]);
        if phase != current {
            for bank in &mut synthesis {
                bank.reset();
            }
            if reset_everything {
                chain.reset();
                for bank in &mut analysis {
                    bank.reset();
                }
            }
            current = phase;
        }
        let prepared = aac_ld_mps_prepare::Decoder::new(&configs[phase])
            .unwrap()
            .decode(&mut BitReader::new(&body))
            .unwrap();
        let mut rows = vec![vec![vec![Complex::default(); B]; channels]; slots];
        if c["qmf"].is_null() {
            let decoded = core
                .decode(
                    &std::fs::read(parent.join(format!("aac-eld-mps-core/tree{tree}-{i}.au")))
                        .unwrap(),
                )
                .unwrap();
            for ch in 0..channels {
                let pcm: Vec<_> = decoded
                    .chunks_exact(channels)
                    .map(|r| f64::from(r[ch]) * prepared.clip_protection_gain)
                    .collect();
                for (t, values) in analysis[ch].process(&pcm).unwrap().into_iter().enumerate() {
                    rows[t][ch] = values.to_vec();
                }
            }
        } else {
            let offset = num(&c["qmf"]["expected_offset"]);
            let stored = num(&c["qmf"]["expected_bands"]);
            for t in 0..slots {
                for ch in 0..channels {
                    for b in 0..B.min(stored) {
                        let at = offset + ((ch * 12 * slots + i * slots + t) * stored + b) * 16;
                        rows[t][ch][b] = Complex {
                            re: f64::from_le_bytes(qmf[at..at + 8].try_into().unwrap())
                                * prepared.clip_protection_gain,
                            im: f64::from_le_bytes(qmf[at + 8..at + 16].try_into().unwrap())
                                * prepared.clip_protection_gain,
                        };
                    }
                }
            }
        }
        let residuals = vec![
            None;
            configs[phase].tree.ott_lfe.len()
                + usize::from(configs[phase].tree.ttt_boxes)
        ];
        let out = chain
            .decode(&mut BitReader::new(&body), &rows, &residuals)
            .unwrap();
        let mut planar = Vec::new();
        for (ch, bank) in synthesis.iter_mut().enumerate() {
            let slots: Vec<[Complex; B]> = (0..slots)
                .map(|t| {
                    std::array::from_fn(|b| Complex {
                        re: out.dry[t][ch][b].re + out.wet[t][ch][b].re,
                        im: out.dry[t][ch][b].im + out.wet[t][ch][b].im,
                    })
                })
                .collect();
            planar.push(bank.process(&slots).unwrap());
        }
        let order = aac_ld_mps_pcm::native_order(tree as u8).unwrap();
        for t in 0..slots * B {
            for wave in 0..outputs {
                let c = if matches!(tree, 4 | 6) {
                    [0, 1, 2, 3, 6, 7, 4, 5][wave]
                } else {
                    wave
                };
                let gain = if outputs > 2 && c == 3 {
                    prepared.lfe_gain
                } else if outputs > 2
                    && (matches!(c, 4 | 5) || (matches!(tree, 4 | 6) && matches!(c, 6 | 7)))
                {
                    prepared.surround_gain
                } else {
                    1.0
                };
                expected.push(planar[order[c]][t] * gain);
            }
        }
    }
    expected
}
#[test]
fn header_changes_apply_to_their_own_source_frame_and_identical_headers_keep_history() {
    let qmf = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-eld-spatial-sbr/expected.f64le"),
    )
    .unwrap();
    let mut maximum = 0f64;
    let mut wrong_reset_gap = 0f64;
    for c in manifest()["cases"].as_array().unwrap() {
        let expected = if num(&c["bands"]) == 32 {
            staged_reference::<32>(c, &qmf, false)
        } else {
            staged_reference::<64>(c, &qmf, false)
        };
        let wrong = if num(&c["bands"]) == 32 {
            staged_reference::<32>(c, &qmf, true)
        } else {
            staged_reference::<64>(c, &qmf, true)
        };
        for (a, b) in expected.iter().zip(wrong) {
            wrong_reset_gap = wrong_reset_gap.max((a - b).abs());
        }
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut actual).unwrap();
        assert_eq!(actual.len(), expected.len() * 4);
        for (b, want) in actual.chunks_exact(4).zip(expected) {
            let got = f64::from(f32::from_le_bytes(b.try_into().unwrap()));
            maximum = maximum.max((got - want).abs());
            assert!((got - want).abs() < 2e-7, "{} {got} != {want}", c["name"]);
        }
    }
    assert!(wrong_reset_gap > 1e-8);
    eprintln!(
        "in-band native/staged maximum PCM difference {maximum}; all-state-reset negative-control gap {wrong_reset_gap}"
    );
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}
#[test]
fn changed_headers_restore_asc_on_live_rewind_and_seek_and_preserve_ranges() {
    use fvid::audio::AudioStream;
    use std::time::Duration;
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = movie(c);
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut full).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let width = usize::from(reader.channels()) * 4;
        let mut live = reader.make_decoder().unwrap();
        assert_eq!(play(&mut reader, live.as_mut()), full);
        reader.rewind();
        live.reset();
        for _ in 0..2 {
            let p = reader.next_packet().unwrap().unwrap();
            live.decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap();
        }
        reader.rewind();
        live.reset();
        assert_eq!(
            play(&mut reader, live.as_mut()),
            full,
            "{} reset",
            c["name"]
        );
        for target in [13, 512, 7, (full.len() / width) as i64] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            live.reset();
            assert_eq!(
                play(&mut reader, live.as_mut()),
                full[first..],
                "{} seek {target}",
                c["name"]
            );
        }
        let rate = u64::from(reader.sample_rate());
        for (from, to) in [(1, 7), (3, 17), (1, 7)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &bytes,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = ((from * rate).div_ceil(1000) as usize * width).min(full.len());
            let last = ((to * rate).div_ceil(1000) as usize * width).min(full.len());
            assert_eq!(range, full[first..last]);
        }
    }
}
#[test]
fn malformed_inband_movies_reproduce_specific_errors_and_retry_without_consuming_core() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for bad in m["errors"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["asc"] == bad["asc"] && c["mode"] == "repeat")
            .unwrap();
        let mut discarded = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie(bad), &mut discarded)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{} {error}",
            bad["name"]
        );
        let asc = hex(good["asc"].as_str().unwrap());
        let mut actual = NativeAacDecoder::new(&asc).unwrap();
        let mut reference = NativeAacDecoder::new(&asc).unwrap();
        let n = num(&good["slots"]) * num(&good["bands"]);
        for i in 0..3 {
            let row = &good["frames"][i];
            let at = num(&row["offset"]);
            let packet = &blob[at..at + num(&row["bytes"])];
            let expected = reference
                .decode_timed(packet, (i * n) as i64, n as u64)
                .unwrap();
            if i == 1 {
                let saved = actual.checkpoint();
                let before = actual.retained_payload_bytes().unwrap();
                let row = &bad["frames"][i];
                let at = num(&row["offset"]);
                let error = actual
                    .decode_timed(&blob[at..at + num(&row["bytes"])], n as i64, n as u64)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                assert_eq!(actual.retained_payload_bytes().unwrap(), before);
                assert_eq!(
                    actual.decode_timed(packet, n as i64, n as u64).unwrap(),
                    expected
                );
                actual.restore(&saved).unwrap();
            }
            assert_eq!(
                actual
                    .decode_timed(packet, (i * n) as i64, n as u64)
                    .unwrap(),
                expected
            );
        }
        assert_eq!(actual.finish().unwrap(), reference.finish().unwrap());
    }
}
#[test]
fn bounded_inband_header_reader_keeps_following_frame_bits_and_rolls_back() {
    let m = manifest();
    let config = hex(m["cases"][0]["configs"][0].as_str().unwrap());
    for size in [config.len(), 126, 127, 128, 129] {
        for aligned in [false, true] {
            for offset in [0, 3, 7] {
                let mut cfg = config.clone();
                cfg.resize(size, 0xf0);
                let expected = Config::parse(&cfg).unwrap();
                let mut header = if size < 127 {
                    vec![size as u8 | if aligned { 128 } else { 0 }]
                } else {
                    let n = (size - 127) as u16;
                    vec![127 | if aligned { 128 } else { 0 }, (n >> 8) as u8, n as u8]
                };
                header.extend(cfg);
                if aligned {
                    header.extend([0x12, 0x34]);
                }
                let used = offset + header.len() * 8;
                header.extend([0xde, 0xad]);
                let mut bytes = vec![0u8; (offset + header.len() * 8).div_ceil(8)];
                for (i, byte) in header.iter().enumerate() {
                    for bit in 0..8 {
                        if byte & (1 << (7 - bit)) != 0 {
                            let at = offset + i * 8 + bit;
                            bytes[at / 8] |= 1 << (7 - at % 8);
                        }
                    }
                }
                let mut bits = BitReader::new(&bytes);
                bits.skip(offset).unwrap();
                let (parsed, delay) = Config::read_in_band(&mut bits).unwrap();
                assert_eq!(parsed, expected);
                assert_eq!(delay, aligned.then_some(0x1234));
                assert_eq!(bits.position(), used);
                assert_eq!(bits.read(16).unwrap(), 0xdead);
                for end in offset..used {
                    let mut bits = BitReader::with_bit_len(&bytes, end).unwrap();
                    bits.skip(offset).unwrap();
                    assert!(Config::read_in_band(&mut bits).is_err());
                    assert_eq!(bits.position(), offset);
                }
            }
        }
    }
}

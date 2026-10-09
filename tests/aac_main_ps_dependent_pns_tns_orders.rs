use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
use serde_json::Value;
fn m() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-main-ps-dependent-pns-tns-orders.json"
    ))
    .unwrap()
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn dependent_pns_oracle_detects_omitted_short_window_predictor_reset() {
    for c in m()["cases"].as_array().unwrap() {
        let good = bytes(c["reference"].as_str().unwrap());
        let bad = bytes(c["discarded"].as_str().unwrap());
        assert_eq!(good.len(), bad.len());
        let max = good
            .chunks_exact(4)
            .zip(bad.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(
            max > 1e-6,
            "{} missing history sensitivity: {max}",
            c["name"]
        );
    }
}

fn reference(c: &Value) -> Vec<f32> {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps::Decoder, bits::BitReader};
    let core: Vec<_> = bytes(c["reference"].as_str().unwrap())
        .chunks_exact(4)
        .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
        .collect();
    let mut stage = Decoder::default();
    let mut output = vec![];
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let append = |f: fvid_media::owned_aac::aac_sbr_ps::Frame, out: &mut Vec<f32>| {
        out.extend(
            f.pcm[0]
                .iter()
                .zip(&f.pcm[1])
                .flat_map(|(l, r)| [*l as f32, *r as f32]),
        );
    };
    for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let crc = bits.read(4).unwrap() == 14;
        if let Some(f) = stage
            .read(
                &mut bits,
                raw.len() * 8,
                crc,
                &core[i * 1024..(i + 1) * 1024],
                48000,
                16,
                mode,
            )
            .unwrap()
        {
            append(f, &mut output);
        }
    }
    while let Some(f) = stage.finish().unwrap() {
        append(f, &mut output);
    }
    output
}
#[test]
fn main_ps_distinct_cce_predictors_and_windows_acceptance() {
    let blob = bytes("aac-main-ps-dependent-pns-tns-orders-packets.bin");
    for c in m()["cases"].as_array().unwrap() {
        let mut d = NativePsAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut pcm = vec![];
        let mut indices = vec![];
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let p = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
            let saved = d.checkpoint();
            let mut bad = p.to_vec();
            bad.push(0);
            assert!(
                d.decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("trailing bytes")
            );
            let output = d.decode(p).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(d.decode(p).unwrap(), output);
            if let Some(f) = output {
                indices.push(f.frame_index);
                pcm.extend(f.pcm);
            }
        }
        while let Some(f) = d.finish().unwrap() {
            indices.push(f.frame_index);
            pcm.extend(f.pcm);
        }
        assert_eq!(indices, (0..14).collect::<Vec<_>>());
        let gold = reference(c);
        assert_eq!(pcm.len(), gold.len());
        assert!(gold.chunks_exact(2).any(|s| (s[0] - s[1]).abs() > 1e-6));
        for (i, (a, b)) in pcm.iter().zip(&gold).enumerate() {
            assert!((a - b).abs() < 2e-7, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut root,
        )
        .unwrap();
        assert_eq!(
            root,
            pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()
        );
        d.reset();
        let mut replay = vec![];
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            if let Some(f) = d
                .decode(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                .unwrap()
            {
                replay.extend(f.pcm);
            }
        }
        while let Some(f) = d.finish().unwrap() {
            replay.extend(f.pcm);
        }
        assert_eq!(replay, pcm);
    }
}

#[test]
fn main_ps_cce_probe_ranges_rewind_seek_keep_per_tag_prediction() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::aac_ps_native::InBandPsProbe;
    use std::{io::Cursor, time::Duration};
    fn play(s: &mut dyn AudioStream) -> Vec<u8> {
        let mut d = s.make_decoder().unwrap();
        let mut pcm = vec![];
        while let Some(p) = s.next_packet().unwrap() {
            if let Some(f) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(out) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                    pcm.extend(out.data);
                }
            }
        }
        while let Some(f) = d.finish_packet().unwrap() {
            if let Some(out) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                pcm.extend(out.data);
            }
        }
        pcm
    }
    let blob = bytes("aac-main-ps-dependent-pns-tns-orders-packets.bin");
    for c in m()["cases"].as_array().unwrap() {
        let rate = c["container_rate"].as_u64().unwrap() as usize;
        let mut probe = InBandPsProbe::new(&hex(c["asc"].as_str().unwrap()), rate as u32).unwrap();
        for replay in 0..2 {
            if replay != 0 {
                probe.reset();
            }
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                assert!(
                    probe
                        .read(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                        .unwrap()
                );
            }
        }
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let mut range = vec![];
        fvid::native_media::decode_mp4_aac_pcm_interval(
            &data,
            &mut range,
            Some((Duration::from_millis(10), Duration::from_millis(400))),
        )
        .unwrap();
        assert_eq!(range, full[rate / 100 * 8..rate * 2 / 5 * 8]);
        let mut s =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap();
        assert_eq!(play(&mut s), full);
        s.rewind();
        assert_eq!(play(&mut s), full);
        let landed = s.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut s), full[landed as usize * 8..]);
    }
}

#[test]
fn target_tns_distinguishes_before_and_after_coupling() {
    let manifest = m();
    for source_tns in [false, true] {
        for rate in [24000, 48000] {
            let find = |point| {
                manifest["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| {
                        c["source_tns"] == source_tns
                            && c["tns"] == true
                            && c["point"] == point
                            && c["container_rate"] == rate
                    })
                    .unwrap()
            };
            let before = find(0);
            let after = find(1);
            let a = reference(before);
            let b = reference(after);
            assert_eq!(a.len(), b.len());
            let delta = a
                .iter()
                .zip(&b)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(
                delta > 1e-6,
                "{rate}: TNS ordering is not observable: {delta}"
            );
            let mut before_pcm = vec![];
            let mut after_pcm = vec![];
            fvid::native_media::decode_mp4_aac_pcm(
                &bytes(before["video"]["file"].as_str().unwrap()),
                &mut before_pcm,
            )
            .unwrap();
            fvid::native_media::decode_mp4_aac_pcm(
                &bytes(after["video"]["file"].as_str().unwrap()),
                &mut after_pcm,
            )
            .unwrap();
            assert_ne!(
                before_pcm, after_pcm,
                "{rate}: production collapsed the coupling points"
            );
        }
    }
}

#[test]
fn all_short_tns_order_resolution_compression_combinations_are_coded() {
    use fvid_media::owned_aac::{aac_synthesis::WindowSequence, aac_tns_syntax, bits::BitReader};
    use std::collections::BTreeSet;
    let blob = bytes("aac-main-ps-dependent-pns-tns-orders-syntax.bin");
    let rows: Value =
        serde_json::from_slice(&bytes("aac-main-ps-dependent-pns-tns-orders-syntax.json")).unwrap();
    let mut seen = BTreeSet::new();
    let mut signs = [false; 2];
    for row in rows.as_array().unwrap() {
        let at = row["offset"].as_u64().unwrap() as usize;
        let raw = &blob[at..at + row["bytes"].as_u64().unwrap() as usize];
        let mut bits = BitReader::new(raw);
        assert_eq!(bits.read(1).unwrap(), 1);
        let resolution = 3 + bits.read(1).unwrap();
        assert_eq!(bits.read(4).unwrap(), 15);
        let order = bits.read(3).unwrap();
        bits.read(1).unwrap();
        let compressed = bits.read(1).unwrap();
        for _ in 0..order {
            let width = (resolution - compressed) as u8;
            let raw = bits.read(width).unwrap();
            signs[usize::from(raw & (1 << (width - 1)) != 0)] = true;
        }
        let data = aac_tns_syntax::read_profile(
            &mut BitReader::new(raw),
            WindowSequence::EightShort,
            true,
        )
        .unwrap();
        assert_eq!(data.windows.len(), 8);
        assert_eq!(data.windows[0][0].lpc.len(), order as usize);
        assert!(data.windows[0][0].lpc.iter().all(|x| x.is_finite()));
        seen.insert((order, resolution, compressed));
    }
    let expected: BTreeSet<_> = (1..=7)
        .flat_map(|order| {
            (3..=4).flat_map(move |resolution| {
                (0..=1).map(move |compressed| (order, resolution, compressed))
            })
        })
        .collect();
    assert_eq!(seen, expected);
    assert_eq!(signs, [true, true]);
}

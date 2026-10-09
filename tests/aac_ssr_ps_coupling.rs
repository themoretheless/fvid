//! Configured SSR/PS coupling: independent SSR core, qualified PS stage,
//! enabled waveform acceptance and exact malformed-packet diagnostics.
use fvid_media::owned_aac::{
    aac_ps_native::NativePsAacDecoder,
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_ps::Decoder,
    bits::BitReader,
};
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-ssr-ps-cce.json")).unwrap()
}
fn artifact(name: &str) -> Vec<u8> {
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
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn video(c: &Value) -> Vec<u8> {
    artifact(c["video"]["file"].as_str().unwrap())
}
fn packets(c: &Value) -> Vec<Vec<u8>> {
    let blob = artifact("aac-ssr-ps-cce-packets.bin");
    c["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let at = r["offset"].as_u64().unwrap() as usize;
            blob[at..at + r["bytes"].as_u64().unwrap() as usize].to_vec()
        })
        .collect()
}
fn core(c: &Value) -> Vec<f32> {
    let raw = artifact("aac-ssr-ps-cce-core.f32le");
    let at = c["pcm_offset"].as_u64().unwrap() as usize;
    raw[at..at + c["pcm_bytes"].as_u64().unwrap() as usize]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn reference(c: &Value) -> Vec<f32> {
    let pcm = core(c);
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let point = c["point"].as_i64().unwrap();
    let mut stage = Decoder::default();
    let mut sources = std::collections::BTreeMap::<
        u64,
        (fvid_media::owned_aac::aac_sbr_history::Stream, Dsp),
    >::new();
    let mut left = std::collections::VecDeque::new();
    let mut out = vec![];
    let append = |frame: fvid_media::owned_aac::aac_sbr_ps::Frame,
                  left: &mut std::collections::VecDeque<Vec<f64>>,
                  out: &mut Vec<f32>| {
        let coupled = left.pop_front().unwrap();
        assert_eq!(frame.pcm[0].len(), coupled.len());
        out.extend(
            frame.pcm[0]
                .iter()
                .zip(&frame.pcm[1])
                .zip(coupled)
                .flat_map(|((&l, &r), v)| [l as f32 + v as f32, r as f32]),
        );
    };
    for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let target = if point == 3 {
            vec![0.; 1024]
        } else {
            pcm[i * 1024..(i + 1) * 1024].to_vec()
        };
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let crc = bits.read(4).unwrap() == 14;
        let rendered = stage
            .read(&mut bits, raw.len() * 8, crc, &target, 48000, 16, mode)
            .unwrap();
        let mut coupled = vec![0f64; if mode == OutputRate::Core { 1024 } else { 2048 }];
        if point == 3 {
            let count = c["tags"].as_array().unwrap().len() as f32;
            let common_core: Vec<f32> = pcm[i * 1024..(i + 1) * 1024]
                .iter()
                .map(|v| v / count)
                .collect();
            for tag in c["tags"].as_array().unwrap() {
                let key = tag.as_u64().unwrap().to_string();
                let core = if let Some(source) = c["source_pcm"].get(&key) {
                    core(source)[i * 1024..(i + 1) * 1024].to_vec()
                } else {
                    common_core.clone()
                };
                let state = sources.entry(tag.as_u64().unwrap()).or_default();
                let raw = if c["distinct_sources"] == true {
                    row["source_payloads"][&key].as_str()
                } else {
                    row["source_payload"].as_str()
                };
                let rendered = if let Some(raw) = raw {
                    let raw = hex(raw);
                    let mut bits = BitReader::new(&raw);
                    let crc = bits.read(4).unwrap() == 14;
                    let frame = state
                        .0
                        .read(&mut bits, raw.len() * 8, crc, 48000, 16, 1)
                        .unwrap();
                    state.1.process(&frame, &[&core], 48000, 16, mode).unwrap()
                } else {
                    state
                        .1
                        .process_upsampling(&[&core], 48000, 16, mode)
                        .unwrap()
                };
                for (j, (dest, &sample)) in coupled.iter_mut().zip(&rendered[0]).enumerate() {
                    let ranges = &c["source_gain_ranges"][i];
                    let ratio = if mode == OutputRate::Core { 1 } else { 2 };
                    let enabled = ranges.is_null() || ranges.as_array().unwrap().iter().any(|r| {
                        j / ratio >= r[0].as_u64().unwrap() as usize
                            && j / ratio < r[1].as_u64().unwrap() as usize
                    });
                    if enabled { *dest = (*dest as f32 + sample as f32) as f64; }
                }
            }
        }
        left.push_back(coupled);
        if let Some(frame) = rendered {
            append(frame, &mut left, &mut out);
        }
    }
    while let Some(frame) = stage.finish().unwrap() {
        append(frame, &mut left, &mut out);
    }
    assert!(left.is_empty());
    out
}
#[test]
fn configured_ssr_cce_core_controls_match_independent_nonzero_gain_and_window_pcm() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 20);
    for c in m["controls"].as_array().unwrap() {
        let mut pcm = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(c), &mut pcm).unwrap();
        let gold = core(c);
        assert_eq!(pcm.len(), gold.len() * 4);
        assert!(gold.iter().any(|v| v.abs() > 1e-4));
        for (i, (a, b)) in pcm.chunks_exact(4).zip(gold).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            assert!((a - b).abs() < 2e-7, "{} sample {i}: {a} vs {b}", c["name"]);
        }
    }
}
#[test]
fn composed_ps_stage_has_complete_nonzero_stereo_at_both_clocks() {
    for c in manifest()["cases"].as_array().unwrap() {
        let pcm = reference(c);
        assert_eq!(pcm.len(), c["samples"].as_u64().unwrap() as usize * 2);
        assert!(pcm.chunks_exact(2).any(|v| (v[0] - v[1]).abs() > 1e-6));
    }
}
#[test]
fn native_ssr_ps_cce_waveform_acceptance() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = NativePsAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut pcm = vec![];
        let mut indices = vec![];
        let packets = packets(c);
        for packet in &packets {
            let saved = d.checkpoint();
            let mut bad = packet.clone();
            bad.push(0);
            assert!(
                d.decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("trailing bytes")
            );
            let output = d.decode(packet).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(d.decode(packet).unwrap(), output);
            if let Some(frame) = output {
                indices.push(frame.frame_index);
                pcm.extend(frame.pcm);
            }
        }
        loop {
            let saved = d.checkpoint();
            let output = d.finish().unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(d.finish().unwrap(), output);
            let Some(frame) = output else {
                break;
            };
            indices.push(frame.frame_index);
            pcm.extend(frame.pcm);
        }
        assert_eq!(indices, (0..6).collect::<Vec<_>>());
        let gold = reference(c);
        assert_eq!(pcm.len(), gold.len());
        for (i, (a, b)) in pcm.iter().zip(gold).enumerate() {
            assert!((a - b).abs() < 2e-7, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        d.reset();
        let mut replay = vec![];
        for packet in &packets {
            if let Some(frame) = d.decode(packet).unwrap() {
                replay.extend(frame.pcm);
            }
        }
        while let Some(frame) = d.finish().unwrap() {
            replay.extend(frame.pcm);
        }
        assert_eq!(pcm, replay);
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(c), &mut root).unwrap();
        assert_eq!(
            root,
            pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn invalid_ssr_ps_cce_packets_reproduce_exact_errors_and_preserve_queued_pcm() {
    let m = manifest();
    assert_eq!(m["invalid"].as_array().unwrap().len(), 4);
    for case in m["invalid"].as_array().unwrap() {
        let asc = hex(case["asc"].as_str().unwrap());
        let encoded = packets(case);
        let mut d = NativePsAacDecoder::new(&asc).unwrap();
        assert!(d.decode(&encoded[0]).unwrap().is_none());
        let saved = d.checkpoint();
        assert!(
            d.decode(&encoded[1])
                .unwrap_err()
                .to_string()
                .contains(case["error"].as_str().unwrap())
        );
        let baseline = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["point"] == case["point"]
                    && c["active"] == case["active"]
                    && c["tags"] == case["tags"]
                    && c["bands"] == case["bands"]
                    && c["name"].as_str().unwrap().ends_with("source-sbr")
                        == (case["name"] == "crc")
            })
            .unwrap();
        let valid = packets(baseline);
        assert_eq!(valid[0], encoded[0]);
        let complete = |decoder: &mut NativePsAacDecoder| {
            let mut frames = vec![];
            for packet in &valid[1..] {
                if let Some(frame) = decoder.decode(packet).unwrap() {
                    frames.push(frame);
                }
            }
            while let Some(frame) = decoder.finish().unwrap() {
                frames.push(frame);
            }
            frames
        };
        let after = complete(&mut d);
        d.restore(&saved).unwrap();
        assert_eq!(complete(&mut d), after);
        assert_eq!(
            after.iter().map(|f| f.frame_index).collect::<Vec<_>>(),
            (0..6).collect::<Vec<_>>()
        );
        let mut bytes = vec![];
        let err = fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut bytes)
            .unwrap_err()
            .to_string();
        assert!(err.contains(case["error"].as_str().unwrap()), "{err}");
    }
}

#[test]
fn ssr_ps_cce_ranges_rewind_seek_and_doubly_delayed_eof_preserve_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn play(stream: &mut dyn AudioStream) -> Vec<u8> {
        let mut d = stream.make_decoder().unwrap();
        let mut pcm = vec![];
        while let Some(p) = stream.next_packet().unwrap() {
            if let Some(frame) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(output) = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(output.data);
                }
            }
        }
        while let Some(frame) = d.finish_packet().unwrap() {
            if let Some(output) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(output.data);
            }
        }
        pcm
    }
    for case in manifest()["cases"].as_array().unwrap() {
        let data = video(case);
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let rate = case["container_rate"].as_u64().unwrap();
        let interval = Some((Duration::from_millis(10), Duration::from_millis(200)));
        for _ in 0..2 {
            let mut range = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(&data, &mut range, interval).unwrap();
            assert_eq!(range, full[rate as usize / 100 * 8..rate as usize / 5 * 8]);
        }
        let mut stream =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut stream), full);
        stream.rewind();
        assert_eq!(play(&mut stream), full);
        let landed = stream.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut stream), full[landed as usize * 8..]);
    }
}

#[test]
fn distinct_source_oracle_detects_history_substitution_and_fil_reassociation() {
    let m = manifest();
    let cases: Vec<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["distinct_sources"] == true && c["source_gain_ranges"].is_null())
        .collect();
    assert_eq!(cases.len(), 12);
    for c in cases {
        let first = core(&c["source_pcm"]["1"]);
        let second = core(&c["source_pcm"]["15"]);
        let shape_error = first
            .iter()
            .zip(&second)
            .map(|(a, b)| (2. * a - b).abs())
            .fold(0., f32::max);
        assert!(
            shape_error > 1e-6,
            "source windows are indistinguishable: {}",
            c["name"]
        );
        let expected = reference(c);
        let mut wrong_history = c.clone();
        wrong_history["source_pcm"]["15"] = c["source_pcm"]["1"].clone();
        let substituted = reference(&wrong_history);
        let error: f32 = expected
            .iter()
            .zip(&substituted)
            .map(|(a, b)| (a - b).abs())
            .fold(0., f32::max);
        assert!(
            error > 1e-6,
            "history substitution is invisible: {}",
            c["name"]
        );
        if c["name"].as_str().unwrap().contains("asymmetric") {
            let mut wrong_fil = c.clone();
            for row in wrong_fil["frames"].as_array_mut().unwrap() {
                let payload = row["source_payloads"]["15"].take();
                row["source_payloads"] = serde_json::json!({"1": payload});
            }
            let reassociated = reference(&wrong_fil);
            let error: f32 = expected
                .iter()
                .zip(&reassociated)
                .map(|(a, b)| (a - b).abs())
                .fold(0., f32::max);
            assert!(
                error > 1e-6,
                "FIL reassociation is invisible: {}",
                c["name"]
            );
        }
    }
}

#[test]
fn absent_source_oracle_keeps_queued_gain_boundaries_and_detects_unmuted_tail() {
    let m = manifest();
    let cases: Vec<_> = m["cases"].as_array().unwrap().iter()
        .filter(|c| !c["source_gain_ranges"].is_null()).collect();
    assert_eq!(cases.len(), 28);
    for c in cases {
        assert_eq!(c["present"].as_array().unwrap().len(), 6);
        assert!(c["source_gain_ranges"].as_array().unwrap().iter()
            .any(|r| r.as_array().unwrap().is_empty()));
        if c["name"].as_str().unwrap().contains("-1-") {
            let expected = reference(c);
            let mut unmuted = c.clone();
            unmuted.as_object_mut().unwrap().remove("source_gain_ranges");
            let wrong = reference(&unmuted);
            let error = expected.iter().zip(wrong)
                .map(|(a,b)| (a-b).abs()).fold(0f32, f32::max);
            assert!(error > 1e-6, "absent gains are invisible: {}", c["name"]);
        }
    }
}

#[test]
fn pce_roster_change_preserves_pending_frames() {
    let m = manifest();
    let c = m["cases"].as_array().unwrap().iter()
        .find(|c| c["name"] == "pce-roster-ahead-0-48000").unwrap();
    let mut d = NativePsAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
    let p = packets(c);
    d.decode(&p[0]).unwrap();
    d.decode(&p[1]).unwrap();
    let saved = d.checkpoint();
    let result = d.decode(&p[2]).unwrap();
    d.restore(&saved).unwrap();
    assert_eq!(d.decode(&p[2]).unwrap(), result);
    let mut probe = fvid_media::owned_aac::aac_ps_native::InBandPsProbe::new(
        &hex(c["asc"].as_str().unwrap()), 48000).unwrap();
    for packet in &p { assert!(probe.read(packet).unwrap()); }
    probe.reset();
    for packet in &p { assert!(probe.read(packet).unwrap()); }
    // A checkpoint from a different initial roster remains incompatible.
    let foreign = m["cases"].as_array().unwrap().iter()
        .find(|v| v["tags"] == serde_json::json!([1,15]) && v["bands"] == 64).unwrap();
    let foreign = NativePsAacDecoder::new(&hex(foreign["asc"].as_str().unwrap())).unwrap();
    assert!(d.restore(&foreign.checkpoint()).unwrap_err().to_string()
        .contains("checkpoint configuration mismatch"));
}

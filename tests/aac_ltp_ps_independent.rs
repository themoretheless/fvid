use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
use serde_json::Value;
fn m() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ltp-ps-independent.json"
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
fn independent_oracle_detects_discarded_source_prediction_history() {
    for c in m()["cases"].as_array().unwrap() {
        let good = reference(c);
        let bad = reference_core(c, true);
        assert_eq!(good.len(), bad.len());
        let max = good
            .iter()
            .zip(&bad)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            max > 1e-6,
            "{} missing source history sensitivity: {max}",
            c["name"]
        );
    }
}

fn reference(c: &Value) -> Vec<f32> {
    reference_core(c, false)
}
fn reference_core(c: &Value, discarded: bool) -> Vec<f32> {
    reference_state(c, discarded, false)
}
fn reference_state(c: &Value, discarded: bool, reset_absent_dsp: bool) -> Vec<f32> {
    use fvid_media::owned_aac::{
        aac_sbr_dsp::{Dsp, OutputRate},
        aac_sbr_history::Stream,
        aac_sbr_ps::Decoder,
        bits::BitReader,
    };
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let n = c["n"].as_u64().unwrap() as usize;
    let slots = c["slots"].as_u64().unwrap() as u8;
    let samples = if c["bands"] == 32 { n } else { n * 2 };
    let target: Vec<f32> = bytes(c["target_core"].as_str().unwrap())
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let mut ps = Decoder::default();
    let mut sources = std::collections::BTreeMap::<u64, (Stream, Dsp)>::new();
    let mut coupled: Vec<Vec<f32>> = vec![];
    let mut output = vec![];
    fn append(
        frame: fvid_media::owned_aac::aac_sbr_ps::Frame,
        coupled: &[Vec<f32>],
        out: &mut Vec<f32>,
    ) {
        let left = &coupled[frame.frame_index as usize];
        for i in 0..frame.pcm[0].len() {
            out.push(frame.pcm[0][i] as f32 + left[i]);
            out.push(frame.pcm[1][i] as f32);
        }
    }
    for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let crc = bits.read(4).unwrap() == 14;
        let frame = ps
            .read(
                &mut bits,
                raw.len() * 8,
                crc,
                &target[index * n..(index + 1) * n],
                48000,
                slots,
                mode,
            )
            .unwrap();
        if reset_absent_dsp {
            for (tag, state) in sources.iter_mut() {
                if !row["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|source| source["tag"] == *tag)
                {
                    state.1 = Dsp::default();
                }
            }
        }
        let mut left = vec![0f32; samples];
        for source in row["sources"].as_array().unwrap() {
            let name = source[if discarded { "discarded" } else { "reference" }]
                .as_str()
                .unwrap();
            let data = bytes(name);
            let core: Vec<f32> = data[source["core_index"].as_u64().unwrap() as usize * (n * 4)
                ..(source["core_index"].as_u64().unwrap() as usize + 1) * (n * 4)]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let state = sources.entry(source["tag"].as_u64().unwrap()).or_default();
            let raw = hex(source["payload"].as_str().unwrap());
            let rendered = if raw.is_empty() {
                state
                    .1
                    .process_upsampling(&[&core], 48000, slots, mode)
                    .unwrap()
            } else {
                let mut bits = BitReader::new(&raw);
                let crc = bits.read(4).unwrap() == 14;
                let syntax = state
                    .0
                    .read(&mut bits, raw.len() * 8, crc, 48000, slots, 1)
                    .unwrap();
                state
                    .1
                    .process(&syntax, &[&core], 48000, slots, mode)
                    .unwrap()
            };
            for (a, b) in left.iter_mut().zip(&rendered[0]) {
                *a += *b as f32;
            }
        }
        coupled.push(left);
        if let Some(f) = frame {
            append(f, &coupled, &mut output);
        }
    }
    while let Some(f) = ps.finish().unwrap() {
        append(f, &coupled, &mut output);
    }
    output
}

#[test]
fn ltp_ps_independent_source_prediction_sbr_and_delay_acceptance() {
    let blob = bytes("aac-ltp-ps-independent-packets.bin");
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
            assert!(d
                .decode(&bad)
                .unwrap_err()
                .to_string()
                .contains("trailing bytes"));
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
        assert_eq!(indices, (0..12).collect::<Vec<_>>());
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
fn ltp_ps_independent_probe_ranges_rewind_seek_keep_source_state() {
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
    let blob = bytes("aac-ltp-ps-independent-packets.bin");
    for c in m()["cases"].as_array().unwrap() {
        let rate = c["container_rate"].as_u64().unwrap() as usize;
        let mut probe = InBandPsProbe::new(&hex(c["asc"].as_str().unwrap()), rate as u32).unwrap();
        for replay in 0..2 {
            if replay != 0 {
                probe.reset();
            }
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                assert!(probe
                    .read(&blob[at..at + row["bytes"].as_u64().unwrap() as usize])
                    .unwrap());
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
fn source_sbr_fil_reassociation_changes_independent_ps_pcm() {
    for c in m()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["source_sbr"] == true)
    {
        let mut swapped = c.clone();
        for row in swapped["frames"].as_array_mut().unwrap() {
            let sources = row["sources"].as_array_mut().unwrap();
            if sources.len() != 2 {
                continue;
            }
            let first = sources[0]["payload"].clone();
            sources[0]["payload"] = sources[1]["payload"].clone();
            sources[1]["payload"] = first;
        }
        let good = reference(c);
        let wrong = reference(&swapped);
        assert_eq!(good.len(), wrong.len());
        let delta = good
            .iter()
            .zip(&wrong)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            delta > 1e-6,
            "{} source FIL reassociation is not observable: {delta}",
            c["name"]
        );
    }
}

#[test]
fn dynamic_rosters_preserve_point3_source_histories_and_dsp() {
    let manifest = m();
    let cases = manifest["cases"].as_array().unwrap();
    for c in cases.iter().filter(|c| c["dynamic"] == true) {
        let stable = cases
            .iter()
            .find(|s| {
                s["n"] == c["n"]
                    && s["dynamic"] == false
                    && s["schedule"] == c["schedule"]
                    && s["source_sbr"] == c["source_sbr"]
                    && s["container_rate"] == c["container_rate"]
            })
            .unwrap();
        let mut dynamic_pcm = vec![];
        let mut static_pcm = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut dynamic_pcm,
        )
        .unwrap();
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(stable["video"]["file"].as_str().unwrap()),
            &mut static_pcm,
        )
        .unwrap();
        assert_eq!(
            dynamic_pcm, static_pcm,
            "{} PCE roster changed source state",
            c["name"]
        );
        if c["schedule"].as_str().unwrap().ends_with("return") {
            let good = reference(c);
            let wrong = reference_state(c, false, true);
            assert_eq!(good.len(), wrong.len());
            let delta = good
                .iter()
                .zip(&wrong)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(
                delta > 1e-6,
                "{} discarded DSP history is not observable: {delta}",
                c["name"]
            );
        }
    }
}

#[test]
fn empty_rosters_keep_target_ps_clock_and_uncoupled_channel() {
    for c in m()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["schedule"].as_str().unwrap().starts_with("both-"))
    {
        let mut target = c.clone();
        for row in target["frames"].as_array_mut().unwrap() {
            row["sources"] = serde_json::json!([]);
        }
        let target_pcm = reference(&target);
        let mut raw = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut raw,
        )
        .unwrap();
        let actual: Vec<f32> = raw
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(actual.len(), target_pcm.len());
        let frame_samples = c["container_frame_samples"].as_u64().unwrap() as usize;
        for (frame, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = frame * frame_samples * 2;
            for i in 0..frame_samples {
                assert_eq!(
                    actual[at + i * 2 + 1],
                    target_pcm[at + i * 2 + 1],
                    "{} source changed right target channel",
                    c["name"]
                );
                if row["sources"].as_array().unwrap().is_empty() {
                    assert_eq!(
                        actual[at + i * 2],
                        target_pcm[at + i * 2],
                        "{} absent source leaked PCM at frame {frame}",
                        c["name"]
                    );
                }
            }
        }
    }
}

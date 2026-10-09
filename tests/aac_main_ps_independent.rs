use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
use serde_json::Value;
fn m() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-main-ps-independent.json"
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
    let samples = if c["bands"] == 32 { 1024 } else { 2048 };
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
    for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let crc = bits.read(4).unwrap() == 14;
        let frame = ps
            .read(&mut bits, raw.len() * 8, crc, &[0.; 1024], 48000, 16, mode)
            .unwrap();
        let mut left = vec![0f32; samples];
        for source in row["sources"].as_array().unwrap() {
            let name = source[if discarded { "discarded" } else { "reference" }]
                .as_str()
                .unwrap();
            let data = bytes(name);
            let core: Vec<f32> = data[i * 4096..(i + 1) * 4096]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let state = sources.entry(source["tag"].as_u64().unwrap()).or_default();
            let raw = hex(source["payload"].as_str().unwrap());
            let rendered = if raw.is_empty() {
                state
                    .1
                    .process_upsampling(&[&core], 48000, 16, mode)
                    .unwrap()
            } else {
                let mut bits = BitReader::new(&raw);
                let crc = bits.read(4).unwrap() == 14;
                let syntax = state
                    .0
                    .read(&mut bits, raw.len() * 8, crc, 48000, 16, 1)
                    .unwrap();
                state.1.process(&syntax, &[&core], 48000, 16, mode).unwrap()
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
fn main_ps_independent_source_prediction_sbr_and_delay_acceptance() {
    let blob = bytes("aac-main-ps-independent-packets.bin");
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
fn main_ps_independent_probe_ranges_rewind_seek_keep_source_state() {
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
    let blob = bytes("aac-main-ps-independent-packets.bin");
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

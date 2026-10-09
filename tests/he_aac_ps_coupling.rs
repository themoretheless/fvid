use fvid_media::owned_aac::{
    aac_ps_native::{InBandPsProbe, NativePsAacDecoder},
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_history::Stream,
    aac_sbr_ps,
    bits::BitReader,
};
use serde_json::Value;
const RAW: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-ps-coupling-packets.bin");
const CORE: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-ps-coupling-core.f32le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-ps-coupling-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    let n = row["bytes"].as_u64().unwrap() as usize;
    &RAW[a..a + n]
}
fn decoder(c: &Value) -> NativePsAacDecoder {
    let asc = hex(c["asc"].as_str().unwrap());
    if c["kind"] == "PS" {
        NativePsAacDecoder::new(&asc).unwrap()
    } else {
        NativePsAacDecoder::new_with_in_band_ps(&asc, c["output_rate"].as_u64().unwrap() as u32)
            .unwrap()
    }
}
fn reference(c: &Value) -> Vec<f32> {
    let slots = c["slots"].as_u64().unwrap() as u8;
    let n = slots as usize * 64;
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let mut ps = aac_sbr_ps::Decoder::default();
    let mut sources = std::collections::BTreeMap::<u64, (Stream, Dsp)>::new();
    let mut coupled = vec![];
    let mut output = vec![];
    let off = c["core_pcm"][0].as_u64().unwrap() as usize;
    for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let core: Vec<f32> = CORE[off + index * n * 4..off + (index + 1) * n * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let target: Vec<f32> = if c["point"] == 3 {
            vec![0.; n]
        } else {
            core.iter()
                .map(|v| v * c["tags"].as_array().unwrap().len() as f32)
                .collect()
        };
        let raw = hex(row["sbr"].as_str().unwrap());
        let frame = if raw.is_empty() {
            ps.process_upsampling(&target, 48000, slots, mode).unwrap()
        } else {
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            ps.read(
                &mut bits,
                raw.len() * 8,
                kind == 14,
                &target,
                48000,
                slots,
                mode,
            )
            .unwrap()
        };
        let mut left = vec![0f32; slots as usize * c["bands"].as_u64().unwrap() as usize * 2];
        if c["point"] == 3 {
            for source in row["cce"].as_array().unwrap() {
                let state = sources.entry(source["tag"].as_u64().unwrap()).or_default();
                let raw = hex(source["sbr"].as_str().unwrap());
                let rendered = if raw.is_empty() {
                    state
                        .1
                        .process_upsampling(&[&core], 48000, slots, mode)
                        .unwrap()
                } else {
                    let mut bits = BitReader::new(&raw);
                    let kind = bits.read(4).unwrap();
                    let f = state
                        .0
                        .read(&mut bits, raw.len() * 8, kind == 14, 48000, slots, 1)
                        .unwrap();
                    state.1.process(&f, &[&core], 48000, slots, mode).unwrap()
                };
                for (a, &v) in left.iter_mut().zip(&rendered[0]) {
                    *a += v as f32;
                }
            }
        }
        coupled.push(left);
        if let Some(frame) = frame {
            for i in 0..frame.pcm[0].len() {
                output.push(frame.pcm[0][i] as f32 + coupled[frame.frame_index as usize][i]);
                output.push(frame.pcm[1][i] as f32);
            }
        }
    }
    if let Some(frame) = ps.finish().unwrap() {
        for i in 0..frame.pcm[0].len() {
            output.push(frame.pcm[0][i] as f32 + coupled[frame.frame_index as usize][i]);
            output.push(frame.pcm[1][i] as f32);
        }
    }
    output
}
#[test]
fn ps_coupling_matches_independent_core_stage_composition_and_packet_delay() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = decoder(c);
        let mut probe = InBandPsProbe::new(
            &hex(c["asc"].as_str().unwrap()),
            c["output_rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut actual = vec![];
        let mut indices = vec![];
        for row in c["frames"].as_array().unwrap() {
            assert!(probe.read(packet(row)).unwrap());
            let saved = d.checkpoint();
            let frame = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(frame, d.decode(packet(row)).unwrap());
            if let Some(frame) = frame {
                indices.push(frame.frame_index);
                actual.extend(frame.pcm);
            }
        }
        let frame = d.finish().unwrap().unwrap();
        indices.push(frame.frame_index);
        actual.extend(frame.pcm);
        assert!(d.finish().unwrap().is_none());
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5]);
        let expected = reference(c);
        assert_eq!(actual.len(), expected.len());
        assert!(expected.iter().any(|v| v.abs() > 1e-5));
        for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (a - e).abs() < 2e-7,
                "{} {} {} sample {i}: {a} != {e}",
                c["slots"],
                c["point"],
                c["tags"]
            );
        }
        d.reset();
        let mut replay = vec![];
        for row in c["frames"].as_array().unwrap() {
            if let Some(frame) = d.decode(packet(row)).unwrap() {
                replay.extend(frame.pcm);
            }
        }
        replay.extend(d.finish().unwrap().unwrap().pcm);
        assert_eq!(actual, replay);
    }
}

#[test]
fn ps_cce_failures_leave_pending_pcm_overlap_and_probe_histories_unchanged() {
    use fvid::container::mp4::Mp4Reader;
    for c in manifest()["invalid"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let mut reader =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        for (n, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let mut raw = vec![];
            reader.read_packet(track, n, &mut raw).unwrap();
            assert_eq!(raw, packet(r));
        }
        let mut d = decoder(c);
        let mut p = InBandPsProbe::new(&hex(c["asc"].as_str().unwrap()), 48000).unwrap();
        d.decode(packet(&c["frames"][0])).unwrap();
        p.read(packet(&c["frames"][0])).unwrap();
        let saved = d.checkpoint();
        let probe = p.clone();
        assert!(
            d.decode(packet(&c["frames"][1]))
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        assert!(
            p.read(packet(&c["frames"][1]))
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        let after = d.decode(packet(&c["frames"][2])).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(after, d.decode(packet(&c["frames"][2])).unwrap());
        let after = p.read(packet(&c["frames"][2])).unwrap();
        let mut restored = probe;
        assert_eq!(after, restored.read(packet(&c["frames"][2])).unwrap());
    }
}

#[cfg(feature = "player")]
#[test]
fn ps_cce_videos_accept_root_owned_export_ranges_wav_and_delayed_playback_seek() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    fn play(stream: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut d = stream.make_decoder().unwrap();
        let mut out = vec![];
        while let Some(p) = stream.next_packet().unwrap() {
            if let Some(frame) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(pcm) = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    out.extend(pcm.data);
                }
            }
        }
        if let Some(frame) = d.finish_packet().unwrap() {
            if let Some(pcm) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.extend(pcm.data);
            }
        }
        assert!(d.finish_packet().unwrap().is_none());
        out
    }
    for c in manifest()["cases"].as_array().unwrap() {
        if c["video"].is_null() {
            continue;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let mut owned = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, 2, c["samples"].as_u64().unwrap())
        );
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut root).unwrap();
        assert_eq!(owned, root);
        let expected = reference(c);
        assert_eq!(owned.len(), expected.len() * 4);
        for (a, e) in owned.chunks_exact(4).zip(expected) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap()) - e).abs() < 2e-7,
                "{}",
                path.display()
            );
        }
        let interval = Some((
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(100),
        ));
        for _ in 0..2 {
            let mut range = vec![];
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut range,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(range, owned[480 * 8..4800 * 8]);
        }
        let mut stream =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
        assert_eq!(play(&mut stream), owned);
        stream.rewind();
        assert_eq!(play(&mut stream), owned);
        let landed = stream.seek_to(4800);
        assert_eq!(play(&mut stream), owned[landed as usize * 8..]);
        let dest = std::env::temp_dir().join(format!(
            "fvid-ps-cce-{}-{}.wav",
            std::process::id(),
            path.file_name().unwrap().to_str().unwrap()
        ));
        fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&dest).unwrap(), None)
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, 2, stats.sample_frames)
        );
        std::fs::remove_file(dest).unwrap();
    }
}

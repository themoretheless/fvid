use fvid_media::owned_aac::{
    aac_native::NativeAacDecoder,
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_history::Stream,
    bits::BitReader,
    config::AudioSpecificConfig,
};
use serde_json::Value;
const RAW: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-independent-sbr-packets.bin");
const CORE: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-independent-sbr-core.f32le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-independent-sbr-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let at = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    &RAW[at..at + len]
}
fn render(
    dsp: &mut Dsp,
    syntax: &mut Stream,
    raw: &str,
    refs: &[&[f32]],
    slots: u8,
    mode: OutputRate,
) -> Vec<Vec<f64>> {
    let raw = hex(raw);
    if raw.is_empty() {
        return dsp.process_upsampling(refs, 48000, slots, mode).unwrap();
    }
    let mut bits = BitReader::new(&raw);
    let kind = bits.read(4).unwrap();
    let frame = syntax
        .read(
            &mut bits,
            raw.len() * 8,
            kind == 14,
            48000,
            slots,
            refs.len(),
        )
        .unwrap();
    dsp.process(&frame, refs, 48000, slots, mode).unwrap()
}
fn reference(c: &Value) -> Vec<f32> {
    let channels = c["channels"].as_u64().unwrap() as usize;
    let slots = c["slots"].as_u64().unwrap() as u8;
    let n = slots as usize * 64;
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let mut target = (Dsp::default(), Stream::default());
    let mut states = std::collections::BTreeMap::<u64, (Dsp, Stream)>::new();
    let mut result = vec![];
    let zeros = vec![0f32; n];
    let refs = vec![zeros.as_slice(); channels];
    for (f, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let rendered = render(
            &mut target.0,
            &mut target.1,
            row["sbr"].as_str().unwrap(),
            &refs,
            slots,
            mode,
        );
        let mut out: Vec<f32> = (0..rendered[0].len())
            .flat_map(|i| rendered.iter().map(move |v| v[i] as f32))
            .collect();
        for source in row["cce"].as_array().unwrap() {
            let off = source["core_pcm"][0].as_u64().unwrap() as usize;
            let at = off + f * n * 4;
            let core: Vec<f32> = CORE[at..at + n * 4]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let state = states.entry(source["tag"].as_u64().unwrap()).or_default();
            let cce = render(
                &mut state.0,
                &mut state.1,
                source["sbr"].as_str().unwrap(),
                &[&core],
                slots,
                mode,
            );
            for (i, &v) in cce[0].iter().enumerate() {
                out[i * channels] += v as f32;
                if channels == 2 {
                    out[i * channels + 1] +=
                        v as f32 * source["right_gain"].as_f64().unwrap() as f32;
                }
            }
        }
        result.extend(out);
    }
    result
}
#[test]
fn independent_cce_sbr_uses_tag_histories_and_mixes_after_target_sbr() {
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(
            config.program.as_ref().unwrap().coupling,
            c["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tag| (true, tag.as_u64().unwrap() as u8))
                .collect::<Vec<_>>()
        );
        let mut d =
            NativeAacDecoder::new_with_output_rate(&asc, c["output_rate"].as_u64().unwrap() as u32)
                .unwrap();
        let mut actual = vec![];
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            let output = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            actual.extend(output);
        }
        let retained = d.retained_payload_bytes().unwrap();
        let checkpoint = d.checkpoint();
        assert!(
            d.retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                .unwrap()
                > retained
        );
        let expected = reference(c);
        assert_eq!(actual.len(), expected.len());
        let cancels = c["channels"] == 1
            && c["tags"].as_array().unwrap().len() == 2
            && c["frames"].as_array().unwrap().iter().all(|r| {
                r["sbr"].as_str().unwrap().is_empty()
                    && r["cce"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|c| c["sbr"].as_str().unwrap().is_empty())
            });
        if cancels {
            // Two independently nonzero, opposite CCEs cancel only after mixing.
            assert!(expected.iter().all(|v| v.abs() < 1e-7));
        } else {
            assert!(expected.iter().any(|v| v.abs() > 1e-5));
        }
        for source in c["frames"][0]["cce"].as_array().unwrap() {
            let off = source["core_pcm"][0].as_u64().unwrap() as usize;
            let len = source["core_pcm"][1].as_u64().unwrap() as usize;
            assert!(
                CORE[off..off + len * 4]
                    .chunks_exact(4)
                    .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-5)
            );
        }
        for (n, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert!((a - e).abs() < 2e-7, "case {c} sample {n}: {a} != {e}");
        }
        if c["kind"] == "implicit"
            && (!c["frames"][0]["sbr"].as_str().unwrap().is_empty()
                || c["frames"][0]["cce"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| !r["sbr"].as_str().unwrap().is_empty()))
        {
            let mut detector = NativeAacDecoder::new_with_sbr_detection(&asc).unwrap();
            assert_eq!(detector.sample_rate(), 24000);
            let detected: Vec<f32> = c["frames"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| detector.decode(packet(r)).unwrap())
                .collect();
            assert_eq!(detector.sample_rate(), 48000);
            assert_eq!(detected, actual);
            detector.reset();
            assert_eq!(detector.sample_rate(), 24000);
        }
        d.reset();
        let replay: Vec<f32> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| d.decode(packet(r)).unwrap())
            .collect();
        assert_eq!(actual, replay);
    }
}

#[test]
fn independent_sbr_videos_accept_owned_root_pcm_wav_ranges_and_playback() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    use std::io::Cursor;
    let m = manifest();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in m["cases"].as_array().unwrap() {
        if c["video"].is_null() {
            continue;
        }
        let path = root.join(c["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let mut pcm = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, channels as u16, c["samples"].as_u64().unwrap())
        );
        let reader = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let mut rendered = vec![];
        fvid::native_media::decode_mp4_aac_reader(reader, &mut rendered, None).unwrap();
        assert_eq!(pcm, rendered);
        let interval = Some((
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(100),
        ));
        let mut range = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut range,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(range, &pcm[480 * channels * 4..4800 * channels * 4]);
        let mut repeated = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut repeated,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(repeated, range);
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(&bytes),
                Limits::default(),
            )
            .unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut played = vec![];
            assert_eq!(
                (stream.sample_rate(), stream.channels()),
                (48000, channels as u16)
            );
            while let Some(p) = stream.next_packet().unwrap() {
                let duration = u64::try_from(p.duration).unwrap();
                let frame = decoder
                    .decode_packet(&p.data, p.pts, duration)
                    .unwrap()
                    .unwrap();
                assert_eq!((frame.source_pts, frame.source_duration), (p.pts, duration));
                played.extend(frame.packet.data);
            }
            assert!(decoder.finish_packet().unwrap().is_none());
            assert_eq!(played, pcm);
            stream.rewind();
            let mut decoder = stream.make_decoder().unwrap();
            let mut rewind = vec![];
            while let Some(p) = stream.next_packet().unwrap() {
                rewind.extend(
                    decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap()
                        .packet
                        .data,
                );
            }
            assert_eq!(rewind, pcm);
            let landed = stream.seek_to(4800);
            let mut decoder = stream.make_decoder().unwrap();
            let mut seek = vec![];
            while let Some(p) = stream.next_packet().unwrap() {
                let frame = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(presented) = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    seek.extend(presented.data);
                }
            }
            assert_eq!(seek, pcm[landed as usize * channels * 4..]);
        }
        let expected = reference(c);
        assert_eq!(pcm.len(), expected.len() * 4);
        for (b, e) in pcm.chunks_exact(4).zip(expected) {
            let a = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - e).abs() < 2e-7, "{}: {a} != {e}", path.display());
        }
        let dest = std::env::temp_dir().join(format!(
            "fvid-dependent-sbr-{}-{}.wav",
            std::process::id(),
            path.file_name().unwrap().to_str().unwrap()
        ));
        fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&dest).unwrap(), None)
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, channels as u16, stats.sample_frames)
        );
        std::fs::remove_file(dest).unwrap();
    }
}

#[test]
fn invalid_cce_videos_reject_exact_failure_and_restore_all_sbr_and_overlap_histories() {
    use fvid::container::mp4::Mp4Reader;
    let m = manifest();
    for c in m["invalid"].as_array().unwrap() {
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
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for (n, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let mut raw = vec![];
            reader.read_packet(track, n, &mut raw).unwrap();
            assert_eq!(raw, packet(row));
        }
        d.decode(packet(&c["frames"][0])).unwrap();
        let saved = d.checkpoint();
        assert!(
            d.decode(packet(&c["frames"][1]))
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        let after_failure = d.decode(packet(&c["frames"][2])).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(after_failure, d.decode(packet(&c["frames"][2])).unwrap());
    }
}

#[test]
fn independent_cce_sbr_admission_charges_each_configured_state_before_output() {
    use std::io::Cursor;
    let m = manifest();
    for count in [1, 2] {
        let case = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["slots"] == 16
                    && c["channels"] == 2
                    && c["kind"] == "explicit"
                    && c["bands"] == 64
                    && c["tags"].as_array().unwrap().len() == count
            })
            .unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(path).unwrap();
        // MP4 export admits decoder, checkpoint and restore scratch (three copies).
        // Forty MiB admits one independent CCE but must reject two.
        let options = fvid_control::CopyOptions {
            max_controlled_bytes: Some(40 * 1024 * 1024),
            ..Default::default()
        };
        let mut out = vec![];
        let result = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut out,
            None,
            &options,
        );
        if count == 1 {
            assert!(result.is_ok(), "{result:?}");
            assert!(!out.is_empty());
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("controlled memory budget")
            );
            assert!(out.is_empty());
        }
        let options = fvid_control::CopyOptions {
            max_controlled_bytes: Some(64 * 1024 * 1024),
            ..Default::default()
        };
        let mut accepted = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut accepted,
            None,
            &options,
        )
        .unwrap();
        assert_eq!(
            accepted.len(),
            case["samples"].as_u64().unwrap() as usize * 8
        );
    }
}

#[test]
fn implicit_independent_cce_without_any_fil_keeps_the_exact_lc_core_clock_and_pcm() {
    let m = manifest();
    let mut qualified = 0;
    for c in m["cases"].as_array().unwrap() {
        if c["kind"] != "implicit"
            || !c["frames"].as_array().unwrap().iter().all(|r| {
                r["sbr"].as_str().unwrap().is_empty()
                    && r["cce"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|v| v["sbr"].as_str().unwrap().is_empty())
            })
        {
            continue;
        }
        let asc = hex(c["asc"].as_str().unwrap());
        let mut plain = NativeAacDecoder::new(&asc).unwrap();
        let mut candidate = NativeAacDecoder::new_with_sbr_detection(&asc).unwrap();
        let n = c["slots"].as_u64().unwrap() as usize * 64;
        let channels = c["channels"].as_u64().unwrap() as usize;
        for (frame, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let actual = candidate.decode(packet(row)).unwrap();
            assert_eq!(candidate.sample_rate(), 24000);
            assert_eq!(actual, plain.decode(packet(row)).unwrap());
            let mut expected = vec![0f32; n * channels];
            for source in row["cce"].as_array().unwrap() {
                let at = source["core_pcm"][0].as_u64().unwrap() as usize + frame * n * 4;
                for (i, b) in CORE[at..at + n * 4].chunks_exact(4).enumerate() {
                    let v = f32::from_le_bytes(b.try_into().unwrap());
                    expected[i * channels] += v;
                    if channels == 2 {
                        expected[i * channels + 1] +=
                            v * source["right_gain"].as_f64().unwrap() as f32;
                    }
                }
            }
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| (a - e).abs() < 2e-7)
            );
        }
        candidate.reset();
        assert_eq!(candidate.sample_rate(), 24000);
        qualified += 1;
    }
    assert_eq!(qualified, 12);
}

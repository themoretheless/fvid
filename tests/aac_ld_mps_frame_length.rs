use fvid_media::owned_aac::{NativeAacDecoder, aac_ld_mps_config::Config};
use serde_json::Value;
use std::{collections::HashMap, io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-frame-length")
}
fn m() -> Value {
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
fn read(v: &Value) -> Vec<u8> {
    std::fs::read(root().join(v.as_str().unwrap())).unwrap()
}
fn movie(v: &Value) -> Vec<u8> {
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&read(v), &mut pcm)
        .unwrap_or_else(|e| panic!("{v}: {e}"));
    pcm
}
fn controls(manifest: &Value) -> HashMap<String, Vec<u8>> {
    manifest["fixed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["video"].as_str().unwrap().to_owned(), movie(&c["video"])))
        .collect()
}
#[test]
fn bare_and_fixed_partition_controls_are_playable_without_frame_length_changes() {
    let manifest = m();
    assert_eq!(manifest["controls"].as_array().unwrap().len(), 30);
    assert_eq!(manifest["fixed"].as_array().unwrap().len(), 112);
    for c in manifest["controls"].as_array().unwrap() {
        let actual = movie(&c["video"]);
        let expected = read(&c["golden"]);
        assert_eq!(actual.len(), expected.len());
        let error = actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{}: {error}", c["name"]);
    }
    for c in manifest["fixed"].as_array().unwrap() {
        let actual = movie(&c["video"]);
        assert_eq!(actual.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(actual.iter().any(|&v| v != 0));
    }
}
#[test]
fn frame_length_transitions_keep_the_fixed_partition_waveform_and_container_windows() {
    let manifest = m();
    let fixed = controls(&manifest);
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 336);
    let mut values = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let expected = &fixed[c["control"].as_str().unwrap()];
        let actual = movie(&c["video"]);
        assert_eq!(&actual, expected, "{}", c["name"]);
        let mut mkv = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(&c["file"])),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(&mkv, expected, "{} MKV", c["name"]);
        let mut decoder =
            NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), n(&c["rate"]) as u32).unwrap();
        let blob = std::fs::read(root().join("packets.bin")).unwrap();
        let mut untimed = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let at = n(&row["offset"]);
            untimed.extend(decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap());
        }
        while let Some(frame) = decoder.finish().unwrap() {
            untimed.extend(frame.samples);
        }
        assert_eq!(
            untimed
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            *expected,
            "{} untimed",
            c["name"]
        );
        values += actual.len() / 4;
    }
    eprintln!(
        "Frame-length transitions: {values} PCM values exactly equal to fixed-partition controls"
    );
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
fn variable_spatial_frames_play_seek_and_rewind_with_original_source_windows() {
    use fvid::audio::AudioStream;
    for c in m()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(&c["video"]), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(&c["file"])),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(&c["video"])),
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

#[test]
fn opaque_packet_windows_checkpoints_and_reset_survive_length_transitions() {
    let manifest = m();
    let expected = controls(&manifest);
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let rate = n(&c["rate"]) as u32;
        let mut decoder = NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), rate).unwrap();
        decoder.set_source_packet_ids(true).unwrap();
        let mut collected = Vec::new();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&row["offset"]);
            let packet = &blob[at..at + n(&row["bytes"])];
            let before = decoder.checkpoint();
            let want = decoder
                .decode_timed_in_clock(packet, i as i64 * 97 - 11, n(&c["out_n"]) as u64, rate)
                .unwrap();
            decoder.restore(&before).unwrap();
            let got = decoder
                .decode_timed_in_clock(packet, i as i64 * 97 - 11, n(&c["out_n"]) as u64, rate)
                .unwrap();
            assert_eq!(got, want, "{} checkpoint{i}", c["name"]);
            if let Some(frame) = got {
                collected.push(frame);
            }
        }
        while let Some(frame) = decoder.finish().unwrap() {
            collected.push(frame);
        }
        assert_eq!(collected.len(), 12, "{}", c["name"]);
        for (i, frame) in collected.iter().enumerate() {
            assert_eq!(frame.pts, i as i64 * 97 - 11);
            assert_eq!(frame.duration, n(&c["out_n"]) as u64);
            assert_eq!(frame.samples.len(), n(&c["out_n"]) * n(&c["channels"]));
        }
        let actual = collected
            .iter()
            .flat_map(|f| f.samples.iter().flat_map(|v| v.to_le_bytes()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected[c["control"].as_str().unwrap()]);
        decoder.reset();
        let mut reset = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let at = n(&row["offset"]);
            reset.extend(decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap());
        }
        while let Some(frame) = decoder.finish().unwrap() {
            reset.extend(frame.samples);
        }
        assert_eq!(
            reset
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            actual,
            "{} reset restores initial frame length",
            c["name"]
        );
    }
}
#[test]
fn late_bad_frame_rolls_back_every_queued_length_change_and_accepts_repaired_suffix() {
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 4);
    for bad in manifest["errors"].as_array().unwrap() {
        let mut out = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&read(&bad["video"]), &mut out)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{}: {error}",
            bad["name"]
        );
        let good = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["video"] == bad["good_video"])
            .unwrap();
        let rate = n(&good["rate"]) as u32;
        let mut decoder = NativeAacDecoder::new_with_output_rate(&hex(&good["asc"]), rate).unwrap();
        let mut reference =
            NativeAacDecoder::new_with_output_rate(&hex(&good["asc"]), rate).unwrap();
        for (i, (a, b)) in good["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(bad["frames"].as_array().unwrap())
            .enumerate()
        {
            let wire = |r: &Value| {
                let at = n(&r["offset"]);
                &blob[at..at + n(&r["bytes"])]
            };
            let pts = (i * n(&good["out_n"])) as i64;
            let duration = n(&good["out_n"]) as u64;
            if i == 11 {
                let before = decoder.checkpoint();
                let error = decoder
                    .decode_timed_in_clock(wire(b), pts, duration, rate)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                let actual = decoder
                    .decode_timed_in_clock(wire(a), pts, duration, rate)
                    .unwrap();
                decoder.restore(&before).unwrap();
                assert_eq!(
                    decoder
                        .decode_timed_in_clock(wire(a), pts, duration, rate)
                        .unwrap(),
                    actual
                );
                decoder.restore(&before).unwrap();
            }
            assert_eq!(
                decoder
                    .decode_timed_in_clock(wire(a), pts, duration, rate)
                    .unwrap(),
                reference
                    .decode_timed_in_clock(wire(a), pts, duration, rate)
                    .unwrap()
            );
        }
        loop {
            let a = decoder.finish().unwrap();
            let b = reference.finish().unwrap();
            assert_eq!(a, b);
            if a.is_none() {
                break;
            }
        }
    }
}
#[test]
fn resetting_banks_at_each_frame_length_header_is_a_detectable_wrong_waveform() {
    use fvid_media::owned_aac::{aac_ld_mps_decor::Mode, aac_ld_mps_pcm, bits::BitReader};
    let manifest = m();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "aac-ld-mps-downscaled-aot23-n480-ds2-c1-tree7-batch")
        .unwrap();
    let expected = movie(&c["control"]);
    let gold = read(&c["golden"]);
    let mut wrong = Vec::new();
    for frame in c["schedule"].as_array().unwrap() {
        let config = Config::parse(&hex(&frame["ssc"])).unwrap();
        let mut fresh = aac_ld_mps_pcm::Decoder::new(&config, Mode::Complex).unwrap();
        let start = n(&frame["start"]) * n(&c["out_n"]) * 4;
        let end = n(&frame["end"]) * n(&c["out_n"]) * 4;
        let pcm = vec![
            gold[start..end]
                .chunks_exact(4)
                .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
                .collect(),
        ];
        let rendered = fresh
            .decode(&mut BitReader::new(&hex(&frame["body"])), &pcm, &[None])
            .unwrap();
        wrong.extend(
            rendered
                .samples
                .into_iter()
                .flat_map(|v| (v as f32).to_le_bytes()),
        );
    }
    assert_eq!(wrong.len(), expected.len());
    let gap = wrong
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .map(|(a, b)| {
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
        })
        .fold(0f32, f32::max);
    assert!(
        gap > 1e-6,
        "reset negative control is not observable: {gap}"
    );
    eprintln!("Wrong full-reset at frame-length header PCM gap: {gap}");
}
#[test]
fn reset_after_a_long_frame_restores_initial_length_before_a_headerless_frame() {
    let manifest = m();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "aac-ld-mps-downscaled-aot23-n512-ds2-c1-tree7-early")
        .unwrap();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let rate = n(&c["rate"]) as u32;
    let mut decoder = NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), rate).unwrap();
    let mut fresh = NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), rate).unwrap();
    for row in c["frames"]
        .as_array()
        .unwrap()
        .iter()
        .take(n(&c["schedule"][1]["end"]))
    {
        let at = n(&row["offset"]);
        decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap();
    }
    decoder.reset();
    let first = &c["frames"][0];
    let at = n(&first["offset"]);
    let packet = &blob[at..at + n(&first["bytes"])];
    let expected = fresh.decode(packet).unwrap();
    assert!(!expected.is_empty());
    assert_eq!(decoder.decode(packet).unwrap(), expected);
}
#[test]
fn session_bad_batched_suffix_retains_the_exact_queue_and_dsp_state() {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
        aac_ld_mps_transport::{Extension, Kind, Message},
    };
    let manifest = m();
    for bad in manifest["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("-trailing"))
    {
        let c = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["video"] == bad["good_video"])
            .unwrap();
        let config = Config::parse(&hex(&c["ssc"])).unwrap();
        let mut session = Session::new(&config, Mode::Complex).unwrap();
        let data = read(&c["golden"]);
        let inputs = n(&c["input_channels"]);
        let size = n(&c["out_n"]);
        let planar = |i: usize| {
            (0..inputs)
                .map(|ch| {
                    (0..size)
                        .map(|t| {
                            let at = ((i * size + t) * inputs + ch) * 4;
                            f64::from(f32::from_le_bytes(data[at..at + 4].try_into().unwrap()))
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let stamp = |i: usize| Stamp {
            pts: (i * size) as i64,
            duration: size as u64,
            timescale: n(&c["rate"]) as u32,
        };
        for i in 0..11 {
            assert!(
                session
                    .submit(Some((&planar(i), stamp(i))), &[])
                    .unwrap()
                    .is_empty()
            );
        }
        let mut messages = Vec::new();
        for (i, row) in c["schedule"].as_array().unwrap().iter().enumerate() {
            let mut payload = hex(&row["body"]);
            let kind = if i == 0 {
                Kind::Frame
            } else {
                let ssc = hex(&row["ssc"]);
                let mut head = vec![ssc.len() as u8];
                head.extend(ssc);
                head.extend(payload);
                payload = head;
                Kind::HeaderAndFrame
            };
            messages.push(Message {
                extension: Extension::LowDelay,
                kind,
                payload,
            });
        }
        let saved = session.clone();
        let mut broken = messages.clone();
        broken.last_mut().unwrap().payload.push(255);
        let error = session
            .submit(Some((&planar(11), stamp(11))), &broken)
            .unwrap_err()
            .to_string();
        assert!(error.contains("trailing bytes"), "{error}");
        assert_eq!(session, saved);
        let got = session
            .submit(Some((&planar(11), stamp(11))), &messages)
            .unwrap();
        let want = saved
            .clone()
            .submit(Some((&planar(11), stamp(11))), &messages)
            .unwrap();
        assert_eq!(got, want);
        session.finish().unwrap();
    }
}

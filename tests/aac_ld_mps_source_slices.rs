use fvid_media::owned_aac::{NativeAacDecoder, aac_ld_mps_config::Config};
use serde_json::Value;
use std::{collections::HashMap, io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-source-slices")
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
#[test]
fn unchanged_whole_frame_controls_are_playable() {
    let manifest = m();
    assert_eq!(manifest["controls"].as_array().unwrap().len(), 112);
    for c in manifest["controls"].as_array().unwrap() {
        let pcm = movie(&c["video"]);
        assert_eq!(pcm.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(pcm.iter().any(|&v| v != 0));
    }
}
#[test]
fn inband_partial_frames_keep_the_whole_source_window_waveform() {
    let manifest = m();
    let fixed: HashMap<_, _> = manifest["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["video"].as_str().unwrap(), movie(&c["video"])))
        .collect();
    for c in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["pattern"] != "initial")
    {
        assert_eq!(
            movie(&c["video"]),
            fixed[c["control"].as_str().unwrap()],
            "{}",
            c["name"]
        );
    }
}
#[test]
fn partial_initial_ssc_is_admitted_in_pcm_and_shared_qmf_streams() {
    let manifest = m();
    for c in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["pattern"] == "initial")
    {
        let pcm = movie(&c["video"]);
        assert_eq!(pcm.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(pcm.iter().any(|&v| v != 0));
    }
}
fn component_reference(c: &Value) -> Vec<u8> {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::Mode, aac_ld_mps_pcm, aac_sbr_qmf::Complex, bits::BitReader,
    };
    let initial = Config::parse(&hex(&c["ssc"])).unwrap();
    let shared = c["shared"].as_bool().unwrap();
    let mut decoder = if shared {
        aac_ld_mps_pcm::Decoder::new_ld_sbr_qmf(&initial, Mode::Complex)
    } else {
        aac_ld_mps_pcm::Decoder::new(&initial, Mode::Complex)
    }
    .unwrap();
    let input = read(&c["input"]);
    let channels = n(&c["input_channels"]);
    let bins = n(&c["qmf_bins"]);
    let mut result = Vec::new();
    for entry in c["plan"].as_array().unwrap() {
        let config = Config::parse(&hex(&entry["ssc"])).unwrap();
        decoder.reconfigure(&config).unwrap();
        let wire = hex(&entry["body"]);
        let mut bits = BitReader::new(&wire);
        let residuals = vec![None; config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes)];
        let start = n(&entry["start"]);
        let end = n(&entry["end"]);
        let frame = if shared {
            let stored = n(&c["stored_bands"]);
            let rows_total = n(&c["samples"]) / bins;
            let rows = (start / bins..end / bins)
                .map(|t| {
                    (0..channels)
                        .map(|ch| {
                            (0..bins)
                                .map(|b| {
                                    if b >= stored {
                                        Complex::default()
                                    } else {
                                        let at = ((ch * rows_total + t) * stored + b) * 16;
                                        Complex {
                                            re: f64::from_le_bytes(
                                                input[at..at + 8].try_into().unwrap(),
                                            ),
                                            im: f64::from_le_bytes(
                                                input[at + 8..at + 16].try_into().unwrap(),
                                            ),
                                        }
                                    }
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect::<Vec<_>>();
            decoder.decode_qmf(&mut bits, &rows, &residuals).unwrap()
        } else {
            let pcm = (0..channels)
                .map(|ch| {
                    (start..end)
                        .map(|t| {
                            let at = (t * channels + ch) * 4;
                            f64::from(f32::from_le_bytes(input[at..at + 4].try_into().unwrap()))
                        })
                        .collect()
                })
                .collect::<Vec<_>>();
            decoder.decode(&mut bits, &pcm, &residuals).unwrap()
        };
        let order = [0, 1, 2, 3, 6, 7, 4, 5];
        for row in frame.samples.chunks_exact(n(&c["channels"])) {
            for ch in 0..row.len() {
                result.extend_from_slice(
                    &(row[if matches!(n(&c["tree"]), 4 | 6) {
                        order[ch]
                    } else {
                        ch
                    }] as f32)
                        .to_le_bytes(),
                );
            }
        }
    }
    result
}
#[test]
fn native_split_streams_match_saved_input_component_composition_in_both_containers() {
    let manifest = m();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 448);
    let mut maximum = 0f32;
    let mut values = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let actual = movie(&c["video"]);
        let expected = component_reference(c);
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
        assert!(error < 1e-9, "{}: {error}", c["name"]);
        maximum = maximum.max(error);
        values += actual.len() / 4;
        let mut mkv = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(&c["file"])),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mkv, actual, "{} MKV", c["name"]);
    }
    eprintln!("Split-source direct composition: {values} PCM values;max error {maximum}");
}
#[test]
fn checkpointed_partial_windows_return_original_clock_stamps_and_opaque_ids() {
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let expected = movie(&c["video"]);
        for opaque in [false, true] {
            let mut decoder =
                NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), n(&c["rate"]) as u32)
                    .unwrap();
            decoder.set_source_packet_ids(opaque).unwrap();
            let stamp = |i: usize| {
                let start = i * n(&c["out_n"]) * 100 / n(&c["rate"]);
                let end = (i + 1) * n(&c["out_n"]) * 100 / n(&c["rate"]);
                (
                    if opaque {
                        i64::MAX - i as i64 * 113
                    } else {
                        start as i64 - 13
                    },
                    (end - start) as u64,
                )
            };
            let mut windows = Vec::new();
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = n(&row["offset"]);
                let wire = &blob[at..at + n(&row["bytes"])];
                let saved = decoder.checkpoint();
                let (pts, duration) = stamp(i);
                let want = decoder
                    .decode_timed_in_clock(wire, pts, duration, 100)
                    .unwrap();
                decoder.restore(&saved).unwrap();
                let got = decoder
                    .decode_timed_in_clock(wire, pts, duration, 100)
                    .unwrap();
                assert_eq!(got, want, "{} checkpoint{i}", c["name"]);
                if let Some(frame) = got {
                    windows.push(frame);
                }
            }
            while let Some(frame) = decoder.finish().unwrap() {
                windows.push(frame);
            }
            assert_eq!(windows.len(), 12, "{}", c["name"]);
            for (i, window) in windows.iter().enumerate() {
                assert_eq!(
                    (window.pts, window.duration),
                    stamp(i),
                    "{} window{i} opaque{opaque}",
                    c["name"]
                );
                assert_eq!(window.samples.len(), n(&c["out_n"]) * n(&c["channels"]));
            }
            assert_eq!(
                windows
                    .iter()
                    .flat_map(|f| f.samples.iter().flat_map(|v| v.to_le_bytes()))
                    .collect::<Vec<_>>(),
                expected,
                "{} opaque{opaque}",
                c["name"]
            );
        }
    }
}
#[test]
fn reset_and_checkpoint_restore_a_source_window_held_across_calls() {
    let manifest = m();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "aac-ld-mps-sbr-rates-n480-r48000-c2-x2-tree2-initial")
        .unwrap();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let expected = movie(&c["video"]);
    let mut decoder =
        NativeAacDecoder::new_with_output_rate(&hex(&c["asc"]), n(&c["rate"]) as u32).unwrap();
    let first = &c["frames"][0];
    let at = n(&first["offset"]);
    assert!(
        decoder
            .decode(&blob[at..at + n(&first["bytes"])])
            .unwrap()
            .is_empty()
    );
    assert!(decoder.delayed());
    let checkpoint = decoder.checkpoint();
    let retained = decoder.retained_payload_bytes().unwrap();
    assert!(
        decoder
            .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
            .unwrap()
            > retained
    );
    decoder.reset();
    let mut replay = Vec::new();
    for row in c["frames"].as_array().unwrap() {
        let at = n(&row["offset"]);
        replay.extend(decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap());
    }
    while let Some(frame) = decoder.finish().unwrap() {
        replay.extend(frame.samples);
    }
    assert_eq!(
        replay
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        expected
    );
    decoder.restore(&checkpoint).unwrap();
    let mut restored = Vec::new();
    for row in c["frames"].as_array().unwrap().iter().skip(1) {
        let at = n(&row["offset"]);
        restored.extend(decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap());
    }
    while let Some(frame) = decoder.finish().unwrap() {
        restored.extend(frame.samples);
    }
    assert_eq!(
        restored
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        expected
    );
}
#[test]
fn malformed_late_frame_preserves_a_partially_assembled_native_window_for_retry() {
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 4);
    for bad in manifest["errors"].as_array().unwrap() {
        let mut pcm = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&read(&bad["video"]), &mut pcm)
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
        let mut decoder =
            NativeAacDecoder::new_with_output_rate(&hex(&good["asc"]), n(&good["rate"]) as u32)
                .unwrap();
        decoder.set_source_packet_ids(true).unwrap();
        let mut reference =
            NativeAacDecoder::new_with_output_rate(&hex(&good["asc"]), n(&good["rate"]) as u32)
                .unwrap();
        reference.set_source_packet_ids(true).unwrap();
        let mut result = Vec::new();
        for (i, (a, b)) in good["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(bad["frames"].as_array().unwrap())
            .enumerate()
        {
            let packet = |r: &Value| {
                let at = n(&r["offset"]);
                &blob[at..at + n(&r["bytes"])]
            };
            let pts = i as i64 * 13 - 11;
            let duration = n(&good["out_n"]) as u64;
            let clock = n(&good["rate"]) as u32;
            if i == n(&bad["bad_packet"]) {
                let saved = decoder.checkpoint();
                let error = decoder
                    .decode_timed_in_clock(packet(b), pts, duration, clock)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                let want = decoder
                    .decode_timed_in_clock(packet(a), pts, duration, clock)
                    .unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(
                    decoder
                        .decode_timed_in_clock(packet(a), pts, duration, clock)
                        .unwrap(),
                    want
                );
                decoder.restore(&saved).unwrap();
            }
            let got = decoder
                .decode_timed_in_clock(packet(a), pts, duration, clock)
                .unwrap();
            assert_eq!(
                got,
                reference
                    .decode_timed_in_clock(packet(a), pts, duration, clock)
                    .unwrap()
            );
            if let Some(frame) = got {
                result.extend(frame.samples);
            }
        }
        loop {
            let got = decoder.finish().unwrap();
            assert_eq!(got, reference.finish().unwrap());
            if let Some(frame) = got {
                result.extend(frame.samples);
            } else {
                break;
            }
        }
        assert_eq!(
            result
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            movie(&good["video"])
        );
    }
}
#[test]
fn source_span_offsets_and_original_clock_survive_zero_tick_slices() {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
        aac_ld_mps_transport::{Extension, Kind, Message},
    };
    let manifest = m();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "aac-ld-mps-downscaled-aot23-n480-ds2-c1-tree7-initial")
        .unwrap();
    let input = read(&c["input"]);
    let config = Config::parse(&hex(&c["ssc"])).unwrap();
    let mut session = Session::new(&config, Mode::Complex).unwrap();
    let size = n(&c["out_n"]);
    let clock = 100u32;
    let stamp = |i: usize| {
        let begin = i * size * clock as usize / n(&c["rate"]);
        let end = (i + 1) * size * clock as usize / n(&c["rate"]);
        Stamp {
            pts: begin as i64 - 13,
            duration: (end - begin) as u64,
            timescale: clock,
        }
    };
    let mut cursor = 0usize;
    let mut zero = 0;
    for i in 0..12 {
        let pcm = vec![
            input[i * size * 4..(i + 1) * size * 4]
                .chunks_exact(4)
                .map(|v| f64::from(f32::from_le_bytes(v.try_into().unwrap())))
                .collect(),
        ];
        let messages = c["plan"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, r)| n(&r["arrival"]) == i)
            .map(|(index, r)| {
                let body = hex(&r["body"]);
                let payload = if index == 0 {
                    body
                } else {
                    let ssc = hex(&r["ssc"]);
                    let mut p = vec![ssc.len() as u8];
                    p.extend(ssc);
                    p.extend(body);
                    p
                };
                Message {
                    extension: Extension::LowDelay,
                    kind: if index == 0 {
                        Kind::Frame
                    } else {
                        Kind::HeaderAndFrame
                    },
                    payload,
                }
            })
            .collect::<Vec<_>>();
        for frame in session.submit(Some((&pcm, stamp(i))), &messages).unwrap() {
            for source in frame.sources {
                let packet = cursor / size;
                let offset = cursor % size;
                let original = stamp(packet);
                assert_eq!(source.offset, offset);
                assert_eq!(source.source_samples, size);
                assert_eq!(source.original_stamp, original);
                let begin = (original.duration as usize * offset / size) as i64;
                let end = (original.duration as usize * (offset + source.samples) / size) as i64;
                assert_eq!(
                    source.stamp,
                    Stamp {
                        pts: original.pts + begin,
                        duration: (end - begin) as u64,
                        timescale: clock
                    }
                );
                if source.stamp.duration == 0 {
                    zero += 1;
                }
                cursor += source.samples;
            }
        }
    }
    assert_eq!(cursor, n(&c["samples"]));
    assert!(zero > 0);
    session.finish().unwrap();
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
fn split_source_movies_play_seek_and_rewind_without_exposing_partial_windows() {
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

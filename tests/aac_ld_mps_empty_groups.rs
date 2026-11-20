use fvid_media::owned_aac::{
    NativeAacDecoder, aac_ld_mps_entropy::Kind, aac_ld_mps_parameters::Parameters, bits::BitReader,
};
use serde_json::Value;
use std::{collections::HashMap, io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-empty-groups")
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
    let mut out = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&read(v), &mut out)
        .unwrap_or_else(|e| panic!("{v}: {e}"));
    out
}
#[test]
fn ordinary_empty_pcm_controls_isolate_grouped_symbol_admission() {
    let manifest = m();
    assert_eq!(manifest["controls"].as_array().unwrap().len(), 28);
    for c in manifest["controls"].as_array().unwrap() {
        let out = movie(&c["video"]);
        assert_eq!(out.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(out.iter().any(|&v| v != 0));
    }
}
#[test]
fn grouped_dummy_symbols_preserve_native_pcm_and_matroska_playback() {
    let manifest = m();
    let controls: HashMap<_, _> = manifest["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["video"].as_str().unwrap(), movie(&c["video"])))
        .collect();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 252);
    let mut values = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let actual = movie(&c["video"]);
        assert_eq!(
            actual,
            controls[c["control"].as_str().unwrap()],
            "{}",
            c["name"]
        );
        let mut mkv = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(&c["file"])),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mkv, actual, "{} Matroska", c["name"]);
        values += actual.len() / 4;
    }
    eprintln!("Empty grouped coding: {values} PCM values exactly match empty unit-stride controls");
}
#[test]
fn empty_parameters_consume_the_original_c_dummy_payload_and_rollback_prefixes() {
    let manifest = m();
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let units = manifest["units"].as_array().unwrap();
    assert_eq!(units.len(), 864);
    assert_eq!(oracle["cases"].as_array().unwrap().len(), units.len());
    for (u, c) in units.iter().zip(oracle["cases"].as_array().unwrap()) {
        let kind = match u["kind"].as_str().unwrap() {
            "CLD" => Kind::Cld,
            "ICC" => Kind::Icc,
            "CPC" => Kind::Cpc,
            _ => unreachable!(),
        };
        let wire = hex(&u["wire"]);
        let mut decoder = Parameters::new(kind, 0, u["default"].as_i64().unwrap() as i16).unwrap();
        let mut reader = BitReader::new(&wire);
        let actual = decoder
            .decode_with_quantization(&mut reader, &[3, 7], 8, u["independent"].as_bool().unwrap())
            .unwrap();
        assert_eq!(reader.position(), n(&u["bits"]));
        assert_eq!(reader.position(), n(&c["bits"]));
        assert_eq!(actual.values, vec![Vec::<i16>::new(); 2]);
        assert_eq!(actual.coarse, vec![u["coarse"].as_bool().unwrap(); 2]);
        assert_eq!(c["groups"], 1);
        assert_eq!(c["map"], serde_json::json!([0, 0]));
        assert_eq!(c["history"], 0);
        assert_eq!(c["dummy"], u["dummy"]);
        for width in 0..n(&u["bits"]) {
            let mut short = BitReader::with_bit_len(&wire, width).unwrap();
            let mut fresh =
                Parameters::new(kind, 0, u["default"].as_i64().unwrap() as i16).unwrap();
            let saved = fresh.clone();
            assert!(
                fresh
                    .decode(&mut short, &[3, 7], 8, u["independent"].as_bool().unwrap())
                    .is_err(),
                "{u} prefix{width}"
            );
            assert_eq!(short.position(), 0);
            assert_eq!(fresh, saved);
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
fn grouped_empty_movies_play_seek_and_rewind_with_exact_audio() {
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
fn grouped_empty_packets_restore_checkpoints_and_original_clock_stamps() {
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
        }
    }
}
#[test]
fn invalid_empty_pcm_symbol_is_rejected_transactionally_and_can_be_retried() {
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 28);
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
fn historical_split_ttt_refusal_movie_now_preserves_the_control_waveform() {
    let manifest = m();
    let cases = manifest["ttt_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1);
    for c in cases {
        assert_eq!(movie(&c["video"]), movie(&c["control"]));
    }
}

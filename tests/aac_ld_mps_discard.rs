use fvid_media::owned_aac::{
    NativeAacDecoder, aac_ld_mps_entropy::Kind, aac_ld_mps_parameters::Parameters, bits::BitReader,
};
use serde_json::Value;
use std::{collections::HashMap, io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-discard")
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
fn all_split_ttt_unit_stride_controls_are_playable() {
    let manifest = m();
    assert_eq!(manifest["controls"].as_array().unwrap().len(), 84);
    for c in manifest["controls"].as_array().unwrap() {
        let out = movie(&c["video"]);
        assert_eq!(out.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(out.iter().any(|&v| v != 0));
    }
}
#[test]
fn grouped_split_ttt_movies_preserve_control_audio_in_both_containers() {
    let manifest = m();
    let controls: HashMap<_, _> = manifest["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["video"].as_str().unwrap(), movie(&c["video"])))
        .collect();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 756);
    let mut scalars = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let out = movie(&c["video"]);
        assert_eq!(
            out,
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
        assert_eq!(mkv, out, "{} MKV", c["name"]);
        scalars += out.len() / 4;
    }
    eprintln!("Discarded TTT groups: {scalars} PCM values match unit-stride controls exactly");
}
#[test]
fn discarded_indices_have_original_c_cursors_without_weakening_real_band_bounds() {
    let manifest = m();
    let units = manifest["units"].as_array().unwrap();
    assert_eq!(units.len(), 936);
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let rows = oracle["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 10920);
    let mut seen = vec![0; units.len()];
    let mut out_of_range = 0;
    for row in rows {
        let i = n(&row["unit"]);
        let u = &units[i];
        seen[i] += 1;
        assert_eq!(n(&row["bits"]), n(&u["bits"]));
        assert_eq!(row["map"], serde_json::json!([0, 0]));
        assert_eq!(row["groups"], 1);
        let bounds = match (u["kind"].as_str().unwrap(), u["coarse"].as_bool().unwrap()) {
            ("CLD", false) => (-15, 15),
            ("CLD", true) => (-7, 7),
            ("ICC", false) => (0, 7),
            ("ICC", true) => (0, 3),
            ("CPC", false) => (-20, 30),
            ("CPC", true) => (-10, 15),
            _ => unreachable!(),
        };
        out_of_range += row["dummy"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| {
                let x = v.as_i64().unwrap();
                x < bounds.0 || x > bounds.1
            })
            .count();
    }
    assert!(out_of_range > 1000);
    for (i, u) in units.iter().enumerate() {
        let kind = match u["kind"].as_str().unwrap() {
            "CLD" => Kind::Cld,
            "ICC" => Kind::Icc,
            "CPC" => Kind::Cpc,
            _ => unreachable!(),
        };
        assert_eq!(
            seen[i],
            if kind == Kind::Icc {
                8
            } else if kind == Kind::Cpc && !u["coarse"].as_bool().unwrap() {
                12
            } else {
                14
            }
        );
        let wire = hex(&u["wire"]);
        let mut decoder = Parameters::new(kind, 0, u["default"].as_i64().unwrap() as i16).unwrap();
        let mut b = BitReader::new(&wire);
        let out = decoder
            .decode_with_quantization(&mut b, &[3, 7], 8, u["independent"].as_bool().unwrap())
            .unwrap_or_else(|e| panic!("unit{i} {u}: {e}"));
        assert_eq!(b.position(), n(&u["bits"]));
        assert_eq!(out.values, vec![Vec::<i16>::new(); 2]);
        assert_eq!(out.coarse, vec![u["coarse"].as_bool().unwrap(); 2]);
        if u["adversarial"] == true {
            let mut real = Parameters::new(kind, 1, 0).unwrap();
            let saved = real.clone();
            let mut b = BitReader::new(&wire);
            let e = real
                .decode(&mut b, &[3, 7], 8, false)
                .unwrap_err()
                .to_string();
            assert!(
                e.contains("parameter index exceeds quantization range"),
                "{e}"
            );
            assert_eq!(b.position(), 0);
            assert_eq!(real, saved);
        }
        for width in 0..n(&u["bits"]) {
            let mut b = BitReader::with_bit_len(&wire, width).unwrap();
            let mut d = decoder.clone();
            let saved = d.clone();
            assert!(
                d.decode(&mut b, &[3, 7], 8, u["independent"].as_bool().unwrap())
                    .is_err(),
                "unit{i} prefix{width}"
            );
            assert_eq!(b.position(), 0);
            assert_eq!(d, saved);
        }
    }
    eprintln!("Original C discarded indices outside current bounds: {out_of_range}");
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
fn split_ttt_movies_play_seek_and_rewind_with_exact_audio() {
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
fn discarded_groups_keep_checkpoints_reset_and_original_clock_stamps() {
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
fn discarded_groups_still_reject_invalid_pcm_codewords_transactionally() {
    let manifest = m();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    assert_eq!(manifest["errors"].as_array().unwrap().len(), 84);
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

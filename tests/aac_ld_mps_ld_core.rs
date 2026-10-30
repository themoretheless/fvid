use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-ld-core")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn movie(c: &Value) -> Vec<u8> {
    std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap()
}
fn outputs(c: &Value) -> usize {
    [6, 6, 6, 8, 8, 8, 8, 2][n(&c["tree"])]
}
#[test]
fn aac_ld_with_attached_spatial_config_plays_instead_of_refusing_asc_tail() {
    use fvid_media::owned_aac::{NativeAacDecoder, config::AudioSpecificConfig};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 318);
    for c in m["cases"].as_array().unwrap() {
        let data = movie(c);
        let mut pcm = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let asc = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        assert_eq!(asc.core.object_type, 23);
        assert!(asc.ld_mps.is_some());
        assert_eq!(stats.channels as usize, outputs(c));
        assert_eq!(stats.sample_rate as usize, n(&c["rate"]));
        assert_eq!(stats.sample_frames as usize, n(&c["samples"]));
        assert_eq!(pcm.len(), n(&c["samples"]) * outputs(c) * 4);
        assert!(pcm.iter().any(|&v| v != 0));
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let blob = std::fs::read(root().join("packets.bin")).unwrap();
        let mut untimed = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let at = n(&row["offset"]);
            untimed.extend(decoder.decode(&blob[at..at + n(&row["bytes"])]).unwrap());
        }
        while let Some(frame) = decoder.finish().unwrap() {
            untimed.extend(frame.samples);
        }
        let full = if c["mode"] == "padding" {
            let plain = m["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == c["name"].as_str().unwrap().replace("-padding", "-plain"))
                .unwrap();
            let mut bytes = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&movie(plain), &mut bytes).unwrap();
            bytes
        } else {
            pcm
        };
        assert_eq!(untimed.len() * 4, full.len());
        for (want, got) in full.chunks_exact(4).zip(untimed) {
            assert_eq!(
                f32::from_le_bytes(want.try_into().unwrap()),
                got,
                "{} untimed",
                c["name"]
            );
        }
    }
}

fn duration(c: &Value, i: usize) -> usize {
    c["durations"].as_array().map_or(n(&c["n"]), |v| {
        let ticks = n(&v[i]) * n(&c["rate"]);
        assert_eq!(ticks % n(&c["container_rate"]), 0);
        ticks / n(&c["container_rate"])
    })
}
fn reference(c: &Value, golden: &[u8]) -> Vec<f64> {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::Config,
        aac_ld_mps_decor::Mode,
        aac_ld_mps_session::{Session, Stamp},
        aac_ld_mps_transport::{Extension, Kind, Message},
    };
    let cfg = Config::parse(&hex(c["ssc"].as_str().unwrap())).unwrap();
    let mut session = Session::new(&cfg, Mode::Complex).unwrap();
    let mut result = Vec::new();
    let width = n(&c["channels"]);
    let samples = n(&c["n"]);
    let offset = n(&c["core_reference_offset"]);
    let mut pts = 0;
    for i in 0..8 {
        let mut planar = vec![vec![0f64; samples]; width];
        for t in 0..samples {
            for ch in 0..width {
                let at = offset + ((i * samples + t) * width + ch) * 4;
                planar[ch][t] =
                    f64::from(f32::from_le_bytes(golden[at..at + 4].try_into().unwrap()));
            }
        }
        let mut messages = Vec::new();
        if (i + 1) % n(&c["group"]) == 0 {
            let f = &c["reference"][i / n(&c["group"])];
            let mut payload = hex(f["spatial"].as_str().unwrap());
            let header = f["header"].as_bool().unwrap();
            if header {
                let cfg = hex(f["ssc"].as_str().unwrap());
                let mut prefix = if cfg.len() < 127 {
                    vec![cfg.len() as u8]
                } else {
                    let mut v = vec![127];
                    v.extend_from_slice(&((cfg.len() - 127) as u16).to_be_bytes());
                    v
                };
                prefix.extend(cfg);
                prefix.extend(payload);
                payload = prefix;
            }
            messages.push(Message {
                extension: Extension::LowDelay,
                kind: if header {
                    Kind::HeaderAndFrame
                } else {
                    Kind::Frame
                },
                payload,
            });
        }
        let frames = session
            .submit(
                Some((
                    &planar,
                    Stamp {
                        pts,
                        duration: duration(c, i) as u64,
                        timescale: n(&c["rate"]) as u32,
                    },
                )),
                &messages,
            )
            .unwrap();
        pts += duration(c, i) as i64;
        for frame in frames {
            for row in frame.pcm.samples.chunks_exact(outputs(c)) {
                for wave in 0..outputs(c) {
                    let ch = if matches!(n(&c["tree"]), 4 | 6) {
                        [0, 1, 2, 3, 6, 7, 4, 5][wave]
                    } else {
                        wave
                    };
                    result.push(row[ch]);
                }
            }
        }
    }
    session.finish().unwrap();
    assert_eq!(result.len(), 8 * samples * outputs(c));
    let mut presented = Vec::new();
    for i in 0..8 {
        let first = i * samples * outputs(c);
        presented.extend_from_slice(&result[first..first + duration(c, i) * outputs(c)]);
    }
    presented
}
#[test]
fn full_spatial_pcm_matches_the_independent_ld_core_oracle_and_qualified_session() {
    let gold = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-ld-layout-reference.f32le"),
    )
    .unwrap();
    let mut maximum = 0f64;
    let mut count = 0;
    for c in manifest()["cases"].as_array().unwrap() {
        let want = reference(c, &gold);
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut pcm).unwrap();
        assert_eq!(pcm.len(), want.len() * 4);
        for (i, (got, want)) in pcm.chunks_exact(4).zip(want).enumerate() {
            let got = f64::from(f32::from_le_bytes(got.try_into().unwrap()));
            let error = (got - want).abs();
            maximum = maximum.max(error);
            assert!(error < 1e-9, "{} sample{i}: {got} != {want}", c["name"]);
            count += 1;
        }
    }
    eprintln!("AAC-LD spatial oracle samples {count}, maximum PCM difference {maximum}");
}
#[test]
fn original_core_windows_keep_exact_nondefault_source_clocks() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut frames = Vec::new();
        let mut pts = 17i64;
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = n(&row["offset"]);
            if let Some(f) = d
                .decode_timed_in_clock(
                    &blob[at..at + n(&row["bytes"])],
                    pts,
                    (duration(c, i) * 3) as u64,
                    (n(&c["rate"]) * 3) as u32,
                )
                .unwrap()
            {
                frames.push(f);
            }
            pts += (duration(c, i) * 3) as i64;
        }
        while let Some(f) = d.finish().unwrap() {
            frames.push(f);
        }
        assert_eq!(frames.len(), 8, "{} source frame count", c["name"]);
        let mut pts = 17;
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(
                (f.pts, f.duration),
                (pts, (duration(c, i) * 3) as u64),
                "{} source{i}",
                c["name"]
            );
            assert_eq!(f.samples.len(), n(&c["n"]) * outputs(c));
            pts += (duration(c, i) * 3) as i64;
        }
    }
}
fn same_pcm(got: &[u8], want: &[u8], name: &Value, stage: &str) {
    assert_eq!(got.len(), want.len(), "{name} {stage} byte count");
    let first = got.iter().zip(want).position(|(a, b)| a != b);
    assert!(
        first.is_none(),
        "{name} {stage} first differing byte {first:?}"
    );
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut data = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&p);
        if let Some(mut f) = reader.decode_source_packet(decoder, &p, stamp).unwrap() {
            if let Some(limit) = reader
                .decoded_packet_sample_limit(f.source_pts, f.source_duration)
                .unwrap()
            {
                let bytes = limit * usize::from(reader.channels()) * 4;
                assert!(bytes <= f.packet.data.len());
                f.packet.data.truncate(bytes);
            }
            if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                data.extend(pcm.data);
            }
        }
    }
    while let Some(mut f) = decoder.finish_packet().unwrap() {
        if let Some(limit) = reader
            .decoded_packet_sample_limit(f.source_pts, f.source_duration)
            .unwrap()
        {
            let bytes = limit * usize::from(reader.channels()) * 4;
            assert!(bytes <= f.packet.data.len());
            f.packet.data.truncate(bytes);
        }
        if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            data.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    data
}
#[test]
fn stream_playback_rewind_seek_and_repeat_ranges_keep_the_same_source_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = movie(c);
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut full).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        same_pcm(&full, &owned, &c["name"], "owned");
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let mut d = reader.make_decoder().unwrap();
        let width = usize::from(reader.channels()) * 4;
        same_pcm(&play(&mut reader, d.as_mut()), &full, &c["name"], "live");
        reader.rewind();
        d.reset();
        same_pcm(&play(&mut reader, d.as_mut()), &full, &c["name"], "rewind");
        for target in [13, 4 * n(&c["n"]) + 7, 7, full.len() / width] {
            let ticks = (target as u128 * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            d.reset();
            assert_eq!(
                play(&mut reader, d.as_mut()),
                full[first..],
                "{} seek{target}",
                c["name"]
            );
        }
        for _ in 0..2 {
            let mut range = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut range,
                Some((
                    Duration::from_secs_f64(13. / n(&c["rate"]) as f64),
                    Duration::from_secs_f64((4 * n(&c["n"]) + 7) as f64 / n(&c["rate"]) as f64),
                )),
                &Default::default(),
            )
            .unwrap();
            let start = (13f64 / n(&c["rate"]) as f64 * 1e9).round() as u64;
            let end = (((4 * n(&c["n"]) + 7) as f64 / n(&c["rate"]) as f64) * 1e9).round() as u64;
            let first = (u128::from(start) * n(&c["rate"]) as u128).div_ceil(1_000_000_000)
                as usize
                * width;
            let last =
                (u128::from(end) * n(&c["rate"]) as u128).div_ceil(1_000_000_000) as usize * width;
            assert_eq!(range, full[first..last], "{} range", c["name"]);
        }
    }
}
#[test]
fn absent_flags_and_bad_lengths_are_distinct_from_late_spatial_rollback() {
    use fvid_media::owned_aac::{NativeAacDecoder, config::AudioSpecificConfig};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let gold = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("aac-ld-layout-reference.f32le"),
    )
    .unwrap();
    for c in m["controls"].as_array().unwrap() {
        let asc = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        assert!(asc.ld_mps.is_none());
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut pcm).unwrap();
        let at = n(&c["core_reference_offset"]);
        for (got, want) in pcm
            .chunks_exact(4)
            .zip(gold[at..at + 8 * n(&c["n"]) * n(&c["channels"]) * 4].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(got.try_into().unwrap())
                    - f32::from_le_bytes(want.try_into().unwrap()))
                .abs()
                    < 1e-10
            );
        }
    }
    for c in m["asc_errors"].as_array().unwrap() {
        let error = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
            .unwrap_err()
            .to_string();
        assert!(error.contains(c["error"].as_str().unwrap()), "{error}");
        assert!(
            fvid::native_media::decode_mp4_aac_pcm(&movie(c), &mut Vec::new())
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
    }
    for bad in m["errors"].as_array().unwrap() {
        let name = bad["name"].as_str().unwrap().split("-bad-").next().unwrap();
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie(bad), &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{}: {error}",
            bad["name"]
        );
        let asc = hex(good["asc"].as_str().unwrap());
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        let mut control = NativeAacDecoder::new(&asc).unwrap();
        let size = n(&good["n"]);
        for i in 0..8 {
            let r = &good["frames"][i];
            let at = n(&r["offset"]);
            let packet = &blob[at..at + n(&r["bytes"])];
            let want = control
                .decode_timed(packet, (i * size) as i64, size as u64)
                .unwrap();
            if i == 3 {
                let saved = d.checkpoint();
                let retained = d.retained_payload_bytes().unwrap();
                let row = &bad["frames"][i];
                let offset = n(&row["offset"]);
                let error = d
                    .decode_timed(
                        &blob[offset..offset + n(&row["bytes"])],
                        (i * size) as i64,
                        size as u64,
                    )
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                assert_eq!(d.retained_payload_bytes().unwrap(), retained);
                assert_eq!(
                    d.decode_timed(packet, (i * size) as i64, size as u64)
                        .unwrap(),
                    want
                );
                d.restore(&saved).unwrap();
            }
            assert_eq!(
                d.decode_timed(packet, (i * size) as i64, size as u64)
                    .unwrap(),
                want
            );
        }
        loop {
            let a = d.finish().unwrap();
            let b = control.finish().unwrap();
            assert_eq!(a, b);
            if a.is_none() {
                break;
            }
        }
    }
}

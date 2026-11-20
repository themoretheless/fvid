use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(CORPUS)
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
#[test]
fn bare_high_rate_eld_controls_match_the_scalar_oracle() {
    let m = manifest();
    let gold = std::fs::read(root().join("core-reference.f32le")).unwrap();
    assert_eq!(m["controls"].as_array().unwrap().len(), 18);
    for c in m["controls"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut p = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut p)
            .unwrap_or_else(|e| panic!("{} bare ELD: {e}", c["name"]));
        assert_eq!(stats.sample_rate as usize, n(&c["rate"]));
        assert_eq!(stats.channels as usize, n(&c["channels"]));
        let at = n(&c["core_reference_offset"]);
        let count = n(&c["samples"]) * n(&c["channels"]) * 4;
        assert_eq!(p.len(), count);
        for (a, b) in p.chunks_exact(4).zip(gold[at..at + count].chunks_exact(4)) {
            let e = (f32::from_le_bytes(a.try_into().unwrap())
                - f32::from_le_bytes(b.try_into().unwrap()))
            .abs();
            assert!(e < 1e-7, "{} bare core difference {e}", c["name"]);
        }
    }
}
#[test]
fn ordinary_128_bin_eld_spatial_movies_play_instead_of_refusing_decorrelation() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), PROGRAMMES);
    for c in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut p = Vec::new();
        let s = fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut p)
            .unwrap_or_else(|e| panic!("{} spatial: {e}", c["name"]));
        assert_eq!(s.sample_rate as usize, n(&c["rate"]));
        assert_eq!(s.channels as usize, n(&c["output_channels"]));
        assert_eq!(s.sample_frames as usize, n(&c["samples"]));
        assert_eq!(p.len(), n(&c["samples"]) * n(&c["output_channels"]) * 4);
        assert!(p.iter().any(|&b| b != 0));
        let mut own = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut own,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(p == own, "{} owned MP4", c["name"]);
    }
}

fn outputs(c: &Value) -> usize {
    n(&c["output_channels"])
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn mp4(c: &Value) -> Vec<u8> {
    std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap()
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
    for i in 0..n(&c["core_count"]) {
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
    assert_eq!(result.len(), n(&c["core_count"]) * samples * outputs(c));
    let mut presented = Vec::new();
    for i in 0..n(&c["core_count"]) {
        let first = i * samples * outputs(c);
        presented.extend_from_slice(&result[first..first + duration(c, i) * outputs(c)]);
    }
    presented
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    while let Some(p) = reader.next_packet()? {
        let stamp = reader.decoder_stamp(&p);
        if let Some(mut f) = reader.decode_source_packet(decoder, &p, stamp)? {
            if let Some(limit) =
                reader.decoded_packet_sample_limit(f.source_pts, f.source_duration)?
            {
                let bytes = limit * usize::from(reader.channels()) * 4;
                assert!(bytes <= f.packet.data.len());
                f.packet.data.truncate(bytes);
            }
            if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts)? {
                data.extend(pcm.data);
            }
        }
    }
    while let Some(mut f) = decoder.finish_packet()? {
        if let Some(limit) = reader.decoded_packet_sample_limit(f.source_pts, f.source_duration)? {
            let bytes = limit * usize::from(reader.channels()) * 4;
            assert!(bytes <= f.packet.data.len());
            f.packet.data.truncate(bytes);
        }
        if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts)? {
            data.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet()?.is_none());
    Ok(data)
}

#[test]
fn spatial_128_pcm_matches_the_scalar_eld_core_and_qualified_spatial_chain() {
    let m = manifest();
    let gold = std::fs::read(root().join("core-reference.f32le")).unwrap();
    let mut count = 0;
    let mut maximum = 0f64;
    for c in m["cases"].as_array().unwrap() {
        let mut p = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4(c), &mut p).unwrap();
        let want = reference(c, &gold);
        assert_eq!(p.len(), want.len() * 4);
        for (row, want) in p.chunks_exact(4).zip(want) {
            let got = f64::from(f32::from_le_bytes(row.try_into().unwrap()));
            let error = (got - want).abs();
            maximum = maximum.max(error);
            count += 1;
            assert!(error < 1e-9, "{} scalar spatial error {error}", c["name"]);
        }
    }
    eprintln!("128-bin native spatial oracle samples {count}; maximum difference {maximum}");
}

fn expected(c: &Value) -> Vec<u8> {
    let mut p = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&mp4(c), &mut p).unwrap();
    p
}
#[test]
fn quantized_matroska_exports_and_navigation_keep_all_source_windows() {
    use fvid::audio::AudioStream;
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), PROGRAMMES);
    let mut cached = None;
    let mut full = Vec::new();
    for c in m["cases"].as_array().unwrap() {
        let index = c["name"].as_str().unwrap();
        if cached != Some(index) {
            full = expected(c);
            cached = Some(index);
        }
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut p = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&bytes, &mut p, None)
            .unwrap_or_else(|e| panic!("{} frontend {e}", c["name"]));
        assert!(p == full, "{} frontend PCM", c["name"]);
        p.clear();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut p,
            None,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{} owned {e}", c["name"]));
        assert!(p == full, "{} owned PCM", c["name"]);
        let mut r = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let mut d = r.make_decoder().unwrap();
        let stride = usize::from(r.channels()) * 4;
        for stage in ["initial", "rewind"] {
            let p =
                play(&mut r, d.as_mut()).unwrap_or_else(|e| panic!("{} {stage}: {e}", c["name"]));
            assert!(p == full, "{} {stage} PCM", c["name"]);
            r.rewind();
            d.reset();
        }
        for target in [
            13,
            3 * n(&c["n"]) + 7,
            6 * n(&c["n"]) + 11,
            8 * n(&c["n"]) + 7,
            7,
            full.len() / stride,
        ] {
            let ticks =
                (target as u128 * u128::from(r.timescale()) / u128::from(r.sample_rate())) as i64;
            let at = r.seek_to(ticks);
            d.reset();
            let first = ((at.max(0) as u128 * u128::from(r.sample_rate()))
                .div_ceil(u128::from(r.timescale())) as usize
                * stride)
                .min(full.len());
            let p = play(&mut r, d.as_mut())
                .unwrap_or_else(|e| panic!("{} seek{target}: {e}", c["name"]));
            assert!(p == full[first..], "{} seek{target} PCM", c["name"]);
        }
        for _ in 0..2 {
            let rate = c["rate"].as_u64().unwrap();
            let a = std::time::Duration::from_nanos((13u64 * 1_000_000_000).div_ceil(rate));
            let b = std::time::Duration::from_nanos(
                ((3 * c["n"].as_u64().unwrap() + 7) * 1_000_000_000).div_ceil(rate),
            );
            let first = (a.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * stride;
            let last = (b.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * stride;
            let mut p = Vec::new();
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&bytes),
                &mut p,
                Some((a, b)),
                &Default::default(),
            )
            .unwrap();
            assert!(p == full[first..last], "{} repeated range", c["name"]);
        }
    }
}

#[test]
fn late_spatial_corruption_preserves_multicore_history_and_reaches_the_specific_error() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for item in m["errors"].as_array().unwrap() {
        let bad = item;
        let name = bad["name"].as_str().unwrap().split("-bad-").next().unwrap();
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let expected = bad["error"].as_str().unwrap();
        let mkv = std::fs::read(root().join(item["file"].as_str().unwrap())).unwrap();
        let mut p = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(&mp4(bad), &mut p).unwrap_err();
        assert!(e.to_string().contains(expected), "{} MP4: {e}", bad["name"]);
        p.clear();
        let e =
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut p, None).unwrap_err();
        assert!(
            e.to_string().contains(expected),
            "{} Matroska: {e}",
            bad["name"]
        );
        p.clear();
        let e = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut p,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            e.to_string().contains(expected),
            "{} owned Matroska: {e}",
            bad["name"]
        );
        let mut r =
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(mkv), Default::default())
                .unwrap();
        let mut adapter = r.make_decoder().unwrap();
        for _ in 0..2 {
            let e = play(&mut r, adapter.as_mut()).unwrap_err();
            assert!(
                e.to_string().contains(expected),
                "{} playback: {e}",
                bad["name"]
            );
            r.rewind();
            adapter.reset();
        }
        let mut d = NativeAacDecoder::new(&hex(good["asc"].as_str().unwrap())).unwrap();
        let mut control = NativeAacDecoder::new(&hex(good["asc"].as_str().unwrap())).unwrap();
        for (i, row) in good["frames"].as_array().unwrap().iter().enumerate() {
            let offset = n(&row["offset"]);
            let packet = &blob[offset..offset + n(&row["bytes"])];
            let pts = (i * n(&good["n"])) as i64;
            let duration = n(&good["n"]) as u64;
            let want = control.decode_timed(packet, pts, duration).unwrap();
            if i == 2 * n(&good["group"]) - 1 {
                let badrow = &bad["frames"][i];
                let at = n(&badrow["offset"]);
                let saved = d.checkpoint();
                let retained = d.retained_payload_bytes().unwrap();
                let e = d
                    .decode_timed(&blob[at..at + n(&badrow["bytes"])], pts, duration)
                    .unwrap_err();
                assert!(
                    e.to_string().contains(expected),
                    "{} native: {e}",
                    bad["name"]
                );
                assert_eq!(d.retained_payload_bytes().unwrap(), retained);
                assert_eq!(d.decode_timed(packet, pts, duration).unwrap(), want);
                d.restore(&saved).unwrap();
            }
            assert_eq!(
                d.decode_timed(packet, pts, duration).unwrap(),
                want,
                "{} retry source{i}",
                bad["name"]
            );
        }
        loop {
            let got = d.finish().unwrap();
            let want = control.finish().unwrap();
            assert_eq!(got, want);
            if got.is_none() {
                break;
            }
        }
    }
}

#[test]
fn wide_spatial_controlled_prefixes_drain_and_partial_windows_refuse() {
    let m = manifest();
    let mut done = std::collections::BTreeSet::new();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "plain" && c["tree"] == 7)
    {
        assert!(done.insert((n(&c["n"]), n(&c["rate"]))));
        let data = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let full = expected(c);
        let packets = (2 * n(&c["group"])) as u64;
        let frames = packets * c["n"].as_u64().unwrap();
        let mut p = Vec::new();
        let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&data),
            &mut p,
            None,
            &fvid_control::CopyOptions {
                max_packets: Some(packets),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_frames, frames);
        assert!(p == full[..frames as usize * outputs(c) * 4]);
        if c["group"] == 4 {
            p.clear();
            let error = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&data),
                &mut p,
                None,
                &fvid_control::CopyOptions {
                    max_packets: Some(3),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("unfinished LD-MPS PCM/spatial association")
            );
            assert!(p.is_empty());
        }
    }
    assert_eq!(done.len(), 6);
}

use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-22-2-esc")
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn packet(blob: &[u8], r: &Value, key: &str) -> Vec<u8> {
    let at = r[format!("{key}offset")].as_u64().unwrap() as usize;
    let len = r[format!("{key}bytes")].as_u64().unwrap() as usize;
    blob[at..at + len].to_vec()
}
fn broken_packet(name: &str) -> Vec<u8> {
    let mut r = fvid::container::mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(root().join(format!("{name}-crc.mp4"))).unwrap()),
        Default::default(),
    )
    .unwrap();
    let index = r
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let mut data = Vec::new();
    r.read_packet(index, 2, &mut data).unwrap();
    data
}
#[test]
fn ep3_22_2_classes_match_scalar_pcm_and_bad_crc_preserves_all_histories() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain.bin")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8);
    let mut maximum = 0.0f32;
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        assert_eq!(c["classes"], 56);
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut control = NativeAacDecoder::new(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        assert_eq!(d.channels(), 24);
        let n = c["n"].as_u64().unwrap();
        let bad = broken_packet(name);
        let mut pcm = Vec::new();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let good = packet(&blob, r, "");
            let original = packet(&plain, r, "plain_");
            let stamp = i as i64 * n as i64;
            let saved = d.checkpoint();
            if i == 2 {
                let err = d.decode_timed(&bad, stamp, n).unwrap_err().to_string();
                assert!(err.contains("EP class CRC mismatch"), "{name}: {err}");
            }
            let actual = d
                .decode_timed(&good, stamp, n)
                .unwrap_or_else(|e| panic!("{name} AU{i}: {e}"));
            let want = control.decode_timed(&original, stamp, n).unwrap();
            assert_eq!(actual, want);
            d.restore(&saved).unwrap();
            assert_eq!(d.decode_timed(&good, stamp, n).unwrap(), want);
            if let Some(f) = actual {
                pcm.extend(f.samples);
            }
            assert!(
                d.retained_payload_bytes_with_checkpoint(Some(&saved))
                    .unwrap()
                    > 0
            );
        }
        while let Some(f) = d.finish().unwrap() {
            assert_eq!(Some(f.clone()), control.finish().unwrap());
            pcm.extend(f.samples);
        }
        assert!(control.finish().unwrap().is_none());
        let reference = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        assert_eq!(pcm.len() * 4, reference.len());
        for (i, (v, b)) in pcm.iter().zip(reference.chunks_exact(4)).enumerate() {
            let want = f32::from_le_bytes(b.try_into().unwrap());
            let error = (v - want).abs();
            assert!(
                v.is_finite() && error < 1e-7,
                "{name} scalar{i}: {v} != {want}"
            );
            maximum = maximum.max(error);
        }
    }
    eprintln!("AAC22.2 EP3 scalar maximum: {maximum}");
}
fn play(r: &mut dyn fvid::audio::AudioStream, d: &mut dyn fvid::audio::AudioDecode) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(p) = r.next_packet().unwrap() {
        let stamp = r.decoder_stamp(&p);
        if let Some(f) = r.decode_source_packet(d, &p, stamp).unwrap() {
            if let Some(p) = r.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    while let Some(f) = d.finish_packet().unwrap() {
        if let Some(p) = r.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    out
}
#[test]
fn ep3_22_2_movies_export_rewind_seek_and_reach_the_specific_crc_failure() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mp4 = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let mkv = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut expected).unwrap();
        assert_eq!(expected.len() as u64, c["samples"].as_u64().unwrap() * 96);
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, expected);
        let mut other = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut other,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(other, expected);
        let readers: [Box<dyn AudioStream>; 2] = [
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(mp4),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(mkv),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for mut r in readers {
            let mut d = r.make_decoder().unwrap();
            assert_eq!(play(r.as_mut(), d.as_mut()), expected);
            r.rewind();
            d.reset();
            assert_eq!(play(r.as_mut(), d.as_mut()), expected);
            for sample in [
                13,
                c["n"].as_u64().unwrap(),
                c["samples"].as_u64().unwrap(),
                7,
            ] {
                let target = (u128::from(sample) * u128::from(r.timescale())
                    / u128::from(r.sample_rate())) as i64;
                let landed = r.seek_to(target);
                d.reset();
                let at = ((landed.max(0) as u128 * u128::from(r.sample_rate()))
                    .div_ceil(u128::from(r.timescale())) as usize
                    * 96)
                    .min(expected.len());
                assert_eq!(
                    play(r.as_mut(), d.as_mut()),
                    expected[at..],
                    "{name} seek{sample}"
                );
            }
        }
        let mut rejected = Vec::new();
        let bad = std::fs::read(root().join(format!("{name}-crc.mp4"))).unwrap();
        let err = fvid::native_media::decode_mp4_aac_pcm(&bad, &mut rejected)
            .unwrap_err()
            .to_string();
        assert!(err.contains("EP class CRC mismatch"), "{name}: {err}");
    }
}

#[test]
fn rvlc_22_2_ep1_groups_keep_resilience_pcm_dependencies_and_mp4_navigation() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::aac_ep1::AccessUnit;
    let m = manifest();
    let units = std::fs::read(root().join("units.bin")).unwrap();
    let plain = std::fs::read(root().join("plain.bin")).unwrap();
    assert_eq!(m["ep1"].as_array().unwrap().len(), 10);
    for c in m["ep1"].as_array().unwrap() {
        let sbr = c["name"].as_str().unwrap().contains("-sbr-");
        assert_eq!(c["instances"], if sbr { 84 } else { 80 });
        assert_eq!(c["groups"], if sbr { 42 } else { 40 });
        let name = c["name"].as_str().unwrap();
        let n = c["n"].as_u64().unwrap();
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut control = NativeAacDecoder::new(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        let mut produced = Vec::new();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let pts = -17 + i as i64 * n as i64;
            let inputs: Vec<_> = r["units"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(ordinal, g)| {
                    let at = g["offset"].as_u64().unwrap() as usize;
                    let count = g["bytes"].as_u64().unwrap() as usize;
                    let scale = 2 + ordinal as u32 % 3;
                    AccessUnit {
                        stream_id: g["stream_id"].as_u64().unwrap() as u16,
                        depends_on: g["depends_on"].as_u64().map(|v| v as u16),
                        data: &units[at..at + count],
                        bit_len: g["bits"].as_u64().unwrap() as usize,
                        pts: pts * i64::from(scale),
                        duration: n * u64::from(scale),
                        timescale: 48000 * scale,
                    }
                })
                .collect();
            let saved = decoder.checkpoint();
            assert!(decoder.decode_esc(&inputs[..inputs.len() - 1]).is_err());
            let original = packet(&plain, r, "");
            let frame = decoder
                .decode_esc(&inputs)
                .unwrap_or_else(|e| panic!("{name} AU{i}: {e}"));
            // Root stream is last in transmitted order; its clock may differ
            // from the logical control. PCM and rational source time must agree.
            let want = control.decode_timed(&original, pts, n).unwrap();
            assert_eq!(
                frame.as_ref().map(|f| &f.samples),
                want.as_ref().map(|f| &f.samples)
            );
            let root_clock = inputs
                .iter()
                .find(|u| u.depends_on.is_none())
                .unwrap()
                .timescale;
            if let (Some(got), Some(expected)) = (&frame, &want) {
                assert_eq!(
                    i128::from(got.pts) * 48000,
                    i128::from(expected.pts) * i128::from(root_clock)
                );
                assert_eq!(
                    u128::from(got.duration) * 48000,
                    u128::from(expected.duration) * u128::from(root_clock)
                );
            }

            decoder.restore(&saved).unwrap();
            assert_eq!(decoder.decode_esc(&inputs).unwrap(), frame);
            if let Some(frame) = frame {
                produced.extend(frame.samples);
            }
        }
        assert!(decoder.finish().unwrap().is_none());
        assert!(control.finish().unwrap().is_none());
        let scalar = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        assert_eq!(scalar.len(), produced.len() * 4);
        for (v, b) in produced.iter().zip(scalar.chunks_exact(4)) {
            let want = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                v.is_finite() && (*v - want).abs() < 1e-7,
                "{name}: {v} != {want}"
            );
        }
        let movie = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let original = std::fs::read(root().join(format!("{name}-control.mp4"))).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&original, &mut expected).unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut actual).unwrap();
        assert_eq!(actual, expected);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(movie), Default::default())
                .unwrap();
        let mut d = reader.make_decoder().unwrap();
        assert_eq!(play(&mut reader, d.as_mut()), expected);
        reader.rewind();
        d.reset();
        assert_eq!(play(&mut reader, d.as_mut()), expected);
        for sample in [13, n, c["samples"].as_u64().unwrap(), 7] {
            let target = (u128::from(sample) * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(target);
            d.reset();
            let at = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * 96)
                .min(expected.len());
            assert_eq!(
                play(&mut reader, d.as_mut()),
                expected[at..],
                "{name} seek{sample}"
            );
        }
    }
}

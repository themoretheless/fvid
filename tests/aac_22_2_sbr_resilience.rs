use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-22-2-sbr-resilience")
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
fn combined_ld_sbr_resilience_22_2_matches_scalar_pcm_and_navigates() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::aac_ep1::AccessUnit;
    let m = manifest();
    let units = std::fs::read(root().join("units.bin")).unwrap();
    let plain = std::fs::read(root().join("plain.bin")).unwrap();
    assert_eq!(m["ep1"].as_array().unwrap().len(), 12);
    let mut maximum = 0.0f32;
    let mut scalars = 0usize;
    for c in m["ep1"].as_array().unwrap() {
        assert_eq!(c["instances"], if c["tool"] == "tns" { 132 } else { 108 });
        assert_eq!(c["groups"], if c["tool"] == "tns" { 66 } else { 54 });
        let rate = c["rate"].as_u64().unwrap() as u32;
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
                        timescale: rate * scale,
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
                    i128::from(got.pts) * i128::from(rate),
                    i128::from(expected.pts) * i128::from(root_clock)
                );
                assert_eq!(
                    u128::from(got.duration) * u128::from(rate),
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
            maximum = maximum.max((*v - want).abs());
            scalars += 1;
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
    eprintln!("combined LD-SBR: {scalars} scalars, max error {maximum}");
}

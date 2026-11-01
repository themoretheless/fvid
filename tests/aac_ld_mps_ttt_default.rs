use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder, aac_ld_mps_decor::Mode};
use serde_json::Value;
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-ttt-default")
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(f) = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            output.extend(pcm.data);
        }
    }
    output
}
#[test]
fn predictive_icc_defaults_match_explicit_unity_through_playback_and_seek() {
    use fvid::audio::AudioStream;
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root().join("oracle.json")).unwrap()).unwrap();
    let golden = std::fs::read(root().join("reference.f32le")).unwrap();
    let mut maximum = 0.0f32;
    let mut scalars = 0;
    for c in cases {
        let control = std::fs::read(root().join(c["control"]["video"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut expected).unwrap();
        assert_eq!(
            expected.len() as u64,
            1536 * c["channels"].as_u64().unwrap() * 4
        );
        let bytes = std::fs::read(root().join(c["default"]["video"].as_str().unwrap())).unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut actual)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(actual, expected, "{}", c["name"]);
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, expected);
        let mut matroska = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(
                std::fs::read(root().join(c["default"]["file"].as_str().unwrap())).unwrap(),
            ),
            &mut matroska,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(matroska, expected);
        for mode in [Mode::Real, Mode::Complex] {
            let mut produced = Vec::new();
            let opts = AacDecodeOptions {
                spatial_decorrelation: mode,
                ..Default::default()
            };
            let mut decoder =
                NativeAacDecoder::new_with_options(&hex(&c["asc"]), opts).unwrap();
            let mut reference = NativeAacDecoder::new_with_options(&hex(&c["asc"]), opts).unwrap();
            for i in 0..3 {
                let packet = |key: &str| {
                    let r = &c[key]["frames"][i];
                    let at = r["offset"].as_u64().unwrap() as usize;
                    &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
                };
                let want = reference
                    .decode_timed(packet("control"), i as i64 * 512, 512)
                    .unwrap();
                let checkpoint = decoder.checkpoint();
                let got = decoder
                    .decode_timed(packet("default"), i as i64 * 512, 512)
                    .unwrap();
                assert_eq!(got, want);
                if let Some(frame) = got {
                    produced.extend(frame.samples);
                }
                decoder.restore(&checkpoint).unwrap();
                assert_eq!(
                    decoder
                        .decode_timed(packet("default"), i as i64 * 512, 512)
                        .unwrap(),
                    want
                );
            }
            loop {
                let frame = decoder.finish().unwrap();
                assert_eq!(frame, reference.finish().unwrap());
                let Some(frame) = frame else {
                    break;
                };
                produced.extend(frame.samples);
            }
            let label = if mode == Mode::Real {
                "real"
            } else {
                "complex"
            };
            let saved = oracle["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["name"] == c["name"] && v["mode"] == label)
                .unwrap();
            assert_eq!(saved["default_control_byte_identical"], true);
            let at = saved["offset"].as_u64().unwrap() as usize;
            let size = saved["bytes"].as_u64().unwrap() as usize;
            assert_eq!(produced.len() * 4, size);
            for (sample, bytes) in produced.iter().zip(golden[at..at + size].chunks_exact(4)) {
                let want = f32::from_le_bytes(bytes.try_into().unwrap());
                let error = (sample - want).abs();
                assert!(
                    sample.is_finite() && want.is_finite() && error < 1e-7,
                    "{} {mode:?}: {sample} != {want}",
                    c["name"]
                );
                maximum = maximum.max(error);
                scalars += 1;
            }
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        let width = usize::from(reader.channels()) * 4;
        let mut decoder = reader.make_decoder().unwrap();
        assert_eq!(play(&mut reader, decoder.as_mut()), expected);
        reader.rewind();
        decoder.reset();
        assert_eq!(play(&mut reader, decoder.as_mut()), expected);
        for target in [13, 512, 1024, 1536, 7] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(expected.len());
            decoder.reset();
            assert_eq!(play(&mut reader, decoder.as_mut()), expected[first..]);
        }
        for (from, to) in [(1, 7), (10, 25), (1, 7)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &bytes,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = (from * 48) as usize * width;
            let last = (to * 48) as usize * width;
            assert_eq!(range, expected[first..last]);
        }
    }
    assert_eq!(scalars * 4, golden.len());
    eprintln!("predictive-default original ISO PCM: {scalars} scalars;max error {maximum}");
}

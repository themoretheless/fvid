use fvid_media::owned_aac::aac_ld_mps_decor::Mode;
use fvid_media::owned_aac::{aac_ld_mps_config::Config, bits::BitReader};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join("aac-ld-mps-arbdmx-columns")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn read(c: &Value, key: &str) -> Vec<u8> {
    std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()
}
fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn grid_change_movies_decode_identical_nonzero_pcm_in_both_containers() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut mp4 = Vec::new();
        let mut mkv = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut mp4)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(read(c, "file")),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mp4, mkv);
        assert_eq!(mp4.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
        assert!(mp4.iter().any(|&v| v != 0));
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
fn downscaled_spatial_playback_seek_and_rewind_keep_source_windows() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut full).unwrap();
        for mkv in [false, true] {
            let mut reader: Box<dyn AudioStream> = if mkv {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(read(c, "file")),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(read(c, "video")),
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
fn per_input_gains_preserve_packet_clocks_and_checkpoint_retry_in_both_modes() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        for mode in [Mode::Real, Mode::Complex] {
            let mut d = NativeAacDecoder::new_with_options(
                &hex(&c["asc"]),
                AacDecodeOptions {
                    output_sample_rate: Some(n(&c["rate"]) as u32),
                    spatial_decorrelation: mode,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(usize::from(d.channels()), n(&c["channels"]));
            assert_eq!(
                d.channel_mask(),
                match n(&c["tree"]) {
                    5 => 0xff,
                    6 => 0x63f,
                    7 => 3,
                    _ => unreachable!(),
                }
            );
            let mut out = Vec::new();
            for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = n(&r["offset"]);
                let packet = &blob[at..at + n(&r["bytes"])];
                let pts = (i * n(&c["out_n"])) as i64;
                let duration = n(&c["out_n"]) as u64;
                let state = d.checkpoint();
                let bytes = d.retained_payload_bytes().unwrap();
                assert!(
                    d.decode_timed(&packet[..packet.len() - 1], pts, duration)
                        .is_err()
                );
                assert_eq!(d.retained_payload_bytes().unwrap(), bytes);
                let frame = d.decode_timed(packet, pts, duration).unwrap();
                d.restore(&state).unwrap();
                assert_eq!(d.decode_timed(packet, pts, duration).unwrap(), frame);
                if let Some(f) = frame {
                    assert_eq!(f.pts, pts);
                    assert_eq!(f.duration, duration);
                    out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
                }
            }
            while let Some(f) = d.finish().unwrap() {
                out.extend(f.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            assert_eq!(out.len(), n(&c["samples"]) * n(&c["channels"]) * 4);
            assert!(out.iter().any(|&v| v != 0));
        }
    }
}

#[test]
fn gain_rows_are_complete_for_every_input_channel() {
    use fvid_media::owned_aac::aac_ld_mps_frame::{Decoder, ParameterId};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 378);
    assert_eq!(m["controls"].as_array().unwrap().len(), 126);
    for c in m["cases"].as_array().unwrap() {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        cfg.validate_low_delay_profile().unwrap();
        assert!(cfg.arbitrary_downmix);
        assert_eq!(usize::from(cfg.tree.input_channels), n(&c["inputs"]));

        for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
            let current = Config::parse(&hex(&c["configs"][i])).unwrap();
            assert_eq!(current.arbitrary_downmix, c["frames"][i]["enabled"] == true);
            let mut syntax = Decoder::new(&current).unwrap();
            let bytes = hex(wire);
            let mut bits = BitReader::new(&bytes);
            let f = syntax.decode(&mut bits).unwrap();
            assert_eq!(bits.remaining(), 0);
            if !current.arbitrary_downmix {
                assert!(
                    !f.parameters
                        .iter()
                        .any(|p| matches!(p.id, ParameterId::DownmixGain(_)))
                );
                continue;
            }
            for input in 0..n(&c["inputs"]) {
                let p = f
                    .parameters
                    .iter()
                    .find(|p| p.id == ParameterId::DownmixGain(input as u8))
                    .unwrap();
                let expected: Vec<Vec<i16>> =
                    serde_json::from_value(c["frames"][i]["gains"][input].clone()).unwrap();
                assert_eq!(p.values, expected);
            }
        }
    }
}
#[test]
fn per_column_endpoints_and_slots_match_independent_decimal_and_preserve_m2() {
    use fvid_media::owned_aac::aac_ld_mps_assemble::Decoder;
    let golden = std::fs::read(root().join("expected.f64le")).unwrap();
    let mut count = 0;
    let mut maximum = 0.0f64;
    for c in manifest()["cases"].as_array().unwrap() {
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let mut decoder = Decoder::new(&cfg).unwrap();
        let base = Config::parse(&hex(&c["base_ssc"])).unwrap();
        assert!(!base.arbitrary_downmix);
        let mut control = Decoder::new(&base).unwrap();
        let base_wire = hex(&c["base_reference"]);
        let inputs = n(&c["inputs"]);
        let rows = n(&c["rows"]);
        let bands = n(&c["parameter_bands"]);
        for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
            let current = Config::parse(&hex(&c["configs"][i])).unwrap();
            decoder.reconfigure(&current).unwrap();
            let bytes = hex(wire);
            let saved = decoder.clone();
            let mut short = BitReader::new(&bytes[..bytes.len() - 1]);
            assert!(decoder.decode(&mut short).is_err());
            assert_eq!(short.position(), 0);
            assert_eq!(decoder, saved);
            let frame = decoder.decode(&mut BitReader::new(&bytes)).unwrap();
            let plain = control.decode(&mut BitReader::new(&base_wire)).unwrap();
            assert_eq!(frame.dry, plain.dry);
            assert_eq!(frame.wet, plain.wet);
            for (actual, offsets) in [
                (&frame.m1.endpoints, &c["oracle"][i]["endpoints"]),
                (&frame.m1.slots, &c["oracle"][i]["slots"]),
            ] {
                assert_eq!(actual.len(), offsets.as_array().unwrap().len());
                for (matrix, offset) in actual.iter().zip(offsets.as_array().unwrap()) {
                    assert_eq!(matrix.len(), bands * 2 * rows * inputs);
                    for band in 0..bands {
                        for plane in 0..2 {
                            for row in 0..rows {
                                for col in 0..inputs {
                                    let want = if plane == 0 && col == n(&c["row_sources"][row]) {
                                        let at = n(offset) + (band * inputs + col) * 8;
                                        f64::from_le_bytes(golden[at..at + 8].try_into().unwrap())
                                    } else {
                                        0.
                                    };
                                    let value =
                                        matrix[((band * 2 + plane) * rows + row) * inputs + col];
                                    let error = (value - want).abs();
                                    maximum = maximum.max(error);
                                    assert!(
                                        error < 2e-10,
                                        "{} frame{i} band{band} plane{plane} row{row} col{col}: {value} != {want}",
                                        c["name"]
                                    );
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("ADG sparse Decimal oracle: {count} coefficients, max error {maximum}");
    assert!(count > 1_000_000);
}
#[test]
fn unity_signalling_matches_plain_control_and_varying_gains_change_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut plain = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut plain).unwrap();
        let mut actual = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut actual).unwrap();
        if c["pattern"] == "unity" {
            assert_eq!(actual, plain, "{} unity", c["name"]);
        } else {
            assert_ne!(actual, plain, "{} varying", c["name"]);
        }
    }
}

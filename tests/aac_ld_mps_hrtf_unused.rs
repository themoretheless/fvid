use fvid_media::owned_aac::aac_ld_mps_decor::Mode;
use fvid_media::owned_aac::{aac_ld_mps_config::Config, bits::BitReader};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join("aac-ld-mps-hrtf-unused")
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
    let mut controls = std::collections::HashMap::<String, Vec<u8>>::new();
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
        let expected = controls
            .entry(c["control"].as_str().unwrap().to_owned())
            .or_insert_with(|| {
                let data = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
                let mut out = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
                out
            });
        assert_eq!(&mp4, expected, "{} unused metadata changed PCM", c["name"]);
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
fn unused_metadata_keeps_packet_clocks_and_checkpoint_retry_in_both_modes() {
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
                    0 | 1 => 0x3f,
                    5 => 0xff,
                    6 => 0x63f,
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
fn complete_hrtf_metadata_and_frames_parse_for_all_unused_grid_pairs() {
    use fvid_media::owned_aac::aac_ld_mps_frame::Decoder;
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 1512);
    assert_eq!(m["controls"].as_array().unwrap().len(), 168);
    for c in m["cases"].as_array().unwrap() {
        for i in 0..3 {
            let cfg = Config::parse(&hex(&c["configs"][i])).unwrap();
            cfg.validate_low_delay_profile().unwrap();
            assert_eq!(cfg.tree.ttt_boxes, 0);
            assert_eq!(usize::from(cfg.parameter_bands), n(&c["spatial_bands"]));
            assert_eq!(
                cfg.hrtf.as_ref().map(|h| u64::from(h.bands)),
                c["hrtf_bands"][i].as_u64()
            );
            let bytes = hex(&c["reference"][i]);
            let mut bits = BitReader::new(&bytes);
            Decoder::new(&cfg).unwrap().decode(&mut bits).unwrap();
            assert_eq!(bits.remaining(), 0);
        }
    }
}
#[test]
fn unused_metadata_and_cycles_preserve_all_matrix_endpoints_and_slots() {
    use fvid_media::owned_aac::aac_ld_mps_assemble::Decoder;
    for c in manifest()["cases"].as_array().unwrap() {
        let configs: Vec<Config> = c["configs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Config::parse(&hex(v)).unwrap())
            .collect();
        let mut actual = Decoder::new(&configs[0]).unwrap();
        let mut plain_cfg = configs[0].clone();
        plain_cfg.hrtf = None;
        let mut plain = Decoder::new(&plain_cfg).unwrap();
        for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
            actual.reconfigure(&configs[i]).unwrap();
            let bytes = hex(wire);
            let saved = actual.clone();
            let mut short = BitReader::new(&bytes[..bytes.len() - 1]);
            assert!(actual.decode(&mut short).is_err());
            assert_eq!(actual, saved);
            assert_eq!(short.position(), 0);
            let a = actual.decode(&mut BitReader::new(&bytes)).unwrap();
            let b = plain.decode(&mut BitReader::new(&bytes)).unwrap();
            assert_eq!(a.m1, b.m1);
            assert_eq!(a.dry, b.dry);
            assert_eq!(a.wet, b.wet);
        }
    }
}
#[test]
fn unused_hrtf_shape_and_values_are_still_validated_atomically() {
    use fvid_media::owned_aac::aac_ld_mps_assemble::Decoder;
    let m = manifest();
    for tree in [0, 1, 5, 6] {
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| n(&c["tree"]) == tree && c["kind"] == "static")
            .unwrap();
        let cfg = Config::parse(&hex(&c["ssc"])).unwrap();
        let mut decoder = Decoder::new(&cfg).unwrap();
        let bytes = hex(&c["reference"][0]);
        decoder.decode(&mut BitReader::new(&bytes)).unwrap();
        for bad in 0..3 {
            let mut malformed = cfg.clone();
            let h = malformed.hrtf.as_mut().unwrap();
            match bad {
                0 => {
                    h.channels[0].left.pop();
                }
                1 => h.channels[0].phase.as_mut().unwrap()[0] = 64,
                _ => {
                    h.channels.pop();
                }
            };
            let saved = decoder.clone();
            let error = decoder.reconfigure(&malformed).unwrap_err().to_string();
            assert!(
                error.contains(if bad == 1 {
                    "HRTF values"
                } else {
                    "HRTF geometry"
                }),
                "{error}"
            );
            assert_eq!(decoder, saved);
        }
    }
}

#[test]
fn unused_hrtf_and_arbitrary_downmix_keep_the_same_joint_matrix_history() {
    use fvid_media::owned_aac::aac_ld_mps_assemble::Decoder;
    let current = manifest();
    let cycle = current["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "cycle")
        .unwrap();
    let hrtfs: Vec<_> = cycle["configs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Config::parse(&hex(v)).unwrap().hrtf.unwrap())
        .collect();
    let mut checked = 0;
    for corpus in ["aac-ld-mps-arbdmx", "aac-ld-mps-arbdmx-columns"] {
        let m: Value = serde_json::from_slice(
            &std::fs::read(root().parent().unwrap().join(corpus).join("manifest.json")).unwrap(),
        )
        .unwrap();
        for c in m["cases"].as_array().unwrap() {
            if corpus.ends_with("columns")
                && (c["pattern"] != "switch" || !matches!(n(&c["tree"]), 5 | 6))
            {
                continue;
            }
            let initial = Config::parse(&hex(&c["ssc"])).unwrap();
            assert!(initial.arbitrary_downmix);
            assert_eq!(initial.tree.ttt_boxes, 0);
            let mut with = initial.clone();
            with.hrtf = Some(hrtfs[0].clone());
            let mut a = Decoder::new(&with).unwrap();
            let mut b = Decoder::new(&initial).unwrap();
            for (i, wire) in c["reference"].as_array().unwrap().iter().enumerate() {
                let cfg = if c["configs"].is_array() {
                    Config::parse(&hex(&c["configs"][i])).unwrap()
                } else {
                    initial.clone()
                };
                let mut modified = cfg.clone();
                modified.hrtf = Some(hrtfs[i % 3].clone());
                a.reconfigure(&modified).unwrap();
                b.reconfigure(&cfg).unwrap();
                let bytes = hex(wire);
                let x = a.decode(&mut BitReader::new(&bytes)).unwrap();
                let y = b.decode(&mut BitReader::new(&bytes)).unwrap();
                assert_eq!(x.m1, y.m1);
                assert_eq!(x.dry, y.dry);
                assert_eq!(x.wet, y.wet);
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 144);
}

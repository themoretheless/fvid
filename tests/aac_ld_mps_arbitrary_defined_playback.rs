use fvid_media::owned_aac::aac_ld_mps_decor::Mode;
use fvid_media::owned_aac::{aac_ld_mps_config::Config, bits::BitReader};
use serde_json::Value;
use std::io::Cursor;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join("aac-ld-mps-arbitrary-defined")
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
fn changing_grids_preserve_packet_clocks_and_checkpoint_retry_in_both_modes() {
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
            assert_eq!(d.channel_mask(), 0);
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
fn every_defined_or_refused_movie_has_complete_covered_syntax() {
    use fvid_media::owned_aac::{
        aac_ld_mps_config::ExtensionValue, aac_ld_mps_frame, aac_ld_mps_prepare,
    };
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 1716);
    assert_eq!(m["errors"].as_array().unwrap().len(), 96);
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["errors"].as_array().unwrap())
    {
        for i in 0..3 {
            let cfg = Config::parse(&hex(&c["configs"][i])).unwrap();
            cfg.validate_low_delay_profile().unwrap();
            let ExtensionValue::ArbitraryTree { ott, .. } = &cfg.extensions[0].value else {
                panic!("missing tree")
            };
            assert_eq!(ott.len(), 2);
            assert_eq!(usize::from(cfg.parameter_bands), n(&c["grids"][i]));
            assert_eq!(
                ott.iter().map(|o| usize::from(o.bands)).collect::<Vec<_>>(),
                c["cutoffs"][i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(n)
                    .collect::<Vec<_>>()
            );
            let map = aac_ld_mps_prepare::qmf_map(cfg.parameter_bands, 32).unwrap();
            assert!(map.iter().all(|&b| ott.iter().all(|o| b < o.bands)));
            let wire = hex(&c["reference"][i]);
            let mut bits = BitReader::new(&wire);
            let frame = aac_ld_mps_frame::Decoder::new(&cfg)
                .unwrap()
                .decode(&mut bits)
                .unwrap();
            assert_eq!(bits.remaining(), 0);
            assert_eq!(frame.extensions[0].parameters.len(), 2);
            for (box_index, parameter) in frame.extensions[0].parameters.iter().enumerate() {
                assert_eq!(parameter.values.len(), 2);
                for (set, row) in parameter.values.iter().enumerate() {
                    assert_eq!(row.len(), usize::from(ott[box_index].bands));
                    for (band, &index) in row.iter().enumerate() {
                        assert_eq!(
                            index,
                            ((i * 11 + set * 7 + box_index * 3 + band * 5) % 31) as i16 - 15,
                            "{} CLD",
                            c["name"]
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn undefined_history_refuses_at_exact_frame_and_valid_retry_keeps_checkpoint() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let packet = |r: &Value| &blob[n(&r["offset"])..n(&r["offset"]) + n(&r["bytes"])];
    for c in m["errors"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new_with_options(
            &hex(&c["asc"]),
            AacDecodeOptions {
                output_sample_rate: Some(n(&c["rate"]) as u32),
                ..Default::default()
            },
        )
        .unwrap();
        let failed = n(&c["first_error"]);
        assert!((1..=2).contains(&failed));
        for i in 0..failed {
            d.decode(packet(&c["frames"][i])).unwrap();
        }
        let saved = d.checkpoint();
        let retained = d.retained_payload_bytes().unwrap();
        let error = d
            .decode(packet(&c["frames"][failed]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("undefined dormant gain history"),
            "{}: {error}",
            c["name"]
        );
        assert_eq!(retained, d.retained_payload_bytes().unwrap());
        let retry = hex(&c["retry"]);
        let actual = d.decode(&retry).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(actual, d.decode(&retry).unwrap());
        for mkv in [false, true] {
            let mut out = Vec::new();
            let error = if mkv {
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(read(c, "file")),
                    &mut out,
                    None,
                    &Default::default(),
                )
                .unwrap_err()
                .to_string()
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&read(c, "video"), &mut out)
                    .unwrap_err()
                    .to_string()
            };
            assert!(error.contains("undefined dormant gain history"), "{error}");
        }
    }
}

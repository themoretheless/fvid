use fvid_media::owned_aac::{
    NativeAacDecoder,
    aac_ep1::{self, AccessUnit},
    config::AudioSpecificConfig,
};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-ep1")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
fn units<'a>(blob: &'a [u8], row: &Value) -> Vec<AccessUnit<'a>> {
    row["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| AccessUnit {
            stream_id: r["stream_id"].as_u64().unwrap() as u16,
            depends_on: r["depends_on"].as_u64().map(|v| v as u16),
            data: packet(blob, r),
            bit_len: r["bits"].as_u64().unwrap() as usize,
            pts: r["pts"].as_i64().unwrap(),
            duration: r["duration"].as_u64().unwrap(),
            timescale: r["timescale"].as_u64().unwrap() as u32,
        })
        .collect()
}
fn near(name: &str, actual: &[f32], gold: &[u8]) {
    assert_eq!(actual.len() * 4, gold.len(), "{name} PCM size");
    assert!(actual.iter().all(|v| v.is_finite()));
    let worst = actual
        .iter()
        .zip(gold.chunks_exact(4))
        .map(|(a, b)| (a - f32::from_le_bytes(b.try_into().unwrap())).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-7, "{name} PCM error {worst}");
}
#[test]
fn ep1_configuration_retains_transport_mode_and_refuses_byte_packet_shortcuts() {
    let m = manifest();
    let plain = read("logical-packets.bin");
    assert_eq!(m["cases"].as_array().unwrap().len(), 96);
    assert_eq!(m["ps_cases"].as_array().unwrap().len(), 24);
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        assert!(parsed.separate_esc);
        assert!(parsed.protection.is_none());
        assert!(!parsed.direct_mapping);
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        assert!(
            d.decode(packet(&plain, &c["frames"][0]))
                .unwrap_err()
                .to_string()
                .contains("separately associated ESC")
        );
        let normal = hex(c["plain_asc"].as_str().unwrap());
        let mut normal = NativeAacDecoder::new(&normal).unwrap();
        assert!(
            normal
                .decode_esc(&[])
                .unwrap_err()
                .to_string()
                .contains("requires epConfig=1")
        );
        assert!(
            normal
                .restore(&d.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("checkpoint configuration mismatch")
        );
        assert!(d.restore(&normal.checkpoint()).is_err());
    }
}
#[test]
fn ep1_dependency_order_signed_rational_clocks_and_information_bits_are_exact() {
    let m = manifest();
    let blob = read("access-units.bin");
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["ps_cases"].as_array().unwrap())
    {
        for row in c["frames"].as_array().unwrap() {
            let mut input = units(&blob, row);
            let associated = aac_ep1::associate(&input).unwrap();
            assert_eq!(associated.pts, row["pts"].as_i64().unwrap());
            assert_eq!(associated.duration, row["duration"].as_u64().unwrap());
            assert_eq!(associated.timescale, 24000);
            let mut parent = None;
            for (i, class) in associated.payload.classes.iter().enumerate() {
                let source = input.iter().find(|u| u.depends_on == parent).unwrap();
                assert_eq!(class.class_index, i);
                assert_eq!(class.bits.len(), source.bit_len);
                assert_eq!(
                    class.bits,
                    (0..source.bit_len)
                        .map(|i| (source.data[i / 8] >> (7 - i % 8)) & 1)
                        .collect::<Vec<_>>()
                );
                parent = Some(source.stream_id);
            }
            input.reverse();
            assert_eq!(
                aac_ep1::associate(&input).unwrap().payload,
                associated.payload
            );
        }
    }
}
#[test]
fn ep1_native_and_public_decoders_match_independent_pcm_checkpoint_and_reset() {
    let m = manifest();
    let blob = read("access-units.bin");
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = hex(c["asc"].as_str().unwrap());
        let golden = read(c["golden"].as_str().unwrap());
        let mut owned = NativeAacDecoder::new(&asc).unwrap();
        let mut public = fvid::codec::aac_native::NativeAacDecoder::new(&asc).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for row in c["frames"].as_array().unwrap() {
                let input = units(&blob, row);
                let checkpoint = owned.checkpoint();
                let output = owned.decode_esc(&input).unwrap().unwrap();
                assert_eq!(output.pts, row["pts"].as_i64().unwrap());
                assert_eq!(output.duration, row["duration"].as_u64().unwrap());
                owned.restore(&checkpoint).unwrap();
                assert_eq!(
                    owned.decode_esc(&input).unwrap().unwrap().samples,
                    output.samples
                );
                let compatible = public.decode_esc(&input).unwrap().unwrap();
                assert_eq!(compatible.samples, output.samples);
                assert_eq!(compatible.pts, output.pts);
                actual.extend(output.samples);
            }
            near(name, &actual, &golden);
            owned.reset();
            public.reset();
        }
    }
}
#[test]
fn ep1_invalid_dependencies_boundaries_and_source_times_cannot_change_history() {
    let m = manifest();
    let blob = read("access-units.bin");
    for c in m["cases"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for row in c["frames"].as_array().unwrap().iter().take(4) {
            let input = units(&blob, row);
            let mut mutations = Vec::new();
            let mut wrong = input.clone();
            wrong.push(input[0]);
            mutations.push((wrong, "duplicate EP1"));
            let mut wrong = input.clone();
            wrong[0].depends_on = Some(u16::MAX);
            mutations.push((wrong, "missing EP1"));
            let leaf = input
                .iter()
                .position(|u| !input.iter().any(|v| v.depends_on == Some(u.stream_id)))
                .unwrap();
            let root = input.iter().position(|u| u.depends_on.is_none()).unwrap();
            let mut wrong = input.clone();
            wrong[leaf].depends_on = None;
            mutations.push((wrong, "multiple EP1 root"));
            let mut wrong = input.clone();
            wrong[root].depends_on = Some(wrong[leaf].stream_id);
            mutations.push((wrong, "cyclic EP1"));
            if input.len() > 2 {
                let mut wrong = input.clone();
                wrong[leaf].depends_on = Some(input[root].stream_id);
                mutations.push((wrong, "forked EP1"));
            }
            let mut wrong = input.clone();
            wrong[0].bit_len += 8;
            mutations.push((wrong, "bit boundary disagrees"));
            let mut wrong = input.clone();
            wrong[0].pts += 1;
            mutations.push((wrong, "same source time"));
            let mut wrong = input.clone();
            wrong[0].duration += 1;
            mutations.push((wrong, "same source time"));
            let mut wrong = input.clone();
            wrong[0].timescale = 0;
            mutations.push((wrong, "zero EP1"));
            mutations.push((vec![], "no ESC access units"));
            for (wrong, expected) in mutations {
                let error = d.decode_esc(&wrong).unwrap_err().to_string();
                assert!(
                    error.contains(expected),
                    "{} expected {expected}, got {error}",
                    c["name"]
                );
            }
            assert_eq!(
                d.decode_esc(&input).unwrap().unwrap().samples,
                reference.decode_esc(&input).unwrap().unwrap().samples,
                "{} history changed after refused association",
                c["name"]
            );
        }
    }
}

#[test]
fn ep1_late_sbr_syntax_failures_preserve_all_prior_synthesis_history() {
    let m = manifest();
    let blob = read("access-units.bin");
    let mut tested = 0;
    for c in m["cases"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let input = units(&blob, row);
            let category = if c["aot"] == 39 { 9 } else { 5 };
            if let Some(index) = row["units"]
                .as_array()
                .unwrap()
                .iter()
                .position(|u| u["category"] == category)
            {
                let mut data = input[index].data.to_vec();
                let expected = if category == 9 {
                    data[0] ^= 0x80;
                    "low-delay SBR CRC mismatch"
                } else {
                    data[0] = (data[0] & 15) | 0xe0;
                    "ER AAC SBR CRC is forbidden"
                };
                let mut wrong = input.clone();
                wrong[index].data = &data;
                let error = d.decode_esc(&wrong).unwrap_err().to_string();
                assert!(
                    error.contains(expected),
                    "{} expected {expected}, got {error}",
                    c["name"]
                );
                tested += 1;
            }
            assert_eq!(
                d.decode_esc(&input).unwrap().unwrap().samples,
                reference.decode_esc(&input).unwrap().unwrap().samples,
                "{} valid retry after late ESC failure",
                c["name"]
            );
        }
    }
    assert!(tested > 250, "SBR failure controls disappeared");
}

#[test]
fn ep1_ps_pcm_discovery_delayed_identity_and_checkpoint_match_scalar_core_pipeline() {
    use fvid_media::owned_aac::{
        aac_ps_native::{InBandPsProbe, NativePsAacDecoder},
        aac_sbr_dsp::OutputRate,
        aac_sbr_ps,
        bits::BitReader,
    };
    let m = manifest();
    let blob = read("access-units.bin");
    let plain = read("logical-packets.bin");
    for c in m["ps_cases"].as_array().unwrap() {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let asc = hex(c["asc"].as_str().unwrap());
        let raw = hex(c["plain_asc"].as_str().unwrap());
        let build = |asc: &[u8]| {
            if c["signal"] == "implicit" {
                NativePsAacDecoder::new_with_in_band_ps(asc, rate).unwrap()
            } else {
                NativePsAacDecoder::new(asc).unwrap()
            }
        };
        let mut d = build(&asc);
        let mut reference = build(&raw);
        let mut probe = InBandPsProbe::new(&asc, rate).unwrap();
        assert!(reference.restore(&d.checkpoint()).is_err());
        assert!(d.restore(&reference.checkpoint()).is_err());
        assert!(
            d.decode(packet(&plain, &c["frames"][0]))
                .unwrap_err()
                .to_string()
                .contains("separately associated ESC")
        );
        assert!(
            probe
                .read(packet(&plain, &c["frames"][0]))
                .unwrap_err()
                .to_string()
                .contains("separately associated ESC")
        );
        let n = c["n"].as_u64().unwrap() as usize;
        let core = read(c["core_reference"].as_str().unwrap())
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        let mut stage = aac_sbr_ps::Decoder::default();
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        let append = |out: &mut Vec<f32>, frame: aac_sbr_ps::Frame| {
            out.extend(
                frame.pcm[0]
                    .iter()
                    .zip(&frame.pcm[1])
                    .flat_map(|(l, r)| [*l as f32, *r as f32]),
            )
        };
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let input = units(&blob, row);
            let index = row["units"]
                .as_array()
                .unwrap()
                .iter()
                .position(|u| u["category"] == 5)
                .unwrap();
            let mut data = input[index].data.to_vec();
            data[0] = (data[0] & 15) | 0xe0;
            let mut wrong = input.clone();
            wrong[index].data = &data;
            for error in [
                d.decode_esc(&wrong).unwrap_err().to_string(),
                probe.read_esc(&wrong).unwrap_err().to_string(),
            ] {
                assert!(error.contains("ER AAC SBR CRC is forbidden"), "{error}");
            }
            let checkpoint = d.checkpoint();
            let frame = d.decode_esc(&input).unwrap();
            d.restore(&checkpoint).unwrap();
            assert_eq!(frame, d.decode_esc(&input).unwrap());
            assert_eq!(
                frame,
                reference.decode(packet(&plain, row)).unwrap(),
                "{} original PS frame identity",
                c["name"]
            );
            assert!(probe.read_esc(&input).unwrap());
            if let Some(frame) = frame {
                actual.extend(frame.pcm);
            }
            let payload = hex(c["payloads"][i].as_str().unwrap());
            let mut bits = BitReader::new(&payload);
            assert_eq!(bits.read(4).unwrap(), 13);
            if let Some(frame) = stage
                .read(
                    &mut bits,
                    payload.len() * 8,
                    false,
                    &core[i * n..(i + 1) * n],
                    48000,
                    (n / 64) as u8,
                    if rate == 24000 {
                        OutputRate::Core
                    } else {
                        OutputRate::Double
                    },
                )
                .unwrap()
            {
                append(&mut expected, frame);
            }
        }
        while let Some(frame) = d.finish().unwrap() {
            assert_eq!(Some(frame.clone()), reference.finish().unwrap());
            actual.extend(frame.pcm);
        }
        assert!(reference.finish().unwrap().is_none());
        assert!(d.finish().unwrap().is_none());
        while let Some(frame) = stage.finish().unwrap() {
            append(&mut expected, frame);
        }
        assert_eq!(actual.len(), expected.len());
        let worst = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            worst < 2e-6,
            "{} scalar core/component PS PCM error {worst}",
            c["name"]
        );
    }
}

#[test]
fn ep1_disconnected_cycles_large_ids_and_fractional_clocks_are_checked_before_unpacking() {
    let root = AccessUnit {
        stream_id: 0,
        depends_on: None,
        data: &[0xff],
        bit_len: 1,
        pts: i64::MIN / 3,
        duration: 17,
        timescale: 24000,
    };
    let leaf = AccessUnit {
        stream_id: u16::MAX,
        depends_on: Some(0),
        data: &[],
        bit_len: 0,
        ..root
    };
    let result = aac_ep1::associate(&[leaf, root]).unwrap();
    assert_eq!(result.payload.classes[0].bits, [1]);
    assert!(result.payload.classes[1].bits.is_empty());
    let mut cycle_a = leaf;
    cycle_a.stream_id = 3;
    cycle_a.depends_on = Some(4);
    let mut cycle_b = leaf;
    cycle_b.stream_id = 4;
    cycle_b.depends_on = Some(3);
    let error = aac_ep1::associate(&[root, cycle_a, cycle_b])
        .unwrap_err()
        .to_string();
    assert!(error.contains("disconnected or cyclic"), "{error}");
    let mut extra = root;
    extra.data = &[0xff, 0];
    assert!(
        aac_ep1::associate(&[extra])
            .unwrap_err()
            .to_string()
            .contains("bit boundary disagrees")
    );
    let root = AccessUnit {
        pts: -3,
        duration: 5,
        timescale: 7,
        ..root
    };
    let leaf = AccessUnit {
        pts: -6,
        duration: 10,
        timescale: 14,
        ..leaf
    };
    let result = aac_ep1::associate(&[leaf, root]).unwrap();
    assert_eq!((result.pts, result.duration, result.timescale), (-3, 5, 7));
    let mut wrong = leaf;
    wrong.pts = -5;
    assert!(
        aac_ep1::associate(&[root, wrong])
            .unwrap_err()
            .to_string()
            .contains("same source time")
    );
}

#[test]
fn ep1_missing_logical_aac_alignment_is_not_replaced_by_transport_padding() {
    let m = manifest();
    let blob = read("access-units.bin");
    let mut tested = 0;
    for c in m["cases"].as_array().unwrap() {
        // The no-extension profiles reach finish directly. SBR extension
        // byte geometry has its own earlier alignment validation.
        if c["name"].as_str().unwrap().starts_with("sbr-") {
            continue;
        }
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let input = units(&blob, row);
            let pad = row["alignment_bits"].as_u64().unwrap() as usize;
            if pad != 0 {
                let leaf = input
                    .iter()
                    .position(|u| !input.iter().any(|v| v.depends_on == Some(u.stream_id)))
                    .unwrap();
                let mut wrong = input.clone();
                wrong[leaf].bit_len -= pad;
                wrong[leaf].data = &wrong[leaf].data[..wrong[leaf].bit_len.div_ceil(8)];
                let error = match d.decode_esc(&wrong) {
                    Err(error) => error.to_string(),
                    Ok(_) => panic!("{} accepted missing logical AAC alignment", c["name"]),
                };
                assert!(
                    error.contains("incomplete AAC ESC alignment bits"),
                    "{}: {error}",
                    c["name"]
                );
                tested += 1;
            }
            assert_eq!(
                d.decode_esc(&input).unwrap().unwrap().samples,
                reference.decode_esc(&input).unwrap().unwrap().samples
            );
        }
    }
    assert!(tested > 20);
}

#[test]
fn protected_ep3_alignment_movies_reproduce_the_same_error_after_valid_fec_recovery() {
    let m = manifest();
    let blob = read("ep3-alignment-packets.bin");
    assert_eq!(m["ep3_alignment"].as_array().unwrap().len(), 8);
    for c in m["ep3_alignment"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let ep = parsed.protection.as_ref().unwrap();
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        let mut reference = NativeAacDecoder::new(&asc).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            if i == c["bad_index"].as_u64().unwrap() as usize {
                let bad = ep.decode_class_frames(packet(&blob, row)).unwrap();
                let start = row["good_offset"].as_u64().unwrap() as usize;
                let good = &blob[start..start + row["good_bytes"].as_u64().unwrap() as usize];
                let good = ep.decode_class_frames(good).unwrap();
                assert_eq!(bad[0].classes.len(), good[0].classes.len());
                let total = |classes: &[_]| -> usize {
                    let classes: &[fvid_media::owned_aac::aac_ep::RecoveredClass] = classes;
                    classes.iter().map(|p| p.bits.len()).sum()
                };
                assert_eq!(
                    total(&good[0].classes) - total(&bad[0].classes),
                    row["alignment_bits"].as_u64().unwrap() as usize
                );
                let error = d.decode(packet(&blob, row)).unwrap_err().to_string();
                assert!(
                    error.contains("incomplete AAC ESC alignment bits"),
                    "{}: {error}",
                    c["name"]
                );
            }
            let start = row["good_offset"].as_u64().unwrap() as usize;
            let good = &blob[start..start + row["good_bytes"].as_u64().unwrap() as usize];
            assert_eq!(d.decode(good).unwrap(), reference.decode(good).unwrap());
        }
        for filename in ["video", "matroska"] {
            let data = read(c[filename].as_str().unwrap());
            let mut out = Vec::new();
            let error = if filename == "video" {
                fvid::native_media::decode_mp4_aac_pcm(&data, &mut out)
            } else {
                fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut out, None)
            }
            .unwrap_err()
            .to_string();
            assert!(
                error.contains("incomplete AAC ESC alignment bits"),
                "{} {filename}: {error}",
                c["name"]
            );
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn missing_logical_alignment_movies_fail_at_the_specific_boundary_in_the_player() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m = manifest();
    for c in m["ep3_alignment"].as_array().unwrap() {
        for mp4 in [true, false] {
            let data = read(c[if mp4 { "video" } else { "matroska" }].as_str().unwrap());
            let result = (|| -> Result<(), String> {
                let mut reader: Box<dyn AudioStream> = if mp4 {
                    Box::new(
                        fvid::playback_mp4_audio::Mp4AudioReader::open(
                            Cursor::new(data),
                            Default::default(),
                        )
                        .map_err(|e| e.to_string())?,
                    )
                } else {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(
                            Cursor::new(data),
                            Default::default(),
                        )
                        .map_err(|e| e.to_string())?,
                    )
                };
                let mut decoder = reader.make_decoder().map_err(|e| e.to_string())?;
                while let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? {
                    if let Some(frame) = decoder
                        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                        .map_err(|e| e.to_string())?
                    {
                        reader
                            .present_decoded(frame.packet, frame.source_pts)
                            .map_err(|e| e.to_string())?;
                    }
                }
                while let Some(frame) = decoder.finish_packet().map_err(|e| e.to_string())? {
                    reader
                        .present_decoded(frame.packet, frame.source_pts)
                        .map_err(|e| e.to_string())?;
                }
                reader.validate_eof().map_err(|e| e.to_string())?;
                Ok(())
            })();
            let error = result.unwrap_err();
            assert!(
                error.contains("incomplete AAC ESC alignment bits"),
                "{} player: {error}",
                c["name"]
            );
        }
    }
}

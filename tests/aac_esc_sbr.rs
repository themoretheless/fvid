use fvid::codec::aac_native::NativeAacDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-esc-sbr")
}
#[test]
fn esc_sbr_matches_independent_scalar_qmf_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let golden: Vec<f32> = std::fs::read(root().join(c["golden"].as_str().unwrap()))
            .unwrap()
            .chunks_exact(4)
            .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
            .collect();
        let mut d = NativeAacDecoder::new(&asc).unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut actual = Vec::new();
            for row in c["frames"].as_array().unwrap() {
                let a = row["offset"].as_u64().unwrap() as usize;
                let b = a + row["bytes"].as_u64().unwrap() as usize;
                actual.extend(d.decode(&packets[a..b]).unwrap());
            }
            assert_eq!(actual.len(), golden.len());
            assert!(actual.iter().all(|v| v.is_finite()));
            let worst = actual
                .iter()
                .zip(&golden)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(worst < 1e-7, "{name}: PCM error {worst}");
            assert!(actual.iter().any(|&v| v.abs() > 1e-5));
            d.reset();
        }
    }
}

#[test]
fn esc_sbr_videos_decode_through_public_and_owned_mp4_audio_paths() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut public = Vec::new();
        let mut owned = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(public, owned);
        let golden = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(public.len(), golden.len());
        let worst = public
            .chunks_exact(4)
            .zip(golden.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(worst < 1e-7, "{name}: public PCM error {worst}");
    }
}

#[test]
fn esc_sbr_matroska_and_sample_ranges_preserve_complete_pcm() {
    use std::{io::Cursor, time::Duration};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video, &mut full).unwrap();
        let mut decoded = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut decoded, None)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(decoded, full, "{name} Matroska PCM");
        let mut owned = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full);
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(7u64, 35u64), (2, 19), (7, 35)] {
            let interval = Some((Duration::from_millis(from), Duration::from_millis(to)));
            let first = (from * rate).div_ceil(1000) as usize;
            let last = (to * rate).div_ceil(1000) as usize;
            let end = (last * width).min(full.len());
            let begin = (first * width).min(end);
            let mut out = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(&video, &mut out, interval).unwrap();
            assert_eq!(out, &full[begin..end], "{name} MP4 range");
            out.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, interval).unwrap();
            assert_eq!(out, &full[begin..end], "{name} Matroska range");
        }
    }
}

#[cfg(feature = "player")]
fn try_play(reader: &mut dyn fvid::audio::AudioStream) -> Result<Vec<u8>, String> {
    let mut decoder = reader.make_decoder().map_err(|e| e.to_string())?;
    let mut output = vec![];
    while let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .map_err(|e| e.to_string())?
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .map_err(|e| e.to_string())?
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().map_err(|e| e.to_string())? {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .map_err(|e| e.to_string())?
        {
            output.extend(pcm.data);
        }
    }
    assert!(
        decoder
            .finish_packet()
            .map_err(|e| e.to_string())?
            .is_none()
    );
    reader.validate_eof().map_err(|e| e.to_string())?;
    Ok(output)
}
#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    try_play(reader).unwrap()
}

#[cfg(feature = "player")]
#[test]
fn esc_sbr_player_rewind_and_sample_seek_preserve_pcm_in_both_containers() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["ps_cases"].as_array().unwrap())
        .chain(m["multiple"].as_array().unwrap())
        .chain(m["multi_sbr"].as_array().unwrap())
    {
        let name = c["name"].as_str().unwrap();
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let mut readers: Vec<Box<dyn AudioStream>> = vec![
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
        for (container, reader) in readers.iter_mut().enumerate() {
            assert_eq!(play(reader.as_mut()), full, "{name} container {container}");
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full, "{name} rewind {container}");
            // Repeated targets and returning from EOF must rebuild the same
            // AAC filterbank history as an uninterrupted decode, including frame edges.
            let source_samples = c["n"].as_i64().unwrap() * c["rate"].as_i64().unwrap() / 24000;
            for target in [
                -1,
                0,
                13,
                512,
                2500,
                source_samples - 1,
                source_samples,
                source_samples + 1,
                7,
                7,
                c["container_frame_samples"].as_i64().unwrap() - 1,
                c["container_frame_samples"].as_i64().unwrap(),
                c["container_frame_samples"].as_i64().unwrap() + 1,
                (full.len() / width) as i64,
                (full.len() / width) as i64 + 1,
                13,
                0,
            ] {
                let ticks = (i128::from(target) * i128::from(reader.timescale())
                    / i128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                assert!(landed >= 0);
                if target <= (full.len() / width) as i64 {
                    assert_eq!(
                        landed,
                        ticks.max(0),
                        "{name} seek landed at a different target"
                    );
                }
                assert_eq!(
                    play(reader.as_mut()),
                    full[(((landed as u128 * u128::from(reader.sample_rate()))
                        .div_ceil(u128::from(reader.timescale()))
                        as usize)
                        * width)
                        .min(full.len())..],
                    "{name} container {container} seek {target} landed {landed}"
                );
            }
        }
    }
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &serde_json::Value, prefix: &str) -> &'a [u8] {
    let a = row[format!("{prefix}offset")].as_u64().unwrap() as usize;
    &blob[a..a + row[format!("{prefix}bytes")].as_u64().unwrap() as usize]
}
#[test]
fn malformed_sbr_classes_refuse_specific_failure_and_restore_decoder_history() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = hex(c["asc"].as_str().unwrap());
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        let mut reference = NativeAacDecoder::new(&asc).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let checkpoint = d.checkpoint();
            if i == c["bad_index"].as_u64().unwrap() as usize {
                let error = d.decode(packet(&blob, row, "")).unwrap_err().to_string();
                let expected = match c["variant"].as_str().unwrap() {
                    "short-sbr" => "truncated SBR grid",
                    "spill" => "unconsumed ER SBR ESC information bits",
                    "missing-header" => "missing initial SBR header",
                    "crc" => "ER AAC SBR CRC is forbidden",
                    "missing" => "missing ER AAC SBR element",
                    "excess" => "excess ER AAC SBR element",
                    "order" => "ER AAC extensions must precede SBR",
                    variant => panic!("unqualified malformed variant {variant}"),
                };
                assert!(
                    error.contains(expected),
                    "{name}: expected {expected}, got {error}"
                );
            }
            let actual = d.decode(packet(&blob, row, "good_")).unwrap();
            let expected = reference.decode(packet(&blob, row, "good_")).unwrap();
            assert_eq!(actual, expected, "{name} rollback");
            d.restore(&checkpoint).unwrap();
            assert_eq!(
                d.decode(packet(&blob, row, "good_")).unwrap(),
                expected,
                "{name} checkpoint"
            );
        }
    }
}

#[test]
fn reconstructed_sbr_class_bits_require_current_per_element_header_history() {
    use fvid_media::owned_aac::{
        aac_esc, aac_sbr_extension::State, bits::BitReader, config::AudioSpecificConfig,
    };
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    let info = std::fs::read(root().join("instances.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let config = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let mut states = vec![State::default(); channels];
        let kinds: &[u8] = match c["configuration"].as_u64().unwrap() {
            1 => &[0],
            2 => &[1],
            3 => &[0, 1],
            4 => &[0, 1, 0],
            5 => &[0, 1, 1],
            6 => &[0, 1, 1, 3],
            7 | 12 => &[0, 1, 1, 1, 3],
            11 => &[0, 1, 1, 0, 3],
            14 => &[0, 1, 1, 3, 1],
            _ => unreachable!(),
        };
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let frames = config
                .protection
                .as_ref()
                .unwrap()
                .decode_class_frames(packet(&blob, row, ""))
                .unwrap();
            let frame = &frames[0];
            let headers = states
                .iter()
                .map(|state| state.current_header().cloned())
                .collect::<Vec<_>>();
            let rebuilt = aac_esc::reassemble_er_with_sbr(
                &config.core,
                config.program.as_ref(),
                frame,
                &headers,
            )
            .unwrap();
            assert_eq!(
                rebuilt,
                packet(&plain, row, "plain_"),
                "{} source {index}",
                c["name"]
            );
            if index == 1 {
                let error = aac_esc::reassemble_er(&config.core, config.program.as_ref(), frame)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("missing initial SBR header"), "{error}");
            }
            let instances = row["instances"].as_array().unwrap();
            let core = instances
                .iter()
                .take_while(|p| p["category"] != 5)
                .map(|p| p["bits"].as_u64().unwrap() as usize)
                .sum::<usize>();
            let mut offset = 0;
            let mut group = 0;
            for &kind in kinds {
                let width = if kind == 1 { 2 } else { 1 };
                if kind != 3 {
                    let (at, part) = instances
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p["category"] == 9)
                        .nth(group)
                        .unwrap();
                    let body = packet(&info, part, "");
                    let length = part["bits"].as_u64().unwrap() as usize
                        - if at + 1 == instances.len() {
                            (8 - core % 8) % 8
                        } else {
                            0
                        };
                    let mut bits = BitReader::with_bit_len(body, length).unwrap();
                    states[offset]
                        .read(
                            &mut bits,
                            length,
                            false,
                            48000,
                            (config.core.frame_samples / 64) as u8,
                            width,
                        )
                        .unwrap();
                    assert_eq!(bits.position(), length);
                    group += 1;
                }
                offset += width;
            }
        }
    }
}

#[test]
fn ps_sbr_categories_match_independent_core_pipeline_and_discovery() {
    use fvid_media::owned_aac::{
        aac_ps_native::{InBandPsProbe, NativePsAacDecoder},
        aac_sbr_dsp::OutputRate,
        aac_sbr_ps,
        bits::BitReader,
    };
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["ps_cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap() as u32;
        let build = || {
            if c["signal"] == "implicit" {
                NativePsAacDecoder::new_with_in_band_ps(&asc, rate).unwrap()
            } else {
                NativePsAacDecoder::new(&asc).unwrap()
            }
        };
        let mut d = build();
        let mut probe = InBandPsProbe::new(&asc, rate).unwrap();
        let core = std::fs::read(root().join(c["core_reference"].as_str().unwrap()))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        let n = c["n"].as_u64().unwrap() as usize;
        let mut stage = aac_sbr_ps::Decoder::default();
        let mut expected = Vec::new();
        let mut actual = Vec::new();
        let append = |out: &mut Vec<f32>, frame: aac_sbr_ps::Frame| {
            out.extend(
                frame.pcm[0]
                    .iter()
                    .zip(&frame.pcm[1])
                    .flat_map(|(l, r)| [*l as f32, *r as f32]),
            );
        };
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let raw = hex(c["payloads"][i].as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            assert_eq!(bits.read(4).unwrap(), 13);
            if let Some(frame) = stage
                .read(
                    &mut bits,
                    raw.len() * 8,
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
            let wire = packet(&blob, row, "");
            assert!(probe.read(wire).unwrap(), "{} PS discovery", c["name"]);
            let checkpoint = d.checkpoint();
            let output = d.decode(wire).unwrap();
            d.restore(&checkpoint).unwrap();
            assert_eq!(output, d.decode(wire).unwrap());
            if let Some(frame) = output {
                actual.extend(frame.pcm);
            }
        }
        while let Some(frame) = stage.finish().unwrap() {
            append(&mut expected, frame);
        }
        while let Some(frame) = d.finish().unwrap() {
            actual.extend(frame.pcm);
        }
        assert_eq!(actual.len(), expected.len());
        let worst = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            worst < 2e-6,
            "{} independent core/PS pipeline error {worst}",
            c["name"]
        );
        let mut mp4 = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut mp4,
        )
        .unwrap();
        let mut mkv = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap(),
            &mut mkv,
            None,
        )
        .unwrap();
        assert_eq!(mp4, mkv, "{} PS containers", c["name"]);
        assert_eq!(
            mp4,
            actual
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn multi_source_ps_updates_sbr_context_before_each_child_and_preserves_eof() {
    use fvid_media::owned_aac::{
        aac_esc,
        aac_ps_native::{InBandPsProbe, NativePsAacDecoder},
        config::AudioSpecificConfig,
    };
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    for c in m["multiple"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let raw_asc = hex(c["plain_asc"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut d = NativePsAacDecoder::new_with_in_band_ps(&asc, rate).unwrap();
        let mut reference = NativePsAacDecoder::new_with_in_band_ps(&raw_asc, rate).unwrap();
        let mut probe = InBandPsProbe::new(&asc, rate).unwrap();
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        let first = &c["frames"][0];
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        let recovered = config
            .protection
            .as_ref()
            .unwrap()
            .decode_class_frames(packet(&blob, first, ""))
            .unwrap();
        assert!(aac_esc::reassemble_er(&config.core, None, &recovered[0]).is_ok());
        let stale = aac_esc::reassemble_er(&config.core, None, &recovered[1])
            .unwrap_err()
            .to_string();
        assert!(stale.contains("missing initial SBR header"), "{stale}");
        for row in c["frames"].as_array().unwrap() {
            let bad = packet(&blob, row, "bad_");
            let error = d.decode(bad).unwrap_err().to_string();
            assert!(error.contains("ER AAC SBR CRC is forbidden"), "{error}");
            let error = probe.read(bad).unwrap_err().to_string();
            assert!(error.contains("ER AAC SBR CRC is forbidden"), "{error}");
            assert!(probe.read(packet(&blob, row, "")).unwrap());
            let checkpoint = d.checkpoint();
            let frame = d.decode(packet(&blob, row, "")).unwrap();
            d.restore(&checkpoint).unwrap();
            assert_eq!(frame, d.decode(packet(&blob, row, "")).unwrap());
            if let Some(frame) = frame {
                actual.extend(frame.pcm);
            }
            for unit in row["units"].as_array().unwrap() {
                if let Some(frame) = reference.decode(packet(&plain, unit, "")).unwrap() {
                    expected.extend(frame.pcm);
                }
            }
        }
        while let Some(frame) = d.finish().unwrap() {
            actual.extend(frame.pcm);
        }
        while let Some(frame) = reference.finish().unwrap() {
            expected.extend(frame.pcm);
        }
        assert_eq!(
            actual, expected,
            "{} multi-source source history",
            c["name"]
        );
        assert_eq!(actual.len(), c["samples"].as_u64().unwrap() as usize * 2);
        assert!(d.finish().unwrap().is_none());
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            output,
            actual
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            "{} MP4",
            c["name"]
        );
    }
}

#[test]
fn multi_source_standard_sbr_rebuilds_each_child_and_rolls_back_late_failure() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    for c in m["multi_sbr"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut reference = NativeAacDecoder::new(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        let mut actual = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let checkpoint = d.checkpoint();
            let error = d
                .decode(packet(&blob, row, "bad_"))
                .unwrap_err()
                .to_string();
            assert!(error.contains("ER AAC SBR CRC is forbidden"), "{error}");
            let pcm = d.decode(packet(&blob, row, "")).unwrap();
            let mut expected = Vec::new();
            for unit in row["units"].as_array().unwrap() {
                expected.extend(reference.decode(packet(&plain, unit, "")).unwrap());
            }
            assert_eq!(pcm, expected, "{} per-source history", c["name"]);
            d.restore(&checkpoint).unwrap();
            assert_eq!(d.decode(packet(&blob, row, "")).unwrap(), expected);
            actual.extend(pcm);
        }
        assert_eq!(actual.len(), c["samples"].as_u64().unwrap() as usize);
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap(),
            &mut output,
        )
        .unwrap();
        assert_eq!(
            output,
            actual
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        for filename in ["bad_video", "bad_matroska"] {
            let data = std::fs::read(root().join(c[filename].as_str().unwrap())).unwrap();
            let mut out = Vec::new();
            let error = if filename == "bad_video" {
                fvid::native_media::decode_mp4_aac_pcm(&data, &mut out)
            } else {
                fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut out, None)
            }
            .unwrap_err()
            .to_string();
            assert!(error.contains("ER AAC SBR CRC is forbidden"), "{error}");
        }
    }
}

#[test]
fn malformed_sbr_movies_report_specific_class_syntax_failures_in_both_containers() {
    let m = manifest();
    for c in m["malformed"].as_array().unwrap() {
        let expected = match c["variant"].as_str().unwrap() {
            "short-sbr" => "truncated SBR grid",
            "spill" => "unconsumed ER SBR ESC information bits",
            "missing-header" => "missing initial SBR header",
            "crc" => "ER AAC SBR CRC is forbidden",
            "missing" => "missing ER AAC SBR element",
            "excess" => "excess ER AAC SBR element",
            "order" => "ER AAC extensions must precede SBR",
            _ => unreachable!(),
        };
        for filename in ["video", "matroska"] {
            let data = std::fs::read(root().join(c[filename].as_str().unwrap())).unwrap();
            let mut out = Vec::new();
            let error = if filename == "video" {
                fvid::native_media::decode_mp4_aac_pcm(&data, &mut out)
            } else {
                fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut out, None)
            }
            .unwrap_err()
            .to_string();
            assert!(
                error.contains(expected),
                "{} {filename}: expected {expected}, got {error}",
                c["name"]
            );
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn malformed_single_and_late_child_movies_keep_the_specific_error_through_player() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m = manifest();
    for key in ["malformed", "multiple", "multi_sbr"] {
        for c in m[key].as_array().unwrap() {
            let expected = if key != "malformed" {
                "ER AAC SBR CRC is forbidden"
            } else {
                match c["variant"].as_str().unwrap() {
                    "short-sbr" => "truncated SBR grid",
                    "spill" => "unconsumed ER SBR ESC information bits",
                    "missing-header" => "missing initial SBR header",
                    "crc" => "ER AAC SBR CRC is forbidden",
                    "missing" => "missing ER AAC SBR element",
                    "excess" => "excess ER AAC SBR element",
                    "order" => "ER AAC extensions must precede SBR",
                    _ => unreachable!(),
                }
            };
            for mp4 in [true, false] {
                let filename = if key == "malformed" {
                    if mp4 { "video" } else { "matroska" }
                } else if mp4 {
                    "bad_video"
                } else {
                    "bad_matroska"
                };
                let data = std::fs::read(root().join(c[filename].as_str().unwrap())).unwrap();
                let result = (|| -> Result<Vec<u8>, String> {
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
                    try_play(reader.as_mut())
                })();
                let error = result.unwrap_err();
                assert!(
                    error.contains(expected),
                    "{} {filename}: expected {expected}, got {error}",
                    c["name"]
                );
            }
        }
    }
}

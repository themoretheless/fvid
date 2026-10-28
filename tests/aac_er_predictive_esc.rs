use fvid::codec::aac_native::NativeAacDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-er-predictive-esc")
}
#[test]
fn er_predictive_esc_nonzero_core_matches_independent_pcm() {
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
fn er_predictive_esc_videos_decode_through_public_and_owned_mp4_audio_paths() {
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
fn er_predictive_esc_matroska_and_sample_ranges_preserve_complete_pcm() {
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
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    reader.validate_eof().unwrap();
    output
}
#[cfg(feature = "player")]
#[test]
fn er_predictive_esc_player_rewind_and_sample_seek_preserve_pcm_in_both_containers() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
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
            for target in [
                -1,
                0,
                13,
                512,
                2500,
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
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &serde_json::Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn er_predictive_esc_exact_source_bits_and_class_boundaries() {
    use fvid_media::owned_aac::{aac_esc::reassemble_er, config::AudioSpecificConfig};
    let blob = read("packets.bin");
    let plain = read("plain-packets.bin");
    let information = read("instances.bin");
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 160);
    for c in m["cases"].as_array().unwrap() {
        let config = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        assert!(config.direct_mapping);
        assert_eq!(
            u64::from(config.core.object_type),
            c["aot"].as_u64().unwrap()
        );
        let flags = c["flags"].as_u64().unwrap();
        assert_eq!(config.core.section_data_resilience, flags & 4 != 0);
        assert_eq!(config.core.scalefactor_data_resilience, flags & 2 != 0);
        assert_eq!(config.core.spectral_data_resilience, flags & 1 != 0);
        let ep = config.protection.as_ref().unwrap();
        for row in c["frames"].as_array().unwrap() {
            let frames = ep.decode_class_frames(packet(&blob, row)).unwrap();
            assert_eq!(frames.len(), 1);
            let frame = &frames[0];
            assert_eq!(frame.set_index, row["choice"].as_u64().unwrap() as usize);
            for (index, (class, item)) in frame
                .classes
                .iter()
                .zip(row["instances"].as_array().unwrap())
                .enumerate()
            {
                assert_eq!(class.class_index, index);
                let a = item["offset"].as_u64().unwrap() as usize;
                let n = item["bits"].as_u64().unwrap() as usize;
                assert_eq!(
                    class.bits,
                    (0..n)
                        .map(|i| (information[a + i / 8] >> (7 - i % 8)) & 1)
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(
                frame.classes.len(),
                row["instances"].as_array().unwrap().len()
            );
            let a = row["plain_offset"].as_u64().unwrap() as usize;
            let b = a + row["plain_bytes"].as_u64().unwrap() as usize;
            assert_eq!(
                reassemble_er(&config.core, config.program.as_ref(), frame).unwrap(),
                plain[a..b],
                "{}",
                c["name"]
            );
        }
    }
}
#[test]
fn er_predictive_ep3_checkpoint_identity_reserved_mapping_and_replay() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let blob = read("packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = Owned::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let ep2 = Owned::new(&hex(c["ep2_asc"].as_str().unwrap())).unwrap();
        assert!(
            d.restore(&ep2.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("EP checkpoint configuration mismatch")
        );
        assert!(
            AudioSpecificConfig::parse(&hex(c["reserved_asc"].as_str().unwrap()))
                .unwrap_err()
                .to_string()
                .contains("reserved AAC EP3 directMapping")
        );
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            assert!(
                d.retained_payload_bytes_with_checkpoint(Some(&saved))
                    .unwrap()
                    > d.retained_payload_bytes().unwrap()
            );
            let raw = packet(&blob, row);
            let actual = d.decode(raw).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(actual, d.decode(raw).unwrap());
        }
    }
}
#[test]
fn er_predictive_ep3_malformed_categories_preserve_specific_error_and_history() {
    use fvid_media::owned_aac::NativeAacDecoder as Owned;
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["malformed"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["name"] == c["good"])
            .unwrap();
        let mut d = Owned::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        assert_eq!(c["asc"], good["asc"]);
        let index = c["bad_index"].as_u64().unwrap() as usize;
        for row in &c["frames"].as_array().unwrap()[..index] {
            d.decode(packet(&blob, row)).unwrap();
        }
        let saved = d.checkpoint();
        let error = d
            .decode(packet(&blob, &c["frames"][index]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        let actual = d.decode(packet(&blob, &good["frames"][index])).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(
            actual,
            d.decode(packet(&blob, &good["frames"][index])).unwrap()
        );
        let mp4 = read(c["video"].as_str().unwrap());
        let mkv = read(c["matroska"].as_str().unwrap());
        for error in [
            fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut vec![])
                .unwrap_err()
                .to_string(),
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut vec![], None)
                .unwrap_err()
                .to_string(),
        ] {
            assert!(
                error.contains(c["error"].as_str().unwrap()),
                "{} export: {error}",
                c["name"]
            );
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn malformed_er_predictive_ep3_keep_exact_errors_in_exports_and_player_bridges() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        for error in [
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&mp4),
                &mut vec![],
                None,
                &Default::default(),
            )
            .unwrap_err()
            .to_string(),
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut vec![], None)
                .unwrap_err()
                .to_string(),
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&mkv),
                &mut vec![],
                None,
                &Default::default(),
            )
            .unwrap_err()
            .to_string(),
        ] {
            assert!(
                error.contains(c["error"].as_str().unwrap()),
                "{}: {error}",
                c["name"]
            );
        }
        let readers: Vec<Result<Box<dyn AudioStream>, String>> = vec![
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(mp4), Default::default())
                .map(|r| Box::new(r) as Box<dyn AudioStream>)
                .map_err(|e| e.to_string()),
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(mkv), Default::default())
                .map(|r| Box::new(r) as Box<dyn AudioStream>)
                .map_err(|e| e.to_string()),
        ];
        for opened in readers {
            // Mono discovery validates candidate PS syntax while opening.
            // Other layouts reach the same failure during packet decoding.
            let mut reader = match opened {
                Ok(reader) => reader,
                Err(error) => {
                    assert_eq!(c["channels"], 1);
                    assert!(
                        error.contains(c["error"].as_str().unwrap()),
                        "{} opening: {error}",
                        c["name"]
                    );
                    continue;
                }
            };
            let mut decoder = reader.make_decoder().unwrap();
            let mut decoded = 0;
            let error = loop {
                let p = reader
                    .next_packet()
                    .unwrap()
                    .expect("missing expected EP3 failure");
                match decoder.decode_packet(&p.data, p.pts, p.duration as u64) {
                    Err(error) => break error.to_string(),
                    Ok(Some(frame)) => {
                        if reader
                            .present_decoded(frame.packet, frame.source_pts)
                            .unwrap()
                            .is_some()
                        {
                            decoded += 1;
                        }
                    }
                    Ok(None) => {}
                }
            };
            assert_eq!(decoded, c["bad_index"].as_u64().unwrap());
            assert!(
                error.contains(c["error"].as_str().unwrap()),
                "{} player: {error}",
                c["name"]
            );
        }
    }
}

#[test]
fn er_predictive_ep3_discovery_and_ps_candidates_keep_semantic_mapping_and_checkpoint_identity() {
    use fvid_media::owned_aac::aac_ps_native::{InBandPsProbe, NativePsAacDecoder};
    let m = manifest();
    let blob = read("packets.bin");
    let mut checked = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["channels"] == 1 && c["aot"] == 19)
    {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut probe = InBandPsProbe::new(&asc, 24000).unwrap();
        assert!(probe.requires_ep_recovery());
        for _ in 0..2 {
            for row in c["frames"].as_array().unwrap() {
                assert!(!probe.read(packet(&blob, row)).unwrap());
                assert!(!probe.ps_detected());
            }
            probe.reset();
        }
        let mut ps = NativePsAacDecoder::new_with_in_band_ps(&asc, 24000).unwrap();
        let ep2 =
            NativePsAacDecoder::new_with_in_band_ps(&hex(c["ep2_asc"].as_str().unwrap()), 24000)
                .unwrap();
        assert!(
            ps.restore(&ep2.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("checkpoint configuration mismatch")
        );
        for bad in m["malformed"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|bad| bad["good"] == c["name"])
        {
            let index = bad["bad_index"].as_u64().unwrap() as usize;
            for row in &bad["frames"].as_array().unwrap()[..index] {
                assert!(!probe.read(packet(&blob, row)).unwrap());
            }
            let raw = packet(&blob, &bad["frames"][index]);
            for error in [
                probe.read(raw).unwrap_err().to_string(),
                ps.decode(raw).unwrap_err().to_string(),
            ] {
                assert!(
                    error.contains(bad["error"].as_str().unwrap()),
                    "{} candidate: {error}",
                    bad["name"]
                );
            }
            assert!(!probe.read(packet(&blob, &c["frames"][index])).unwrap());
            assert!(!probe.ps_detected());
        }
        checked += 1;
    }
    assert_eq!(checked, 18);
}

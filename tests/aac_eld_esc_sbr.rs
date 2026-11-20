use fvid::codec::aac_native::NativeAacDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-eld-esc-sbr")
}
#[test]
fn ep3_ld_sbr_nonzero_core_and_high_bands_match_independent_complete_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let golden: Vec<f32> = std::fs::read(root().join(format!("{name}.f32le")))
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
fn ep3_ld_sbr_videos_decode_through_public_and_owned_mp4_audio_paths() {
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
        let golden = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
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
fn ep3_ld_sbr_matroska_and_sample_ranges_preserve_complete_pcm() {
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
fn ep3_ld_sbr_player_rewind_and_sample_seek_preserve_pcm_in_both_containers() {
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
            // LD-SBR history as an uninterrupted decode, including frame edges.
            for target in [
                -1,
                -c["container_frame_samples"].as_i64().unwrap(),
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
                (full.len() / width) as i64 * 2,
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

#[test]
fn ep3_ld_sbr_checkpoint_reset_and_malformed_frame_rollback() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert!(config.direct_mapping);
        assert_eq!(config.sbr_present, Some(true));
        assert_eq!(config.ps_present, Some(false));
        if let Some(flags) = c["resilience"].as_u64() {
            assert_eq!(
                config.core.section_data_resilience,
                flags & 4 != 0,
                "{name}"
            );
            assert_eq!(
                config.core.scalefactor_data_resilience,
                flags & 2 != 0,
                "{name}"
            );
            assert_eq!(
                config.core.spectral_data_resilience,
                flags & 1 != 0,
                "{name}"
            );
        }
        assert_eq!(
            config
                .eld_sbr
                .as_ref()
                .unwrap()
                .output_rate(config.core.sample_rate)
                .unwrap(),
            c["rate"].as_u64().unwrap() as u32
        );
        let mut d = Owned::new(&asc).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let a = row["offset"].as_u64().unwrap() as usize;
            let raw = &packets[a..a + row["bytes"].as_u64().unwrap() as usize];
            let checkpoint = d.checkpoint();
            let retained = d
                .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                .unwrap();
            assert!(retained > d.retained_payload_bytes().unwrap());
            if i == 3 {
                assert!(
                    d.decode(&raw[..raw.len() - 1]).is_err(),
                    "{name} truncated LD-SBR accepted"
                );
            }
            let pts = -37 + i as i64 * 83;
            let duration = 31 + i as u64;
            let actual = d.decode_timed(raw, pts, duration).unwrap().unwrap();
            assert_eq!(actual.pts, pts, "{name} signed PTS");
            assert_eq!(actual.duration, duration, "{name} packet duration");
            assert_eq!(
                actual.samples.len(),
                c["container_frame_samples"].as_u64().unwrap() as usize
                    * config.core.channels as usize
            );
            d.restore(&checkpoint).unwrap();
            let replay = d.decode_timed(raw, pts, duration).unwrap().unwrap();
            assert_eq!(actual.pts, replay.pts, "{name} checkpoint PTS");
            assert_eq!(
                actual.duration, replay.duration,
                "{name} checkpoint duration"
            );
            assert_eq!(actual.samples, replay.samples, "{name} checkpoint PCM");
        }
    }
}

#[test]
fn ep3_ld_sbr_own_malformed_videos_fail_at_the_specific_ld_sbr_block() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        let rows = c["frames"].as_array().unwrap();
        let index = c["bad_index"].as_u64().unwrap() as usize;
        for row in &rows[..index] {
            let a = row["offset"].as_u64().unwrap() as usize;
            d.decode(&packets[a..a + row["bytes"].as_u64().unwrap() as usize])
                .unwrap();
        }
        let row = &rows[index];
        let a = row["offset"].as_u64().unwrap() as usize;
        let error = d
            .decode(&packets[a..a + row["bytes"].as_u64().unwrap() as usize])
            .unwrap_err()
            .to_string();
        assert!(
            error
                .to_lowercase()
                .contains(&c["error"].as_str().unwrap().to_lowercase()),
            "{}: {error}",
            c["name"]
        );
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&video, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error
                .to_lowercase()
                .contains(&c["error"].as_str().unwrap().to_lowercase()),
            "container: {error}"
        );
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
fn packet<'a>(blob: &'a [u8], row: &serde_json::Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
fn reference(c: &serde_json::Value, plain: &[u8]) -> Vec<u8> {
    if c["source_frames"] == 1 {
        return std::fs::read(root().join(format!("{}.f32le", c["name"].as_str().unwrap())))
            .unwrap();
    }
    // Composition reference: the same valid plain AU in source order. Numeric
    // synthesis itself is independently qualified by the 258 saved references.
    let mut d =
        fvid_media::owned_aac::NativeAacDecoder::new(&hex(c["plain_asc"].as_str().unwrap()))
            .unwrap();
    let a = c["plain_unit_offset"].as_u64().unwrap() as usize;
    let b = a + c["plain_unit_bytes"].as_u64().unwrap() as usize;
    let mut result = vec![];
    for _ in 0..c["source_frames"].as_u64().unwrap() * c["frames"].as_array().unwrap().len() as u64
    {
        result.extend(
            d.decode(&plain[a..b])
                .unwrap()
                .into_iter()
                .flat_map(f32::to_le_bytes),
        );
    }
    result
}
#[test]
fn esc_information_and_current_header_reuse_reconstruct_exact_logical_ld_sbr() {
    use fvid_media::owned_aac::{
        aac_eld_sbr::Decoder, aac_esc::reassemble_eld, config::AudioSpecificConfig,
    };
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let information = std::fs::read(root().join("instances.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    let mut switches = 0;
    for c in m["cases"].as_array().unwrap() {
        let parsed = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        let ep = parsed.protection.as_ref().unwrap();
        let switching = c["name"].as_str().unwrap().ends_with("header-switch");
        let mut context = if switching {
            Some(
                Decoder::new(
                    parsed.eld_sbr.clone().unwrap(),
                    parsed.core.sample_rate,
                    parsed.core.frame_samples,
                    parsed.core.channel_configuration,
                )
                .unwrap(),
            )
        } else {
            None
        };
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let frame = ep
                .decode_class_frames(packet(&blob, row))
                .unwrap()
                .remove(0);
            assert_eq!(
                frame.classes.len(),
                row["instances"].as_array().unwrap().len()
            );
            for (index, (class, source)) in frame
                .classes
                .iter()
                .zip(row["instances"].as_array().unwrap())
                .enumerate()
            {
                let a = source["offset"].as_u64().unwrap() as usize;
                let n = source["bits"].as_u64().unwrap() as usize;
                assert_eq!(class.class_index, index);
                assert_eq!(
                    class.bits,
                    (0..n)
                        .map(|i| (information[a + i / 8] >> (7 - i % 8)) & 1)
                        .collect::<Vec<_>>(),
                    "{}",
                    c["name"]
                );
            }
            if let Some(context) = &mut context {
                let raw = reassemble_eld(&parsed.core, &frame, Some(context)).unwrap();
                let a = row["plain_offset"].as_u64().unwrap() as usize;
                let b = a + row["plain_bytes"].as_u64().unwrap() as usize;
                assert_eq!(raw, plain[a..b], "{} source {index}", c["name"]);
                let start = row["instances"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|p| p["category"].as_u64().unwrap() < 9)
                    .map(|p| p["bits"].as_u64().unwrap() as usize)
                    .sum();
                let core = vec![
                    vec![0.; parsed.core.frame_samples as usize];
                    parsed.core.channels as usize
                ];
                context
                    .process_prefix(&raw, start, raw.len() * 8, &core)
                    .unwrap();
                let headers = context
                    .esc_headers(
                        parsed.core.sample_rate,
                        parsed.core.frame_samples,
                        parsed.core.channel_configuration,
                    )
                    .unwrap();
                assert_eq!(headers[0].amplitude_resolution, (4..8).contains(&index));
            }
        }
        if switching {
            switches += 1;
        }
    }
    assert_eq!(switches, 2);
}
#[cfg(feature = "player")]
#[test]
fn multi_source_ep3_ld_sbr_preserves_pcm_packet_clocks_overflow_rollback_and_navigation() {
    use fvid::audio::AudioStream;
    use fvid_media::owned_aac::NativeAacDecoder as Owned;
    use std::io::Cursor;
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    assert_eq!(m["concatenated"].as_array().unwrap().len(), 48);
    for c in m["concatenated"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let expected = reference(c, &plain);
        let count = c["source_frames"].as_u64().unwrap();
        let mut d = Owned::new(&asc).unwrap();
        assert_eq!(
            d.core_packet_samples() as u64,
            c["n"].as_u64().unwrap() * count
        );
        assert_eq!(d.core_frame_samples() as u64, c["n"].as_u64().unwrap());
        let mut native = Vec::new();
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let checkpoint = d.checkpoint();
            let pts = -37 + index as i64 * 113;
            let duration = 29 + index as u64;
            let output = d
                .decode_timed(packet(&blob, row), pts, duration)
                .unwrap()
                .unwrap();
            assert_eq!((output.pts, output.duration), (pts, duration));
            assert_eq!(
                output.samples.len(),
                c["container_frame_samples"].as_u64().unwrap() as usize
                    * c["channels"].as_u64().unwrap() as usize
            );
            d.restore(&checkpoint).unwrap();
            let replay = d
                .decode_timed(packet(&blob, row), pts, duration)
                .unwrap()
                .unwrap();
            assert_eq!(output.samples, replay.samples);
            native.extend(output.samples.into_iter().flat_map(f32::to_le_bytes));
        }
        assert_eq!(native, expected, "{} native", c["name"]);
        assert!(d.finish().unwrap().is_none());
        let checkpoint = d.checkpoint();
        let raw = packet(&blob, &c["frames"][0]);
        assert!(
            d.decode_timed(raw, i64::MAX, count)
                .unwrap_err()
                .to_string()
                .contains("concatenated frame timestamp overflow")
        );
        let after = d.decode(raw).unwrap();
        d.restore(&checkpoint).unwrap();
        assert_eq!(after, d.decode(raw).unwrap());
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut output = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut output).unwrap();
        assert_eq!(output, expected);
        let mut output = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(output, expected);
        let mut output = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut output, None).unwrap();
        assert_eq!(output, expected);
        let mut output = vec![];
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(output, expected);
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
        for reader in &mut readers {
            assert_eq!(play(reader.as_mut()), expected);
            reader.rewind();
            assert_eq!(play(reader.as_mut()), expected);
            let width = c["channels"].as_u64().unwrap() as usize * 4;
            let source = c["container_frame_samples"].as_i64().unwrap() / count as i64;
            let packet = c["container_frame_samples"].as_i64().unwrap();
            for target in [
                -1,
                0,
                source - 1,
                source,
                source + 1,
                packet - 1,
                packet,
                packet + 1,
                7,
                7,
                (expected.len() / width) as i64,
                (expected.len() / width) as i64 + 1,
                13,
                0,
            ] {
                let landed = reader.seek_to(
                    (i128::from(target) * i128::from(reader.timescale())
                        / i128::from(reader.sample_rate())) as i64,
                );
                assert!(landed >= 0);
                let start = ((landed as u128 * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale()))
                    as usize
                    * width)
                    .min(expected.len());
                assert_eq!(
                    play(reader.as_mut()),
                    expected[start..],
                    "{} seek {target}",
                    c["name"]
                );
            }
        }
    }
}
#[test]
fn malformed_ld_sbr_categories_keep_exact_failures_and_restore_complete_packet_history() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut d = Owned::new(&asc).unwrap();
        let index = c["bad_index"].as_u64().unwrap() as usize;
        let mut output = vec![];
        for row in &c["frames"].as_array().unwrap()[..index] {
            output.extend(
                d.decode(packet(&blob, row))
                    .unwrap()
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
        let checkpoint = d.checkpoint();
        let error = d
            .decode(packet(&blob, &c["frames"][index]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        if c["error"] != "EP class CRC mismatch" {
            assert!(
                AudioSpecificConfig::parse(&asc)
                    .unwrap()
                    .protection
                    .unwrap()
                    .decode_class_frames(packet(&blob, &c["frames"][index]))
                    .is_ok()
            );
        }
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .chain(m["concatenated"].as_array().unwrap())
            .find(|v| v["name"] == c["good"])
            .unwrap();
        let raw = packet(&blob, &good["frames"][index]);
        let actual = d.decode(raw).unwrap();
        d.restore(&checkpoint).unwrap();
        assert_eq!(actual, d.decode(raw).unwrap());
        output.extend(actual.into_iter().flat_map(f32::to_le_bytes));
        for row in &good["frames"].as_array().unwrap()[index + 1..] {
            output.extend(
                d.decode(packet(&blob, row))
                    .unwrap()
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
        let expected = reference(good, &plain);
        assert_eq!(output.len(), expected.len());
        assert!(
            output
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .all(|(a, b)| (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7)
        );
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        for error in [
            fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut vec![])
                .unwrap_err()
                .to_string(),
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::io::Cursor::new(&mp4),
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
                std::io::Cursor::new(&mkv),
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
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut readers: Vec<Box<dyn AudioStream>> = vec![
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        std::io::Cursor::new(mp4),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        std::io::Cursor::new(mkv),
                        Default::default(),
                    )
                    .unwrap(),
                ),
            ];
            for reader in &mut readers {
                let mut decoder = reader.make_decoder().unwrap();
                let mut decoded = 0;
                let error = loop {
                    let p = reader
                        .next_packet()
                        .unwrap()
                        .expect("missing expected failure");
                    match decoder.decode_packet(&p.data, p.pts, p.duration as u64) {
                        Err(e) => break e.to_string(),
                        Ok(Some(f)) => {
                            if reader
                                .present_decoded(f.packet, f.source_pts)
                                .unwrap()
                                .is_some()
                            {
                                decoded += 1;
                            }
                        }
                        Ok(None) => {}
                    }
                };
                assert_eq!(decoded, index);
                assert!(
                    error.contains(c["error"].as_str().unwrap()),
                    "{} player: {error}",
                    c["name"]
                );
            }
        }
    }
}

#[test]
fn changed_header_epochs_survive_midstream_reset_and_checkpoint_restore() {
    use fvid_media::owned_aac::NativeAacDecoder as Owned;
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let mut checked = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("header-switch"))
    {
        let mut d = Owned::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let rows = c["frames"].as_array().unwrap();
        let start = d.checkpoint();
        for row in &rows[..6] {
            d.decode(packet(&blob, row)).unwrap();
        }
        let changed = d.checkpoint();
        for row in &rows[6..10] {
            d.decode(packet(&blob, row)).unwrap();
        }
        // Restore a coarse-resolution epoch after a later fine header replaced it.
        d.restore(&changed).unwrap();
        let mut suffix = vec![];
        for row in &rows[6..] {
            suffix.extend(
                d.decode(packet(&blob, row))
                    .unwrap()
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
        let expected =
            std::fs::read(root().join(format!("{}.f32le", c["name"].as_str().unwrap()))).unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let offset = 6 * c["container_frame_samples"].as_u64().unwrap() as usize * width;
        assert_eq!(suffix.len(), expected.len() - offset);
        assert!(
            suffix
                .chunks_exact(4)
                .zip(expected[offset..].chunks_exact(4))
                .all(|(a, b)| (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7)
        );
        d.restore(&start).unwrap();
        for row in &rows[..6] {
            d.decode(packet(&blob, row)).unwrap();
        }
        // Reset while the current header differs from the ASC, not only at EOF.
        d.reset();
        let mut all = vec![];
        for row in rows {
            all.extend(
                d.decode(packet(&blob, row))
                    .unwrap()
                    .into_iter()
                    .flat_map(f32::to_le_bytes),
            );
        }
        assert_eq!(all.len(), expected.len());
        assert!(
            all.chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .all(|(a, b)| (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7)
        );
        checked += 1;
    }
    assert_eq!(checked, 2);
}

use fvid_media::owned_aac::NativeAacDecoder;
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-mps-native")
}
fn source(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aac-eld-mps-core")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packets(case: &serde_json::Value, blob: &[u8]) -> Vec<Vec<u8>> {
    case["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let at = f["offset"].as_u64().unwrap() as usize;
            blob[at..at + f["bytes"].as_u64().unwrap() as usize].to_vec()
        })
        .collect()
}
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    play_with(reader, decoder.as_mut())
}
fn play_with(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut output = Vec::new();
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
    output
}
#[test]
fn native_spatial_mp4_playback_rewind_seek_and_ranges_cover_both_qmf_geometries() {
    use fvid::audio::AudioStream;
    let manifest = manifest();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if matches!(name, "replacement" | "unfinished") {
            continue;
        }
        let movie = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut full).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&movie),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full, "{name} owned");
        let config = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&hex(case["asc"]
            .as_str()
            .unwrap()))
        .unwrap();
        let width = usize::from(config.output_channels()) * 4;
        let n = usize::from(config.core.frame_samples);
        let rate = u64::from(config.output_sample_rate());
        assert_eq!(full.len(), 3 * n * width, "{name}");
        assert!(full.iter().any(|&b| b != 0));
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&movie), Default::default())
                .unwrap();
        assert_eq!(
            usize::from(reader.channels()),
            usize::from(config.output_channels())
        );
        assert_eq!(play(&mut reader), full, "{name} playback");
        reader.rewind();
        assert_eq!(play(&mut reader), full, "{name} rewind");
        for target in [13, n as i64, 7, 3 * n as i64] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            assert_eq!(
                play(&mut reader),
                full[first..],
                "{name} seek {target} landed {landed}"
            );
        }
        for (from, to) in [(7, 23), (2, 19), (7, 23)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &movie,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = (from * rate).div_ceil(1000) as usize * width;
            let last = ((to * rate).div_ceil(1000) as usize * width).min(full.len());
            assert_eq!(range, full[first.min(last)..last], "{name} range");
        }
    }
}
#[test]
fn replacement_refusal_retains_core_noise_qmf_fragment_and_timing_history_for_retry() {
    let manifest = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    let bad = cases.iter().find(|c| c["name"] == "replacement").unwrap();
    let recovery = cases.iter().find(|c| c["name"] == "recovery").unwrap();
    let bad = packets(bad, &blob);
    let good = packets(recovery, &blob);
    let asc = source("tree7.spatial.asc");
    let mut decoder = NativeAacDecoder::new(&asc).unwrap();
    assert!(decoder.decode_timed(&bad[0], 0, 512).unwrap().is_none());
    let saved = decoder.checkpoint();
    let before = decoder.retained_payload_bytes().unwrap();
    let error = decoder
        .decode_timed(&bad[1], 512, 512)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("replaced-fragment source association"),
        "{error}"
    );
    assert_eq!(decoder.retained_payload_bytes().unwrap(), before);
    let first = decoder.decode_timed(&good[1], 512, 512).unwrap().unwrap();
    assert_eq!((first.pts, first.duration), (0, 512));
    let second = decoder.decode_timed(&good[2], 1024, 512).unwrap().unwrap();
    let third = decoder.finish().unwrap().unwrap();
    assert_eq!((second.pts, third.pts), (512, 1024));
    assert!(decoder.finish().unwrap().is_none());
    decoder.restore(&saved).unwrap();
    assert_eq!(
        decoder.decode_timed(&good[1], 512, 512).unwrap().unwrap(),
        first
    );
    assert_eq!(
        decoder.decode_timed(&good[2], 1024, 512).unwrap().unwrap(),
        second
    );
    assert_eq!(decoder.finish().unwrap().unwrap(), third);
    let mut discarded = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join("replacement.mp4")).unwrap(),
        &mut discarded,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("replaced-fragment source association"),
        "{error}"
    );
    let mut full = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&source("tree7.mp4"), &mut full).unwrap();
    let mut recovered = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &std::fs::read(root().join("recovery.mp4")).unwrap(),
        &mut recovered,
    )
    .unwrap();
    assert_eq!(recovered, full);
}
#[test]
fn delayed_ready_frames_are_accounted_checkpointed_and_drained_one_source_window_at_a_time() {
    let base = root().parent().unwrap().join("aac-ld-mps-session");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(base.join("packets.bin")).unwrap();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "tree7-late")
        .unwrap();
    let packets = packets(case, &blob);
    let mut decoder = NativeAacDecoder::new(&source("tree7.spatial.asc")).unwrap();
    assert!(decoder.decode_timed(&packets[0], 0, 512).unwrap().is_none());
    assert!(
        decoder
            .decode_timed(&packets[1], 512, 512)
            .unwrap()
            .is_none()
    );
    let first = decoder
        .decode_timed(&packets[2], 1024, 512)
        .unwrap()
        .unwrap();
    assert_eq!(
        (first.pts, first.duration, first.samples.len()),
        (0, 512, 1024)
    );
    assert!(decoder.delayed());
    let before = decoder.retained_payload_bytes().unwrap();
    let saved = decoder.checkpoint();
    assert!(
        decoder
            .retained_payload_bytes_with_checkpoint(Some(&saved))
            .unwrap()
            >= before + 2 * 1024 * 4
    );
    let second = decoder.finish().unwrap().unwrap();
    let after = decoder.retained_payload_bytes().unwrap();
    assert!(before >= after + 1024 * 4);
    let third = decoder.finish().unwrap().unwrap();
    assert_eq!((second.pts, third.pts), (512, 1024));
    assert!(decoder.finish().unwrap().is_none());
    decoder.restore(&saved).unwrap();
    assert_eq!(decoder.finish().unwrap().unwrap(), second);
    assert_eq!(decoder.finish().unwrap().unwrap(), third);
    decoder.reset();
    assert!(!decoder.delayed());
    assert!(decoder.finish().unwrap().is_none());
}
#[test]
fn eof_and_checkpoint_configuration_errors_are_specific_and_transactional() {
    let movie = std::fs::read(root().join("unfinished.mp4")).unwrap();
    let mut pcm = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unfinished LD-MPS PCM/spatial association"),
        "{error}"
    );
    let one = NativeAacDecoder::new(&source("tree0.spatial.asc")).unwrap();
    let mut other = NativeAacDecoder::new(&source("tree1.spatial.asc")).unwrap();
    let saved = one.checkpoint();
    let before = other.retained_payload_bytes().unwrap();
    assert!(
        other
            .restore(&saved)
            .unwrap_err()
            .to_string()
            .contains("LD-MPS checkpoint")
    );
    assert_eq!(other.retained_payload_bytes().unwrap(), before);
    let mut reference = NativeAacDecoder::new(&source("tree1.spatial.asc")).unwrap();
    let packet = source("tree1-0.au");
    assert_eq!(
        other.decode(&packet).unwrap(),
        reference.decode(&packet).unwrap()
    );
}

#[test]
fn native_pcm_wave_order_matches_spatial_render_of_independent_core_controls() {
    use fvid_media::owned_aac::{
        aac_ld_mps_decor::Mode, aac_ld_mps_pcm, aac_ld_mps_transport::Assembler,
        aac_pce::read_eld_spatial_extensions, bits::BitReader,
    };
    let manifest = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let mut maximum = 0.0f64;
    let mut samples = 0;
    for case in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("core_pcm").is_some())
    {
        let asc = hex(case["asc"].as_str().unwrap());
        let parsed = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&asc).unwrap();
        let config = parsed.ld_mps.as_ref().unwrap();
        let mut native = NativeAacDecoder::new(&asc).unwrap();
        let mut spatial = aac_ld_mps_pcm::Decoder::new(config, Mode::Complex).unwrap();
        let golden: Vec<_> = std::fs::read(root().join(case["core_pcm"].as_str().unwrap()))
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let channels = usize::from(parsed.core.channels);
        let n = usize::from(parsed.core.frame_samples);
        let tree = case["tree"].as_u64().unwrap();
        let mut assembler = Assembler::default();
        for (i, packet) in packets(case, &blob).iter().enumerate() {
            let mut bits = BitReader::new(packet);
            bits.skip(case["frames"][i]["core_bits"].as_u64().unwrap() as usize)
                .unwrap();
            let messages = read_eld_spatial_extensions(&mut bits, &mut assembler).unwrap();
            assert_eq!(messages.len(), 1);
            let planar: Vec<Vec<f64>> = (0..channels)
                .map(|c| {
                    (0..n)
                        .map(|j| f64::from(golden[(i * n + j) * channels + c]))
                        .collect()
                })
                .collect();
            let residuals =
                vec![None; config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes)];
            let expected = spatial
                .decode(
                    &mut BitReader::new(&messages[0].payload),
                    &planar,
                    &residuals,
                )
                .unwrap();
            let actual = native
                .decode_timed(packet, (i * n) as i64, n as u64)
                .unwrap()
                .unwrap();
            assert_eq!(actual.samples.len(), expected.samples.len());
            for (got, row) in actual
                .samples
                .chunks_exact(expected.channels)
                .zip(expected.samples.chunks_exact(expected.channels))
            {
                for c in 0..expected.channels {
                    let want = row[if tree == 4 || tree == 6 {
                        [0, 1, 2, 3, 6, 7, 4, 5][c]
                    } else {
                        c
                    }];
                    let delta = (f64::from(got[c]) - want).abs();
                    maximum = maximum.max(delta);
                    samples += 1;
                    assert!(
                        delta < 5e-8,
                        "{} frame {i} channel {c}: {} != {want}",
                        case["name"],
                        got[c]
                    );
                }
            }
        }
    }
    assert!(samples > 100000);
    eprintln!(
        "independent core input: {samples} spatial samples, maximum absolute PCM difference {maximum}"
    );
}

#[test]
fn all_enabled_native_movie_controls_play_seek_and_reset_a_live_decoder() {
    use fvid::audio::AudioStream;
    let fixtures = root().parent().unwrap().to_path_buf();
    let mut movies = Vec::new();
    for tree in 0..8 {
        movies.push(fixtures.join(format!("aac-eld-mps-core/tree{tree}.mp4")));
    }
    for name in ["aac-ld-mps-session", "aac-eld-spatial-tail"] {
        let folder = fixtures.join(name);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
        for case in manifest["cases"].as_array().unwrap() {
            if !case["error"].is_null()
                || matches!(case["mode"].as_str(), Some("malformed-late" | "unfinished"))
            {
                continue;
            }
            movies.push(folder.join(format!("{}.mp4", case["name"].as_str().unwrap())));
        }
    }
    movies.push(fixtures.join("aac-eld-mps-native/clock90000.mp4"));
    assert_eq!(movies.len(), 72);
    for path in movies {
        let movie = std::fs::read(&path).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut full).unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&movie), Default::default())
                .unwrap();
        let width = usize::from(reader.channels()) * 4;
        assert_eq!(play(&mut reader), full, "{} full", path.display());
        reader.rewind();
        let mut live = reader.make_decoder().unwrap();
        for _ in 0..2 {
            let packet = reader.next_packet().unwrap().unwrap();
            live.decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
        }
        reader.rewind();
        live.reset();
        assert_eq!(
            play_with(&mut reader, live.as_mut()),
            full,
            "{} live reset",
            path.display()
        );
        for target in [13, 512, 7, (full.len() / width) as i64] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            live.reset();
            assert_eq!(
                play_with(&mut reader, live.as_mut()),
                full[first..],
                "{} live seek {target} landed {landed}",
                path.display()
            );
        }
    }
}

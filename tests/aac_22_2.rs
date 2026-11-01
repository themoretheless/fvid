use fvid_media::owned_aac::{NativeAacDecoder, config::AudioSpecificConfig};
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-22-2")
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut pcm = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(f) = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
        {
            if let Some(f) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                pcm.extend(f.data);
            }
        }
    }
    while let Some(f) = decoder.finish_packet().unwrap() {
        if let Some(f) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
            pcm.extend(f.data);
        }
    }
    pcm
}
#[test]
fn indexed_22_2_keeps_every_channel_against_sparse_scalar_pcm_and_checkpoints() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let gold = std::fs::read(root().join("reference.f32le")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 14);
    let mut maximum = 0.0f32;
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config =
            AudioSpecificConfig::parse(&asc).unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(
            u64::from(config.core.object_type),
            c["object_type"].as_u64().unwrap()
        );
        let sbr = c["name"].as_str().unwrap().contains("-sbr-");
        assert_eq!(config.core.sample_rate, if sbr { 24000 } else { 48000 });
        assert_eq!(
            u64::from(config.core.frame_samples),
            c["n"].as_u64().unwrap() / if sbr { 2 } else { 1 }
        );
        assert_eq!(config.core.channels, 24);
        assert_eq!(config.core.channel_configuration, 13);
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(decoder.channels(), 24);
        assert_eq!(decoder.channel_mask(), 0);
        let positions = decoder.channel_positions().unwrap().unwrap();
        assert_eq!(positions.len(), 24);
        for (p, want) in positions.iter().zip(m["positions"].as_array().unwrap()) {
            assert_eq!(format!("{:?}", p.height), want["height"].as_str().unwrap());
            assert_eq!(
                format!("{:?}", p.position),
                want["position"].as_str().unwrap()
            );
            assert_eq!(u64::from(p.index), want["index"].as_u64().unwrap());
            assert_eq!(
                u64::from(p.group_channels),
                want["group_channels"].as_u64().unwrap()
            );
        }
        let n = c["n"].as_u64().unwrap();
        let mut actual = Vec::new();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = r["offset"].as_u64().unwrap() as usize;
            let size = r["bytes"].as_u64().unwrap() as usize;
            let packet = &blob[at..at + size];
            let retained = decoder.retained_payload_bytes().unwrap();
            let checkpoint = decoder.checkpoint();
            assert!(
                decoder
                    .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                    .unwrap()
                    >= retained
            );
            if c["name"].as_str().unwrap().contains("-sbr-") {
                // Reaches the final SBR group after both LFEs were processed.
                assert!(
                    decoder
                        .decode_timed(&packet[..size - 2], i as i64 * n as i64, n)
                        .is_err()
                );
            }
            assert!(
                decoder
                    .decode_timed(&packet[..size / 2], i as i64 * n as i64, n)
                    .is_err()
            );
            let frame = decoder
                .decode_timed(packet, i as i64 * n as i64, n)
                .unwrap_or_else(|e| panic!("{} AU{i}: {e}", c["name"]));
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(
                decoder
                    .decode_timed(packet, i as i64 * n as i64, n)
                    .unwrap(),
                frame
            );
            if let Some(frame) = frame {
                actual.extend(frame.samples);
            }
        }
        while let Some(f) = decoder.finish().unwrap() {
            actual.extend(f.samples);
        }
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let size = c["reference_bytes"].as_u64().unwrap() as usize;
        assert_eq!(actual.len() * 4, size);
        let mut activity = [0.0f32; 24];
        for (i, (got, b)) in actual
            .iter()
            .zip(gold[at..at + size].chunks_exact(4))
            .enumerate()
        {
            let want = f32::from_le_bytes(b.try_into().unwrap());
            let error = (got - want).abs();
            assert!(
                got.is_finite() && want.is_finite() && error < 1e-7,
                "{} scalar{i}: {got} != {want}",
                c["name"]
            );
            maximum = maximum.max(error);
            activity[i % 24] = activity[i % 24].max(got.abs());
        }
        assert!(activity.iter().all(|a| *a > 0.0));
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let center = [k, k];
        let l = [k, 0.];
        let r = [0., k];
        let expected = [
            center,
            l,
            r,
            [1., 0.],
            [0., 1.],
            l,
            r,
            center,
            l,
            r,
            [0., 0.],
            [0., 0.],
            center,
            l,
            r,
            center,
            l,
            r,
            center,
            l,
            r,
            center,
            l,
            r,
        ];
        let router =
            fvid_media::owned_aac::speaker_routing::stereo_router(&asc, 48000, 24, false).unwrap();
        assert_eq!(router.coefficients(), &expected);
    }
    eprintln!("22.2 independent scalar maximum error: {maximum}");
}
#[test]
fn indexed_22_2_mp4_matroska_exports_and_live_navigation_preserve_all_roles() {
    use fvid::audio::AudioStream;
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let bytes = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let matroska = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut expected).unwrap();
        assert_eq!(
            expected.len() as u64,
            c["samples"].as_u64().unwrap() * 24 * 4
        );
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, expected);
        let mut mkv = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&matroska),
            &mut mkv,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(mkv, expected);
        let readers: [Box<dyn AudioStream>; 2] = [
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(matroska),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for mut reader in readers {
            assert_eq!(reader.channels(), 24);
            let mut decoder = reader.make_decoder().unwrap();
            assert_eq!(play(reader.as_mut(), decoder.as_mut()), expected);
            reader.rewind();
            decoder.reset();
            assert_eq!(play(reader.as_mut(), decoder.as_mut()), expected);
            for target in [
                13,
                c["n"].as_u64().unwrap(),
                c["samples"].as_u64().unwrap(),
                7,
            ] {
                let ticks = (u128::from(target) * u128::from(reader.timescale())
                    / u128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                decoder.reset();
                let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale()))
                    as usize
                    * 96)
                    .min(expected.len());
                assert_eq!(
                    play(reader.as_mut(), decoder.as_mut()),
                    expected[first..],
                    "{name} seek{target}"
                );
            }
        }
    }
}

#[test]
fn ld_sbr_22_2_keeps_the_two_lfe_filter_histories_independent() {
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let gold = std::fs::read(root().join("reference.f32le")).unwrap();
    let mut checked = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().contains("-sbr-"))
    {
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut output = Vec::new();
        let n = c["n"].as_u64().unwrap();
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = r["offset"].as_u64().unwrap() as usize;
            let size = r["bytes"].as_u64().unwrap() as usize;
            if let Some(f) = decoder
                .decode_timed(&blob[at..at + size], i as i64 * n as i64, n)
                .unwrap()
            {
                output.extend(f.samples);
            }
        }
        while let Some(f) = decoder.finish().unwrap() {
            output.extend(f.samples);
        }
        assert_eq!(output.len(), c["samples"].as_u64().unwrap() as usize * 24);
        let start = c["reference_offset"].as_u64().unwrap() as usize;
        let mut distinct = 0.0f32;
        for (i, row) in output.chunks_exact(24).enumerate() {
            distinct = distinct.max((row[10] - row[11]).abs());
            for role in [10, 11] {
                let at = start + (i * 24 + role) * 4;
                let want = f32::from_le_bytes(gold[at..at + 4].try_into().unwrap());
                assert!(
                    (row[role] - want).abs() < 1e-7,
                    "{} LFE{} sample{i}: {} != {want}",
                    c["name"],
                    role - 9,
                    row[role]
                );
            }
        }
        assert!(distinct > 1e-6);
        checked += 1;
    }
    assert_eq!(checked, 4);
}

use fvid_media::owned_aac::{
    aac_ps_native::NativePsAacDecoder,
    aac_sbr_dsp::{Dsp, OutputRate},
    aac_sbr_ps::Decoder,
    bits::BitReader,
};
use serde_json::Value;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&bytes("aac-ltp-ps.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn floats(name: &str) -> Vec<f32> {
    bytes(name)
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
// Independent scalar LTP core through the separately qualified owned PS stage.
// This qualifies dispatch/composition; it is not an independent full PS oracle.
fn reference(c: &Value) -> Vec<f32> {
    let core = floats(c["core"].as_str().unwrap());
    let source = if c["point"] == 3 {
        floats("aac-ltp-ps-cce-3-source-core.f32le")
    } else {
        vec![]
    };
    let mode = if c["bands"] == 32 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let mut stage = Decoder::default();
    let mut dsp = Dsp::default();
    let mut lanes = Vec::new();
    if !source.is_empty() {
        for frame in source.chunks_exact(1024) {
            lanes.push(
                dsp.process_upsampling(&[frame], 48000, 16, mode).unwrap()[0]
                    .iter()
                    .map(|v| *v as f32)
                    .collect::<Vec<_>>(),
            );
        }
    }
    let mut output = Vec::new();
    let append = |f: fvid_media::owned_aac::aac_sbr_ps::Frame, out: &mut Vec<f32>| {
        for i in 0..f.pcm[0].len() {
            let left = f.pcm[0][i] as f32;
            out.push(if lanes.is_empty() {
                left
            } else {
                left + lanes[f.frame_index as usize][i]
            });
            out.push(f.pcm[1][i] as f32);
        }
    };
    for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        let crc = bits.read(4).unwrap() == 14;
        if let Some(f) = stage
            .read(
                &mut bits,
                raw.len() * 8,
                crc,
                &core[i * 1024..(i + 1) * 1024],
                48000,
                16,
                mode,
            )
            .unwrap()
        {
            append(f, &mut output);
        }
    }
    while let Some(f) = stage.finish().unwrap() {
        append(f, &mut output);
    }
    output
}
fn encoded(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|v| v.to_le_bytes()).collect()
}
#[test]
fn native_ltp_ps_transitions_and_coupling_match_scalar_core_composition() {
    let blob = bytes("aac-ltp-ps-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let config = hex(c["asc"].as_str().unwrap());
        let mut decoder = if c["signal"] == "implicit" {
            NativePsAacDecoder::new_with_in_band_ps(
                &config,
                c["container_rate"].as_u64().unwrap() as u32,
            )
        } else {
            NativePsAacDecoder::new(&config)
        }
        .unwrap();
        let gold = reference(c);
        for pass in 0..2 {
            if pass > 0 {
                decoder.reset();
            }
            let mut output = Vec::new();
            let mut indices = Vec::new();
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + len];
                let saved = decoder.checkpoint();
                let mut bad = packet.to_vec();
                bad.push(0);
                assert!(decoder
                    .decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("trailing bytes"));
                if let Some(offset) = row["bad_offset"].as_u64() {
                    let offset = offset as usize;
                    let len = row["bad_bytes"].as_u64().unwrap() as usize;
                    let error = decoder.decode(&blob[offset..offset + len]).unwrap_err();
                    assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
                }
                let first = decoder.decode(packet).unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.decode(packet).unwrap(), first);
                if let Some(f) = first {
                    indices.push(f.frame_index);
                    output.extend(f.pcm);
                }
            }
            let checkpoint = decoder.checkpoint();
            let tail = decoder.finish().unwrap();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(decoder.finish().unwrap(), tail);
            if let Some(f) = tail {
                indices.push(f.frame_index);
                output.extend(f.pcm);
            }
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(indices, (0..12).collect::<Vec<_>>());
            assert_eq!(output.len(), gold.len());
            assert!(gold.chunks_exact(2).any(|v| (v[0] - v[1]).abs() > 1e-6));
            for (i, (a, b)) in output.iter().zip(&gold).enumerate() {
                assert!(
                    (*a - *b).abs() < 1e-7,
                    "{} sample {i}: {a} vs {b}",
                    c["name"]
                );
            }
        }
    }
}
#[test]
fn ltp_ps_mp4_public_owned_and_ranges_match_complete_native_pcm() {
    let blob = bytes("aac-ltp-ps-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let source = bytes(c["video"]["file"].as_str().unwrap());
        let gold = reference(c);
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&source, &mut output).unwrap();
        assert_eq!(output.len(), gold.len() * 4);
        for (i, (a, b)) in output.chunks_exact(4).zip(&gold).enumerate() {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap()) - *b).abs() < 1e-7,
                "{} sample {i}",
                c["name"]
            );
        }
        let mut owned = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&source),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, output);
        assert_eq!(stats.channels, 2);
        let mut decoder = NativePsAacDecoder::new_with_in_band_ps(
            &hex(c["asc"].as_str().unwrap()),
            c["container_rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut native = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            if let Some(f) = decoder.decode(&blob[at..at + len]).unwrap() {
                native.extend(f.pcm);
            }
        }
        while let Some(f) = decoder.finish().unwrap() {
            native.extend(f.pcm);
        }
        assert_eq!(output, encoded(&native));
        let rate = c["container_rate"].as_u64().unwrap();
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut selected = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&source),
                &mut selected,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                selected,
                output[(from * rate / 1000) as usize * 8..(to * rate / 1000) as usize * 8]
            );
        }
    }
}
#[test]
fn ltp_ps_adts_discovery_and_probe_preserve_profile_channels_and_pcm() {
    let m = manifest();
    let blob = bytes("aac-ltp-ps-packets.bin");
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["signal"] == "implicit" && c["container_rate"] == 48000)
    {
        let source = bytes(c["adts"].as_str().unwrap());
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut expected,
        )
        .unwrap();
        let mut output = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&source),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.channels, stats.sample_rate, stats.sample_frames),
            (2, 48000, 24576)
        );
        assert_eq!(output, expected);
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut selected = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&source),
                &mut selected,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
            assert_eq!(
                selected,
                expected[(from * 48) as usize * 8..(to * 48) as usize * 8]
            );
        }
        let mut root = Vec::new();
        fvid::native_media::decode_aac_pcm_interval(&source, &mut root, &Default::default(), None)
            .unwrap();
        assert_eq!(root, expected);
        let mut probe = fvid_media::owned_aac::aac_ps_native::InBandPsProbe::new(
            &hex(c["asc"].as_str().unwrap()),
            48000,
        )
        .unwrap();
        assert!(
            fvid_media::owned_aac::aac_ps_native::InBandPsProbe::accepts_mono_program(&hex(c
                ["asc"]
                .as_str()
                .unwrap()))
            .unwrap()
        );
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let mut bad = blob[at..at + len].to_vec();
            bad.push(0);
            assert!(probe.read(&bad).is_err());
            assert!(probe.read(&blob[at..at + len]).unwrap());
        }
    }
}

#[test]
fn ltp_ps_corrupt_videos_report_sbr_crc_after_active_core_history() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["bad_video"].is_object())
    {
        let mut output = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut output,
        )
        .unwrap_err();
        assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
    }
}
#[cfg(feature = "player")]
#[test]
fn ltp_ps_player_rewind_seek_and_delayed_eof_preserve_two_channels() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(frame) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    output.extend(frame.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(frame) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(frame.data);
            }
        }
        output
    }
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut expected).unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(reader.channels(), 2);
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 5800, 9000, c["samples"].as_u64().unwrap() as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 8..]);
        }
    }
}
#[test]
fn ltp_ps_admission_reserves_core_history_before_publishing_pcm() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["signal"] == "explicit" && c["container_rate"] == 48000)
    {
        let source = bytes(c["video"]["file"].as_str().unwrap());
        let mut output = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&source),
            &mut output,
            None,
            &fvid_media::CopyOptions {
                max_controlled_bytes: Some(8 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(output.is_empty());
    }
}

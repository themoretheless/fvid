use fvid_media::owned_aac::aac_native::NativeAacDecoder;
use serde_json::Value;
const RAW: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-multi-sbr-packets.bin");
const MISSING: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-missing-sbr.f64le");
const GOLD: &[u8] = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-multi-sbr-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let off = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    &RAW[off..off + len]
}
fn expected(c: &Value) -> Vec<f32> {
    let channels = c["channels"].as_u64().unwrap() as usize;
    let samples = c["samples"].as_u64().unwrap() as usize;
    let mut pcm = vec![0.; samples * channels];
    let mut source = 0;
    for (i, width) in c["widths"].as_array().unwrap().iter().enumerate() {
        for _ in 0..width.as_u64().unwrap() {
            let target = c["mapping"][source].as_u64().unwrap() as usize;
            source += 1;
            if let Some(original_off) = c["pcm_offsets"][i].as_u64() {
                let missing = c["missing_element"].as_u64() == Some(i as u64);
                let off = if missing {
                    c["missing_pcm_offset"].as_u64().unwrap()
                } else {
                    original_off
                };
                let reference = if missing { MISSING } else { GOLD };
                for n in 0..samples {
                    let at = off as usize + n * 8;
                    pcm[n * channels + target] =
                        f64::from_le_bytes(reference[at..at + 8].try_into().unwrap()) as f32;
                }
            }
        }
    }
    pcm
}
fn compare(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (n, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() < 1e-7, "sample {n}: {a} != {e}");
    }
}
#[test]
fn multiple_pce_elements_keep_independent_sbr_histories_and_pcm_mapping_under_reordering() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new_with_output_rate(
            &hex(c["asc"].as_str().unwrap()),
            c["output_rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut pcm = vec![];
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            let output = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            pcm.extend(output);
        }
        compare(&pcm, &expected(c));
        d.reset();
        let replay: Vec<f32> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| d.decode(packet(row)).unwrap())
            .collect();
        assert_eq!(replay, pcm);
    }
}

#[test]
fn later_element_crc_failure_rolls_back_all_streams_filters_and_core_pcm() {
    let m = manifest();
    for c in m["invalid"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let mut reader = fvid::container::mp4::Mp4Reader::open(
            std::fs::File::open(path).unwrap(),
            Default::default(),
        )
        .unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        for (n, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let mut p = vec![];
            reader.read_packet(track, n, &mut p).unwrap();
            assert_eq!(p, packet(row));
        }
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        decoder.decode(packet(&c["frames"][0])).unwrap();
        let saved = decoder.checkpoint();
        let error = decoder.decode(packet(&c["frames"][1])).unwrap_err();
        assert!(
            error.to_string().contains(c["error"].as_str().unwrap()),
            "{error}"
        );
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["layout"] == c["layout"] && r["slots"] == c["slots"] && r["asc"] == c["asc"]
            })
            .unwrap();
        let output = decoder.decode(packet(&good["frames"][1])).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(output, decoder.decode(packet(&good["frames"][1])).unwrap());
        assert!(
            decoder
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap()
                > decoder.retained_payload_bytes().unwrap()
        );
    }
}

#[test]
fn multichannel_sbr_pce_video_accepts_pcm_intervals_wav_and_memory_admission() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    use std::{io::Cursor, time::Duration};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in manifest()["cases"].as_array().unwrap() {
        if c["video"].is_null() {
            continue;
        }
        let path = root.join(c["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let mut pcm = vec![];
        let options = fvid_control::CopyOptions {
            max_controlled_bytes: Some(128 * 1024 * 1024),
            ..Default::default()
        };
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &options,
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, channels as u16, c["samples"].as_u64().unwrap())
        );
        let actual: Vec<f32> = pcm
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        compare(&actual, &expected(c));
        let reader = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let mut core = vec![];
        fvid::native_media::decode_mp4_aac_reader(reader, &mut core, None).unwrap();
        assert_eq!(core, pcm);
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(&bytes),
                Limits::default(),
            )
            .unwrap();
            assert_eq!(
                (stream.sample_rate(), stream.channels()),
                (48000, channels as u16)
            );
            let mut decoder = stream.make_decoder().unwrap();
            let mut played = vec![];
            while let Some(p) = stream.next_packet().unwrap() {
                let duration = u64::try_from(p.duration).unwrap();
                let frame = decoder
                    .decode_packet(&p.data, p.pts, duration)
                    .unwrap()
                    .unwrap();
                assert_eq!((frame.source_pts, frame.source_duration), (p.pts, duration));
                played.extend(frame.packet.data);
            }
            assert!(decoder.finish_packet().unwrap().is_none());
            assert_eq!(played, pcm);
        }
        let mut range = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut range,
            Some((Duration::from_millis(10), Duration::from_millis(100))),
            &options,
        )
        .unwrap();
        assert_eq!(range, &pcm[480 * channels * 4..4800 * channels * 4]);
        let dest = std::env::temp_dir().join(format!(
            "fvid-multi-sbr-{}-{}.wav",
            std::process::id(),
            path.file_name().unwrap().to_str().unwrap()
        ));
        fvid_media::decode_audio(&path, &dest, &options).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&dest).unwrap(), None)
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.channel_mask),
            (
                48000,
                channels as u16,
                c["channel_mask"].as_u64().unwrap() as u32
            )
        );
        std::fs::remove_file(dest).unwrap();
        let mut rejected = vec![];
        let options = fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut rejected,
                None,
                &options
            )
            .unwrap_err()
            .to_string()
            .contains("controlled memory budget")
        );
        assert!(rejected.is_empty());
    }
}

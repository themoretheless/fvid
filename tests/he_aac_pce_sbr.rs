use fvid_media::owned_aac::aac_native::NativeAacDecoder;
use serde_json::Value;
const PACKETS: &[u8] = include_bytes!("fixtures/playback-errors/he-aac-pce-sbr-packets.bin");
const PCM: &[u8] = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-pcm.f64le");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/he-aac-pce-sbr-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packet(row: &Value) -> &'static [u8] {
    let n = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    &PACKETS[n..n + len]
}
#[test]
fn sole_pce_sce_and_cpe_sbr_accept_independent_pcm_clocks_tags_and_checkpoints() {
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let rate = c["output_rate"].as_u64().unwrap() as u32;
        let mut d = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        let mut pcm = vec![];
        for row in c["frames"].as_array().unwrap() {
            let saved = d.checkpoint();
            let frame = d.decode(packet(row)).unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(frame, d.decode(packet(row)).unwrap());
            pcm.extend(frame);
        }
        assert_eq!(
            (d.sample_rate(), d.channels()),
            (rate, c["channels"].as_u64().unwrap() as u8)
        );
        let channels = d.channels() as usize;
        let off = c["pcm_offset"].as_u64().unwrap() as usize;
        assert_eq!(
            pcm.len(),
            c["samples"].as_u64().unwrap() as usize * channels
        );
        for (n, &a) in pcm.iter().enumerate() {
            let at = off + (n / channels) * 8;
            let e = f64::from_le_bytes(PCM[at..at + 8].try_into().unwrap()) as f32;
            assert!(
                (a - e).abs() < 1e-7,
                "{} {}: {a} != {e}",
                c["kind"],
                c["channels"]
            );
        }
        d.reset();
        let replay: Vec<f32> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| d.decode(packet(r)).unwrap())
            .collect();
        assert_eq!(replay, pcm);
        let incompatible =
            NativeAacDecoder::new_with_output_rate(&hex(c["wrong_asc"].as_str().unwrap()), rate)
                .unwrap()
                .checkpoint();
        assert!(d.restore(&incompatible).is_err());
    }
}

#[test]
fn pce_sbr_mapping_failures_roll_back_filter_and_noise_histories() {
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
            let mut payload = vec![];
            reader.read_packet(track, n, &mut payload).unwrap();
            assert_eq!(payload, packet(row));
        }
        let asc = hex(c["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        decoder.decode(packet(&c["frames"][0])).unwrap();
        let before = decoder.checkpoint();
        assert!(
            decoder
                .decode(packet(&c["frames"][1]))
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        // PCE validation happens before synthesis; packet zero is also a complete
        // original independently coded SBR header and can be replayed from this state.
        let input = packet(&c["frames"][0]);
        let after_failure = decoder.decode(input).unwrap();
        decoder.restore(&before).unwrap();
        assert_eq!(after_failure, decoder.decode(input).unwrap());
    }
}

#[test]
fn pce_sbr_videos_accept_owned_root_pcm_wav_and_actual_implicit_sbr_detection() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    use std::io::Cursor;
    let m = manifest();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in m["cases"].as_array().unwrap() {
        if c["video"].is_null() {
            continue;
        }
        let path = root.join(c["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let mut pcm = vec![];
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (48000, channels as u16, c["samples"].as_u64().unwrap())
        );
        let reader = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
        let mut rendered = vec![];
        fvid::native_media::decode_mp4_aac_reader(reader, &mut rendered, None).unwrap();
        assert_eq!(pcm, rendered);
        let interval = Some((
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(100),
        ));
        let mut range = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut range,
            interval,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(range, &pcm[480 * channels * 4..4800 * channels * 4]);
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(&bytes),
                Limits::default(),
            )
            .unwrap();
            let mut decoder = stream.make_decoder().unwrap();
            let mut played = vec![];
            assert_eq!(
                (stream.sample_rate(), stream.channels()),
                (48000, channels as u16)
            );
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
        let off = c["pcm_offset"].as_u64().unwrap() as usize;
        for (n, b) in pcm.chunks_exact(4).enumerate() {
            let a = f32::from_le_bytes(b.try_into().unwrap());
            let at = off + n / channels * 8;
            let e = f64::from_le_bytes(PCM[at..at + 8].try_into().unwrap()) as f32;
            assert!((a - e).abs() < 1e-7, "{}", path.display());
        }
        if c["kind"] == "implicit" {
            let mut detector =
                NativeAacDecoder::new_with_sbr_detection(&hex(c["asc"].as_str().unwrap())).unwrap();
            assert_eq!(detector.sample_rate(), 24000);
            let mut detected = vec![];
            for row in c["frames"].as_array().unwrap() {
                detected.extend(detector.decode(packet(row)).unwrap());
            }
            assert_eq!(detector.sample_rate(), 48000);
            let detected: Vec<u8> = detected.into_iter().flat_map(f32::to_le_bytes).collect();
            assert_eq!(detected, pcm);
        }
        let dest = std::env::temp_dir().join(format!(
            "fvid-pce-sbr-{}-{}.wav",
            std::process::id(),
            path.file_name().unwrap().to_str().unwrap()
        ));
        fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let info =
            fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&dest).unwrap(), None)
                .unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, channels as u16, stats.sample_frames)
        );
        std::fs::remove_file(dest).unwrap();
    }
}

use fvid::container::mp4::Mp4Reader;
use fvid_media::owned_aac::{aac_native::NativeAacDecoder, config::AudioSpecificConfig};
use serde_json::Value;
use std::io::Cursor;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-extension-flag.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn video(v: &Value) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(v["file"].as_str().unwrap()),
    )
    .unwrap()
}
fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 6144);
    output
}
#[test]
fn gaspecific_extension_flag_preserves_pcm_and_following_pce_sync_fields() {
    let blob = include_bytes!("fixtures/playback-errors/aac-extension-flag-packets.bin");
    for case in manifest()["cases"].as_array().unwrap() {
        let asc = hex(case["asc"].as_str().unwrap());
        let baseline = hex(case["baseline_asc"].as_str().unwrap());
        assert_eq!(
            AudioSpecificConfig::parse(&asc).unwrap(),
            AudioSpecificConfig::parse(&baseline).unwrap()
        );
        let bytes = video(&case["video"]);
        let expected = decode(&video(&case["baseline_video"]));
        assert!(
            expected
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.0)
        );
        assert_eq!(decode(&bytes), expected);
        let mut root = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut root).unwrap();
        assert_eq!(root, expected);
        if case["ps"].as_bool() == Some(true) {
            continue;
        }
        let mut reader = Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut pcm = Vec::new();
        for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let mut packet = Vec::new();
            reader.read_packet(track, i, &mut packet).unwrap();
            let at = row["offset"].as_u64().unwrap() as usize;
            assert_eq!(
                packet,
                &blob[at..at + row["bytes"].as_u64().unwrap() as usize]
            );
            let checkpoint = decoder.checkpoint();
            let frame = decoder
                .decode_timed(
                    &packet,
                    (i as u64 * row["samples"].as_u64().unwrap()) as i64,
                    row["samples"].as_u64().unwrap(),
                )
                .unwrap();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(
                decoder
                    .decode_timed(
                        &packet,
                        (i as u64 * row["samples"].as_u64().unwrap()) as i64,
                        row["samples"].as_u64().unwrap()
                    )
                    .unwrap(),
                frame
            );
            if let Some(frame) = frame {
                pcm.extend(frame.samples);
            }
        }
        if let Some(frame) = decoder.finish().unwrap() {
            pcm.extend(frame.samples);
        }
        assert_eq!(
            pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),
            expected
        );
    }
}
#[test]
fn invalid_or_missing_extension_flag3_has_a_specific_configuration_refusal() {
    for case in manifest()["invalid"].as_array().unwrap() {
        let asc = hex(case["asc"].as_str().unwrap());
        assert_eq!(
            AudioSpecificConfig::parse(&asc).unwrap_err().to_string(),
            case["error"].as_str().unwrap()
        );
        assert_eq!(
            fvid::codec::config::AudioSpecificConfig::parse(&asc)
                .unwrap_err()
                .to_string(),
            case["error"].as_str().unwrap()
        );
        assert_eq!(
            NativeAacDecoder::new(&asc).err().unwrap().to_string(),
            case["error"].as_str().unwrap()
        );
        assert!(
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(video(&case["video"])),
                &mut vec![],
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string()
            .contains(case["error"].as_str().unwrap())
        );
    }
}

#[cfg(feature = "player")]
#[test]
fn extension_flag_playback_replays_checkpoint_seek_and_terminal_samples() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let saved = decoder.checkpoint().unwrap();
            let frame = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&saved).unwrap();
            let replay = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            assert_eq!(
                frame
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration)),
                replay
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration))
            );
            if let Some(frame) = frame {
                if let Some(pcm) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    out.extend(pcm.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.extend(pcm.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        out
    }
    for case in manifest()["cases"].as_array().unwrap() {
        let data = video(&case["video"]);
        let expected = decode(&video(&case["baseline_video"]));
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for at in [1100, 3000, 6144] {
            let landed = reader.seek_to(at);
            let size = case["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(
                play(&mut reader),
                expected[landed as usize * size..],
                "{} seek {at}",
                case["name"]
            );
        }
    }
}

use serde_json::Value;
use std::{io::Cursor, time::Duration};
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/adts-multi.json")).unwrap()
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn multiblock_adts_exposes_exact_logical_packets_timestamps_and_complete_pcm() {
    let blob = include_bytes!("fixtures/playback-errors/adts-multi-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let mut video = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut video,
            None,
            &Default::default(),
        )
        .unwrap();
        for file in c["adts"].as_array().unwrap() {
            let data = bytes(file["file"].as_str().unwrap());
            let owned = fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", file["file"]));
            let root = fvid::container::adts::Aac::parse(&data, &Default::default()).unwrap();
            assert_eq!(owned.packets(), 12);
            assert_eq!(owned.samples(), 12288);
            assert_eq!(root.samples(), 12288);
            let mut streaming =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            let mut root_stream =
                fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            assert_eq!(streaming.audio_specific_config(), owned.configuration);
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let expected = &blob[at..at + len];
                assert_eq!(owned.packet(i), expected);
                assert_eq!(root.packet(i), expected);
                assert_eq!(owned.frames[i].pts, i as u64 * 1024);
                assert_eq!(root.frames[i].pts, i as u64 * 1024);
                assert_eq!(streaming.next_packet().unwrap().unwrap(), expected);
                assert_eq!(root_stream.next_packet().unwrap().unwrap(), expected);
            }
            assert!(streaming.next_packet().unwrap().is_none());
            assert!(root_stream.next_packet().unwrap().is_none());
            let mut actual = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut actual,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, 12288);
            assert_eq!(actual, video, "{}", file["file"]);
            let mut exported = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut exported,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(exported, video);
            let range = Some((Duration::from_millis(55), Duration::from_millis(245)));
            let mut selected = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut selected,
                range,
                &Default::default(),
            )
            .unwrap();
            let stride = c["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(selected, video[1320 * stride..5880 * stride]);
            let limits = fvid_media::owned_aac::adts::Limits {
                packets: 11,
                ..Default::default()
            };
            assert!(
                fvid_media::owned_aac::adts::Aac::parse(&data, &limits)
                    .unwrap_err()
                    .to_string()
                    .contains("packet limit")
            );
        }
    }
}
#[test]
fn malformed_multiblock_crc_positions_and_count_are_exact_refusals() {
    for c in manifest()["invalid"].as_array().unwrap() {
        let data = bytes(c["file"].as_str().unwrap());
        let expected = c["error"].as_str().unwrap();
        assert_eq!(
            fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            expected,
            "{}",
            c["kind"]
        );
        assert_eq!(
            fvid::container::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            expected
        );
        let warm = c["warm_blocks"].as_u64().unwrap_or(0) as usize;
        if warm == 0 {
            assert_eq!(
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data))
                    .err()
                    .unwrap()
                    .to_string(),
                expected
            );
            assert_eq!(
                fvid::container::adts::StreamReader::open(Cursor::new(&data))
                    .err()
                    .unwrap()
                    .to_string(),
                expected
            );
        } else {
            let mut owned =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            let mut root = fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            for _ in 0..warm {
                assert_eq!(
                    owned.next_packet().unwrap().unwrap(),
                    root.next_packet().unwrap().unwrap()
                );
            }
            assert_eq!(owned.next_packet().unwrap_err().to_string(), expected);
            assert_eq!(root.next_packet().unwrap_err().to_string(), expected);
            assert!(owned.next_packet().unwrap().is_none());
            assert!(root.next_packet().unwrap().is_none());
        }
        let mut output = Vec::new();
        assert_eq!(
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut output,
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string(),
            expected
        );
        // Discovery buffers PCM until the output clock is known. A later
        // malformed frame must not publish an unnegotiated core-rate prefix.
        assert!(output.is_empty(), "no PCM from failed clock negotiation");
        if warm > 0 {
            let options = fvid_media::CopyOptions {
                max_packets: Some(warm as u64),
                ..Default::default()
            };
            let mut prefix = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(Cursor::new(&data), &mut prefix, None, &options)
                .unwrap();
            assert_eq!(prefix.len(), warm * 1024 * 4, "explicit valid prefix");
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn multiplexed_adts_playback_rewind_seek_and_checkpoints_match_companion_video() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let saved = decoder.checkpoint().unwrap();
            let actual = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&saved).unwrap();
            let replay = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            assert_eq!(
                actual
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration)),
                replay
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration))
            );
            if let Some(frame) = actual {
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
    for c in manifest()["cases"].as_array().unwrap() {
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        for file in c["adts"].as_array().unwrap() {
            let data = bytes(file["file"].as_str().unwrap());
            let mut reader =
                fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default())
                    .unwrap();
            assert_eq!(play(&mut reader), expected, "{}", file["file"]);
            reader.rewind();
            assert_eq!(play(&mut reader), expected);
            for at in [1100, 4300, 12288] {
                let landed = reader.seek_to(at);
                let stride = c["channels"].as_u64().unwrap() as usize * 4;
                assert_eq!(
                    play(&mut reader),
                    expected[landed as usize * stride..],
                    "{} seek {at}",
                    file["file"]
                );
            }
        }
    }
}

#[test]
fn multiplexed_adts_remuxes_to_mp4_and_matroska_without_packet_loss() {
    use fvid::container::{adts, matroska_write, mp4_write};
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("mixed"))
    {
        for file in case["adts"].as_array().unwrap() {
            let data = bytes(file["file"].as_str().unwrap());
            let mut expected = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut expected,
                None,
                &Default::default(),
            )
            .unwrap();
            for sequential in [false, true] {
                let mut output = Cursor::new(Vec::new());
                if sequential {
                    mp4_write::write_adts_aac_reader(
                        adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                        &mut output,
                    )
                    .unwrap();
                } else {
                    mp4_write::write_adts_aac(&data, &mut output).unwrap();
                }
                let mut actual = Vec::new();
                fvid::native_media::decode_mp4_aac_pcm(output.get_ref(), &mut actual).unwrap();
                assert_eq!(actual, expected, "{} MP4", file["file"]);
            }
            let mut output = Cursor::new(Vec::new());
            matroska_write::write_adts(
                adts::StreamReader::open(Cursor::new(&data)).unwrap(),
                &mut output,
                None,
                None,
            )
            .unwrap();
            let mut actual = Vec::new();
            fvid::native_media::decode_matroska_aac_pcm_interval(
                output.get_ref(),
                &mut actual,
                None,
            )
            .unwrap();
            assert_eq!(actual, expected, "{} Matroska", file["file"]);
        }
    }
}

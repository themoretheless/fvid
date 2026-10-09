use serde_json::Value;
use std::{io::Cursor, time::Duration};
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/adts-layout-sbr.json"
    ))
    .unwrap()
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn compare(actual: &[u8], gold: &[u8], name: &str) {
    assert_eq!(actual.len(), gold.len(), "{name}");
    assert!(gold
        .chunks_exact(4)
        .any(|b| f32::from_le_bytes(b.try_into().unwrap()) != 0.));
    for (a, b) in actual.chunks_exact(4).zip(gold.chunks_exact(4)) {
        assert!(
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                < 2e-7,
            "{name}"
        );
    }
}
#[test]
fn multichannel_lfe_and_sbr_adts_packets_pcm_clock_and_intervals_match_own_video() {
    let blob = include_bytes!("fixtures/playback-errors/adts-layout-sbr-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut video = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut video,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, c["samples"].as_u64().unwrap());
        assert_eq!(stats.sample_rate, c["sample_rate"].as_u64().unwrap() as u32);
        if let Some(file) = c["pcm_file"].as_str() {
            compare(&video, &bytes(file), name);
        }
        for file in c["adts"].as_array().unwrap() {
            let data = bytes(file.as_str().unwrap());
            let indexed =
                fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default()).unwrap();
            assert_eq!(indexed.packets(), 12);
            assert_eq!(indexed.samples(), 12288);
            assert_eq!(
                indexed.channels,
                c["core_channels"].as_u64().unwrap() as u16
            );
            let mut reader =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                assert_eq!(indexed.packet(i), &blob[at..at + len]);
                assert_eq!(reader.next_packet().unwrap().unwrap(), &blob[at..at + len]);
                assert_eq!(indexed.frames[i].pts, i as u64 * 1024);
            }
            assert!(reader.next_packet().unwrap().is_none());
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut pcm,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_rate, c["sample_rate"].as_u64().unwrap() as u32);
            assert_eq!(stats.sample_frames, c["samples"].as_u64().unwrap());
            assert_eq!(pcm, video, "{}", file);
            let mut root = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut root,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(root, video);
            let mut selected = Vec::new();
            let range = Some((Duration::from_millis(55), Duration::from_millis(245)));
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut selected,
                range,
                &Default::default(),
            )
            .unwrap();
            let rate = c["sample_rate"].as_u64().unwrap() as usize;
            let stride = c["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(
                selected,
                video[rate * 55 / 1000 * stride..rate * 245 / 1000 * stride],
                "{} interval",
                file
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn multichannel_and_implicit_sbr_playback_has_negotiated_rate_rewind_and_seek() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        let mut packet_index = 0;
        let packet_ticks = u64::from(reader.sample_rate()) * 1024 / 24000;
        while let Some(packet) = reader.next_packet().unwrap() {
            assert_eq!(packet.pts, packet_index * packet_ticks as i64);
            assert_eq!(packet.duration, packet_ticks as i64);
            packet_index += 1;
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
        output
    }
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
            let data = bytes(file.as_str().unwrap());
            let mut reader =
                fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default())
                    .unwrap();
            assert_eq!(
                reader.sample_rate(),
                c["sample_rate"].as_u64().unwrap() as u32,
                "{} metadata",
                file
            );
            assert_eq!(reader.duration().unwrap(), Duration::from_millis(512));
            assert_eq!(play(&mut reader), video, "{} playback", file);
            reader.rewind();
            assert_eq!(play(&mut reader), video);
            assert_eq!(
                reader.timescale(),
                c["sample_rate"].as_u64().unwrap() as u32
            );
            for target in [1100, 4300, c["samples"].as_i64().unwrap()] {
                let landed = reader.seek_to(target);
                let stride = c["channels"].as_u64().unwrap() as usize * 4;
                assert_eq!(
                    play(&mut reader),
                    video[landed as usize * stride..],
                    "{} seek",
                    file
                );
            }
        }
    }
}

#[test]
fn implicit_sbr_remux_preserves_pcm_and_output_clock() {
    use fvid::container::{adts, matroska_write, mp4_write};
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("mixed"))
    {
        for file in case["adts"].as_array().unwrap() {
            let data = bytes(file.as_str().unwrap());
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
                let indexed = fvid::container::mp4::Mp4Reader::open(
                    Cursor::new(output.get_ref()),
                    Default::default(),
                )
                .unwrap();
                let track = &indexed.tracks()[0];
                assert_eq!(
                    track.sample_rate,
                    case["sample_rate"].as_u64().unwrap() as u32
                );
                assert_eq!(track.timescale, track.sample_rate);
                assert_eq!(track.duration, case["samples"].as_u64().unwrap());
                fvid::native_media::decode_mp4_aac_pcm(output.get_ref(), &mut actual).unwrap();
                assert_eq!(actual, expected, "{} MP4", file);
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
            let indexed = fvid::container::webm::WebmReader::open(
                Cursor::new(output.get_ref()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                indexed.tracks[0].sample_rate,
                case["sample_rate"].as_u64().unwrap()
            );
            fvid::native_media::decode_matroska_aac_pcm_interval(
                output.get_ref(),
                &mut actual,
                None,
            )
            .unwrap();
            assert_eq!(actual, expected, "{} Matroska", file);
        }
    }
}

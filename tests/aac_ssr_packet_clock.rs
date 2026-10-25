#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        let stamp = reader.decoder_stamp(&packet);
        if let Some(frame) = decoder
            .decode_packet(&packet.data, stamp, packet.duration as u64)
            .unwrap()
        {
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
    reader.validate_eof().unwrap();
    out
}
#[cfg(feature = "player")]
#[test]
fn ssr_container_clocks_preserve_pcm_and_exact_seek_with_variable_synthesis_windows() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let folder = root.join("aac-ssr-packet-clock");
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let control = std::fs::read(root.join(c["control"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut expected).unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        assert_eq!(
            expected.len(),
            c["samples"].as_u64().unwrap() as usize * width
        );
        for key in ["variable", "coarse", "matroska"] {
            let bytes = std::fs::read(folder.join(c[key].as_str().unwrap())).unwrap();
            let mut exported = Vec::new();
            let mut backend = Vec::new();
            if key == "matroska" {
                fvid::native_media::decode_matroska_aac_pcm_interval(&bytes, &mut exported, None)
                    .unwrap();
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(&bytes),
                    &mut backend,
                    None,
                    &Default::default(),
                )
                .unwrap();
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut exported).unwrap();
                fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    Cursor::new(&bytes),
                    &mut backend,
                    None,
                    &Default::default(),
                )
                .unwrap();
            }
            assert!(exported == expected, "{name} {key}: public PCM");
            assert!(backend == expected, "{name} {key}: backend PCM");
            let mut reader: Box<dyn AudioStream> = if key == "matroska" {
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(reader.sample_rate(), 24000);
            assert_eq!(
                reader.timescale(),
                if key == "coarse" {
                    1000
                } else if key == "matroska" {
                    1_000_000_000
                } else {
                    24000
                }
            );
            assert!(play(reader.as_mut()) == expected, "{name} {key}: full PCM");
            reader.rewind();
            assert!(
                play(reader.as_mut()) == expected,
                "{name} {key}: rewind PCM"
            );
            for sample in [13u64, 1100, 2600, 5900, 7, c["samples"].as_u64().unwrap()] {
                let target = Duration::from_nanos(sample * 1_000_000_000 / 24000);
                let landed = reader.seek_time(target);
                let first =
                    (reader.time_of(landed).as_nanos() * 24000).div_ceil(1_000_000_000) as usize;
                let actual = play(reader.as_mut());
                let wanted = &expected[(first * width).min(expected.len())..];
                assert!(
                    actual == wanted,
                    "{name} {key} seek {sample}: got {} bytes, expected {}",
                    actual.len(),
                    wanted.len()
                );
            }
        }
    }
}

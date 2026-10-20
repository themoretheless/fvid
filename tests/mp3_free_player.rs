#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp3-free")
}
#[test]
fn free_mp3_raw_and_video_containers_preserve_boundaries_pcm_seek_and_rewind() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 39);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap() as u32;
        let channels = case["channels"].as_u64().unwrap() as u16;
        let stride = usize::from(channels) * 4;
        let expected = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        let samples = case["samples"].as_u64().unwrap() as usize;
        for container in ["mp3", "mkv"] {
            let data = std::fs::read(root().join(format!("{name}.{container}"))).unwrap();
            let mut stream: Box<dyn AudioStream> = if container == "mp3" {
                Box::new(
                    fvid::playback_mp3::Mp3AudioReader::open(data.as_slice(), Default::default())
                        .unwrap_or_else(|e| panic!("{name}: {e}")),
                )
            } else {
                {
                    let mut video = fvid::playback_webm::WebmVideoReader::open_software(
                        Cursor::new(&data),
                        32 << 20,
                    )
                    .unwrap();
                    let mut frames = 0;
                    while video.read_frame().unwrap() {
                        assert_eq!(video.dimensions(), [16, 16]);
                        assert!(video.rgb().iter().all(|&v| v == 130));
                        frames += 1;
                    }
                    assert_eq!(frames, case["frames"].as_u64().unwrap());
                }
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(data),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(stream.codec(), "A_MPEG/L3");
            assert_eq!((stream.sample_rate(), stream.channels()), (rate, channels));
            let mut decoder = stream.make_decoder().unwrap();
            for pass in 0..2 {
                if pass > 0 {
                    stream.rewind();
                    decoder.reset();
                }
                let mut pcm = Vec::new();
                let mut count = 0;
                while let Some(packet) = stream.next_packet().unwrap() {
                    assert_eq!(packet.data[2] & 0xf0, 0);
                    if count == 0 {
                        let fake = case["false_sync"].as_u64().unwrap() as usize;
                        let error = fvid_codecs::mp3_frame::Frame::parse(&packet.data[fake..])
                            .err()
                            .expect("the ancillary header is not a valid coded frame");
                        assert!(error.to_string().contains("CRC"), "{name}: {error}");
                    }
                    assert_eq!(
                        packet.data.len(),
                        case["sizes"][count].as_u64().unwrap() as usize,
                        "{name}"
                    );
                    count += 1;
                    let frame = decoder
                        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(frame) = stream
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        pcm.extend(frame.data);
                    }
                }
                assert_eq!(count, case["frames"].as_u64().unwrap() as usize);
                assert_eq!(pcm, expected, "{name}.{container}");
            }
            for target in [samples - 1, 1, samples / 2, 0, samples] {
                let ticks = if container == "mp3" {
                    target as i64
                } else {
                    (target as u128 * 1_000_000_000 / u128::from(rate)) as i64
                };
                stream.seek_to(ticks);
                decoder.reset();
                let mut pcm = Vec::new();
                while let Some(packet) = stream.next_packet().unwrap() {
                    let frame = decoder
                        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(frame) = stream
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        pcm.extend(frame.data);
                    }
                }
                assert_eq!(
                    pcm,
                    expected[target * stride..],
                    "{name}.{container} seek {target}"
                );
            }
        }
    }
}
#[test]
fn corrupted_second_crc_cannot_disappear_into_a_larger_free_frame() {
    let data = std::fs::read(root().join("bad-second-crc.mp3")).unwrap();
    let error = fvid::playback_mp3::Mp3AudioReader::open(data.as_slice(), Default::default())
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("cannot infer free-format"),
        "{error}"
    );
    let data = std::fs::read(root().join("bad-second-crc.mkv")).unwrap();
    let mut stream =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(data), Default::default())
            .unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    let first = stream.next_packet().unwrap().unwrap();
    assert!(
        decoder
            .decode_packet(&first.data, first.pts, first.duration as u64)
            .unwrap()
            .is_some()
    );
    let second = stream.next_packet().unwrap().unwrap();
    let error = decoder
        .decode_packet(&second.data, second.pts, second.duration as u64)
        .err()
        .unwrap();
    assert!(error.to_string().contains("CRC"), "{error}");
}

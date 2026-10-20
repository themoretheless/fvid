#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::io::Cursor;
#[test]
fn free_mp2_files_and_video_containers_keep_nonzero_pcm_and_frame_boundaries() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp2-native");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|v| v != "ffmeta") {
            continue;
        }
        let meta = std::fs::read(&path).unwrap();
        let channels = usize::from(meta[4]);
        let rate = u32::from_be_bytes(meta[..4].try_into().unwrap());
        let sizes: Vec<usize> = meta[5..]
            .chunks_exact(4)
            .map(|v| u32::from_be_bytes(v.try_into().unwrap()) as usize)
            .collect();
        let expected = std::fs::read(path.with_extension("f32le")).unwrap();
        let expected: Vec<f32> = expected
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        for container in ["mp2", "mkv"] {
            let source = path.with_extension(container);
            let bytes = std::fs::read(&source).unwrap();
            let mut stream: Box<dyn AudioStream> = if container == "mp2" {
                Box::new(
                    fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default())
                        .unwrap_or_else(|e| panic!("{}: {e}", source.display())),
                )
            } else {
                let stats = fvid_media::decode_video(&source)
                    .unwrap_or_else(|e| panic!("{}: {e}", source.display()));
                assert!(stats.video_frames > 0);
                assert_eq!((stats.width, stats.height), (32, 32));
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(bytes),
                        Default::default(),
                    )
                    .unwrap(),
                )
            };
            assert_eq!(stream.codec(), "A_MPEG/L2");
            assert_eq!(stream.sample_rate(), rate);
            assert_eq!(usize::from(stream.channels()), channels);
            for _ in 0..2 {
                stream.rewind();
                let mut decoder = stream.make_decoder().unwrap();
                let mut actual = Vec::new();
                let mut packets = 0;
                while let Some(packet) = stream.next_packet().unwrap() {
                    assert_eq!(packet.data.len(), sizes[packets], "{}", source.display());
                    if container == "mp2" {
                        assert_eq!(packet.pts, (packets * 1152) as i64);
                        assert_eq!(packet.duration, 1152);
                    }
                    let pcm = decoder
                        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                        .unwrap()
                        .unwrap()
                        .packet;
                    actual.extend(
                        pcm.data
                            .chunks_exact(4)
                            .map(|v| f32::from_le_bytes(v.try_into().unwrap())),
                    );
                    packets += 1;
                }
                assert_eq!(packets, 4);
                assert_eq!(actual.len(), expected.len());
                for (a, b) in actual.iter().zip(&expected) {
                    assert!((a - b).abs() < 3e-6, "{}: PCM mismatch", source.display());
                }
            }
        }
        count += 1;
    }
    assert_eq!(count, 57);
}

#[test]
fn single_free_mp2_frame_uses_file_boundary_even_with_ancillary_sync() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp2-native");
    let meta = std::fs::read(dir.join("free-0-0-1-48000.ffmeta")).unwrap();
    let size = u32::from_be_bytes(meta[5..9].try_into().unwrap()) as usize;
    let raw = std::fs::read(dir.join("free-0-0-1-48000.mp2")).unwrap();
    let mut stream =
        fvid::playback_mp3::Mp3AudioReader::open(&raw[..size], Default::default()).unwrap();
    assert_eq!(stream.mp3().packets(), 1);
    let mut decoder = stream.make_decoder().unwrap();
    let packet = stream.next_packet().unwrap().unwrap();
    let pcm = decoder
        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
        .unwrap()
        .unwrap()
        .packet;
    let expected = std::fs::read(dir.join("free-0-0-1-48000.f32le")).unwrap();
    let actual: Vec<f32> = pcm
        .data
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let expected: Vec<f32> = expected[..1152 * 2 * 4]
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 3e-6);
    }
    assert!(stream.next_packet().unwrap().is_none());
}

#[test]
fn damaged_free_frame_cannot_be_hidden_by_combining_adjacent_frames() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp2-native/free-second-crc-damaged.mp2");
    let video = path.with_extension("mkv");
    assert!(fvid_media::decode_video(&video).unwrap().video_frames > 0);
    let bytes = std::fs::read(video).unwrap();
    let mut stream =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    let first = stream.next_packet().unwrap().unwrap();
    decoder
        .decode_packet(&first.data, first.pts, first.duration as u64)
        .unwrap();
    let second = stream.next_packet().unwrap().unwrap();
    let error = decoder
        .decode_packet(&second.data, second.pts, second.duration as u64)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("CRC mismatch"), "{error}");
    let data = std::fs::read(path).unwrap();
    assert!(
        fvid::playback_mp3::Mp3AudioReader::open(data.as_slice(), Default::default()).is_err(),
        "a bad second CRC was hidden by folding frame pairs"
    );
}

#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn predictor_rounding_wraps_at_the_coded_integer_width() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-predictor-overflow");
    // Literal authored PCM is independent of the predictive encoder.
    let target: Vec<i32> = [0, 65538]
        .into_iter()
        .chain((2..16).map(|i| 65540 + (i - 2) * 2))
        .collect();
    let expected: Vec<u8> = target
        .iter()
        .flat_map(|&x| (x as f32 / 8388608.).to_le_bytes().repeat(2))
        .collect();
    let cookie = std::fs::read(root.join("cookie.bin")).unwrap();
    let packet = std::fs::read(root.join("packet.bin")).unwrap();
    let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
    let actual: Vec<u8> = decoder
        .decode_pcm(&packet)
        .unwrap()
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    assert_eq!(
        actual, expected,
        "rounding must wrap before arithmetic shift"
    );
    let bytes = std::fs::read(root.join("rounding.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    for _ in 0..2 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    for (ns, first) in [(0, 0usize), (41666, 2), (41667, 3)] {
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        reader.seek_to(ns);
        let mut decoder = reader.make_decoder().unwrap();
        let mut actual = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                actual.extend(out.data);
            }
        }
        assert_eq!(
            actual,
            [&expected[first * 8..], &expected].concat(),
            "seek {ns}"
        );
    }
    let bytes = std::fs::read(root.join("rounding.m4a")).unwrap();
    let mut reader =
        fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let mut actual = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        actual.extend(
            reader
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap()
                .data,
        );
    }
    assert_eq!(actual, expected.repeat(2));
    for extension in ["mkv", "m4a"] {
        use std::io::{Read, Seek, SeekFrom};
        let out = std::env::temp_dir().join(format!(
            "fvid-alac-overflow-{}-{extension}.wav",
            std::process::id()
        ));
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Remove(out.clone());
        fvid_media::decode_audio(
            &root.join(format!("rounding.{extension}")),
            &out,
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(out).unwrap();
        let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert!(wave.float);
        assert_eq!((wave.channels, wave.bits_per_sample), (2, 32));
        file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
        let mut pcm = vec![0; wave.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        assert_eq!(pcm, expected.repeat(2), "{extension} public export");
    }
}

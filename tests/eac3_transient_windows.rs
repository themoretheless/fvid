#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use fvid::codec::eac3_decoder::{checked_frame, frame};
use std::path::Path;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-transient-windows")
            .join(n),
    )
    .unwrap()
}
#[test]
fn transient_parameters_survive_parsing_with_sample_units_and_future_locations() {
    let control = checked_frame(&read("control.ec3")).unwrap();
    assert!(control.transient_windows.iter().all(Option::is_none));
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["pcm_processing_accepted"], false);
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let raw = read(&format!("{name}.ec3"));
        let h = checked_frame(&raw).unwrap();
        assert!(h.blocks_state.transient_processing);
        let mut active = 0;
        for (index, w) in c["windows"].as_array().unwrap().iter().enumerate() {
            if w.is_null() {
                assert!(h.transient_windows[index].is_none());
            } else {
                active += 1;
                let got = h.transient_windows[index].unwrap();
                assert_eq!(u64::from(got.location), w["location"].as_u64().unwrap());
                assert_eq!(u64::from(got.length), w["length"].as_u64().unwrap());
            }
        }
        assert!(h.transient_windows[2..].iter().all(Option::is_none));
        assert_eq!(h.audio_bit, control.audio_bit + 2 + 18 * active);
        let mut movie = fvid::playback_webm_audio::WebmAudioReader::open(
            std::io::Cursor::new(read(&format!("{name}.mkv"))),
            Default::default(),
        )
        .unwrap();
        for _ in 0..3 {
            let packet = movie.next_packet().unwrap().unwrap();
            assert_eq!(
                checked_frame(&packet.data).unwrap().transient_windows,
                h.transient_windows
            );
        }
        assert!(movie.next_packet().unwrap().is_none());
        assert_eq!(frame(&raw).unwrap().transient_windows, h.transient_windows);
    }
}

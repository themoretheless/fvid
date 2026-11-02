#![cfg(feature = "player")]
use fvid::{audio::{AudioPacket, AudioStream}, playback_ac3::Ac3AudioReader};
#[test]
fn parsed_raw_dolby_windows_preserve_pcm_stamps_and_oversize_refusal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/coded-audio-index-eof");
    for name in ["tone.ac3", "tone.eac3"] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let reader = Ac3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        let stride = usize::from(reader.channels()) * 4;
        for frame in &reader.ac3().frames {
            let packet = |frames| AudioPacket { data: vec![0; frames * stride], pts: 0,
                timebase_num: 1, timebase_den: 1 };
            let output = reader.present_decoded(packet(frame.samples as usize), frame.pts as i64)
                .unwrap().unwrap();
            assert_eq!((output.pts, output.timebase_num, output.timebase_den),
                (frame.pts, 1, reader.sample_rate()));
            assert_eq!(output.data, vec![0; frame.samples as usize * stride]);
            let error = reader.present_decoded(packet(frame.samples as usize + 1), frame.pts as i64)
                .err().expect("a decoded packet must not exceed its indexed window");
            assert!(error.to_string().contains("exceeds its frame window"), "{name}: {error}");
        }
    }
}

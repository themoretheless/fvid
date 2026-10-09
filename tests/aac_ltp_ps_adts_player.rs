use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
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
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}

fn check(file: &str, channels: u16) {
    let data = bytes(file);
    let mut expected = Vec::new();
    fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(&data),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    let mut reader =
        fvid::playback_aac::AacAudioReader::open(Cursor::new(&data), Default::default()).unwrap();
    assert_eq!(
        (reader.channels(), reader.sample_rate(), reader.timescale()),
        (channels, 48000, 48000),
        "{file}"
    );
    assert_eq!(play(&mut reader), expected, "{file}");
    reader.rewind();
    assert_eq!(play(&mut reader), expected, "rewind {file}");
    let stride = channels as usize * 4;
    for target in [1100, 5800, 9000, (expected.len() / stride) as i64] {
        let landed = reader.seek_to(target);
        assert_eq!(
            play(&mut reader),
            expected[landed as usize * stride..],
            "seek {file} {target}"
        );
    }
}
#[test]
fn ltp_ps_adts_player_negotiates_stereo_and_preserves_rewind_seek_and_eof() {
    for kind in ["mono", "cce-0", "cce-1", "cce-3"] {
        check(&format!("aac-ltp-ps-{kind}-synthetic.aac"), 2);
    }
}
#[test]
fn late_ltp_sbr_adts_player_negotiates_rate_and_preserves_rewind_seek_and_eof() {
    for kind in ["sce", "cce", "cce-active"] {
        check(&format!("aac-ltp-late-sbr-{kind}-synthetic.aac"), 1);
    }
}

//! Sample-exact compressed companion trimming must retain decoder preroll.
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn pcm(bytes: &[u8], codec: &str, index: usize, interval: Option<(Duration, Duration)>) -> Vec<u8> {
    let options = fvid_control::CopyOptions {
        streams: vec![index],
        ..Default::default()
    };
    let mut out = Vec::new();
    if codec == "A_AAC" {
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(bytes),
            &mut out,
            interval,
            &options,
        )
        .unwrap();
    } else {
        fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(bytes),
            &mut out,
            interval,
            &options,
        )
        .unwrap();
    }
    out
}
#[test]
fn hevc_interval_retains_aac_opus_preroll_and_exact_selected_samples() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("hevc-audio-interval.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8);
    for (index, c) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["companions"].as_bool().unwrap())
        .enumerate()
    {
        let source = root().join(c["file"].as_str().unwrap());
        let bytes = std::fs::read(&source).unwrap();
        let mut input =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        input.scan_all().unwrap();
        for interval in c["intervals_us"].as_array().unwrap() {
            let (from, to) = (interval[0].as_i64().unwrap(), interval[1].as_i64().unwrap());
            let output = std::env::temp_dir().join(format!(
                "fvid-hevc-audio-trim-{}-{index}-{from}.mkv",
                std::process::id()
            ));
            fvid_media::transcode_lossless(
                &source,
                &output,
                fvid_media::LosslessTransform {
                    horizontal_flip: true,
                    interval: Some((from, to)),
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let data = std::fs::read(&output).unwrap();
            let mut out =
                fvid_media::owned_webm::WebmReader::open(Cursor::new(&data), Default::default())
                    .unwrap();
            out.scan_all().unwrap();
            let video = out.tracks.iter().find(|t| t.kind == 1).unwrap().clone();
            let selected: Vec<_> = c["pts_ns"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .filter(|(_, p)| (from * 1000..to * 1000).contains(&p.as_i64().unwrap()))
                .map(|(i, _)| i)
                .collect();
            let packets: Vec<_> = out
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == video.number)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(packets.len(), selected.len());
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
                video.width as usize,
                video.height as usize,
                &video.codec_private,
                32 << 20,
            )
            .unwrap();
            for (&packet, &frame) in packets.iter().zip(&selected) {
                assert_eq!(
                    out.packets[packet].pts_ns,
                    c["pts_ns"][frame].as_i64().unwrap() - from * 1000
                );
                assert_eq!(
                    out.packets[packet].duration_ns,
                    Some(c["durations_ns"][frame].as_u64().unwrap())
                );
                let decoded = decoder.decode(&out.read_packet(packet).unwrap()).unwrap();
                let gold =
                    std::fs::read(root().join(c["frames"][frame].as_str().unwrap())).unwrap();
                let mut expected = Vec::new();
                let unit = if decoded.depth == 8 { 1 } else { 2 };
                let mut at = 0;
                for (w, h) in [
                    (video.width as usize, video.height as usize),
                    (video.width as usize / 2, video.height as usize / 2),
                    (video.width as usize / 2, video.height as usize / 2),
                ] {
                    let len = w * h * unit;
                    for row in gold[at..at + len].chunks_exact(w * unit) {
                        for sample in row.chunks_exact(unit).rev() {
                            expected.extend_from_slice(sample)
                        }
                    }
                    at += len;
                }
                assert_eq!(decoded.frame.data, expected, "video interval pixels");
            }
            for (i, t) in input
                .tracks
                .clone()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.kind == 2)
            {
                let (mapped, track) = out
                    .tracks
                    .iter()
                    .enumerate()
                    .find(|(_, o)| o.codec == t.codec)
                    .unwrap();
                if t.codec == "A_OPUS" {
                    assert_eq!(&track.codec_private[..10], &t.codec_private[..10]);
                    assert_eq!(&track.codec_private[12..], &t.codec_private[12..]);
                } else {
                    assert_eq!(track.codec_private, t.codec_private);
                }
                let expected = pcm(
                    &bytes,
                    &t.codec,
                    i,
                    Some((
                        Duration::from_micros(from as u64),
                        Duration::from_micros(to as u64),
                    )),
                );
                let actual = pcm(&data, &t.codec, mapped, None);
                assert!(!expected.is_empty());
                assert_eq!(actual, expected, "{} {} {from}..{to}", c["file"], t.codec);
                let originals: Vec<_> = input
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == t.number)
                    .map(|(i, _)| i)
                    .collect();
                let retained: Vec<_> = out
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == track.number)
                    .map(|(i, _)| i)
                    .collect();
                assert!(retained.len() <= originals.len());
                for (ordinal, &j) in retained.iter().enumerate() {
                    assert_eq!(
                        out.read_packet(j).unwrap(),
                        input.read_packet(originals[ordinal]).unwrap(),
                        "compressed preroll bytes"
                    );
                }
            }
            std::fs::remove_file(output).unwrap();
        }
    }
}

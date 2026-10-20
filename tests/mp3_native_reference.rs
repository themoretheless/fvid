#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
#[test]
fn owned_layer3_matches_saved_synthetic_tone_and_reset() {
    let bytes = include_bytes!("fixtures/mp3/tone.mp3");
    let mut stream =
        fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
    let mut owned = fvid_codecs::mp3::Decoder::default();
    let mut reference = fvid::codec::symphonia_decoder::SymphoniaDecoder::new(
        "MP3 reference",
        symphonia::core::codecs::audio::well_known::CODEC_ID_MP3,
        &[],
        stream.sample_rate(),
        stream.channels(),
    )
    .unwrap();
    let mut count = 0;
    let mut squared = 0f64;
    let mut peak = 0f64;
    let mut packets = Vec::new();
    let mut samples = Vec::new();
    while let Some(packet) = stream.next_packet().unwrap() {
        let actual = owned.decode(&packet.data).unwrap();
        assert_eq!(actual.sample_rate, stream.sample_rate());
        assert_eq!(actual.channels, stream.channels());
        let expected = reference
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .unwrap()
            .unwrap();
        let expected: Vec<f32> = expected
            .data
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(actual.samples.len(), expected.len());
        for (a, b) in actual.samples.iter().zip(expected) {
            let e = f64::from(a - b);
            peak = peak.max(e.abs());
            squared += e * e;
            count += 1;
        }
        packets.push(packet.data);
        samples.push(actual.samples);
    }
    let rms = (squared / count as f64).sqrt();
    eprintln!("Owned MP3: {count} samples, peak {peak}, RMS {rms}");
    assert!(count >= 11520);
    assert!(peak < 5e-5 && rms < 5e-6, "peak {peak}, RMS {rms}");
    owned.reset();
    for (packet, expected) in packets.iter().zip(samples) {
        assert_eq!(owned.decode(packet).unwrap().samples, expected);
    }
}

#[test]
fn authored_video_companions_decode_video_and_exact_layer3_audio_packets() {
    use std::io::Cursor;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-decoder");
    for rate in [48000, 24000, 8000, 44100, 22050, 11025, 32000, 16000, 12000] {
        for channels in [1, 2] {
            let name = format!("{rate}-{channels}ch");
            let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
            {
                let mut video =
                    fvid::playback_webm::WebmVideoReader::open_software(Cursor::new(&bytes), 32 << 20)
                        .unwrap();
                let mut pictures = 0;
                while video.read_frame().unwrap() {
                    assert_eq!(video.dimensions(), [16, 16]);
                    assert!(video.rgb().iter().all(|&v| v == 130));
                    pictures += 1;
                }
                assert_eq!(pictures, 4);
                video.rewind();
                assert!(video.read_frame().unwrap());
            }
            let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            assert_eq!((stream.sample_rate(), stream.channels()), (rate, channels));
            assert_eq!(stream.codec(), "A_MPEG/L3");
            let expected = std::fs::read(root.join(format!("{name}.f32le"))).unwrap();
            let expected: Vec<f32> = expected
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let mut owned = fvid_codecs::mp3::Decoder::default();
            let mut pcm = Vec::new();
            while let Some(packet) = stream.next_packet().unwrap() {
                pcm.extend(owned.decode(&packet.data).unwrap().samples);
            }
            assert_eq!(pcm.len(), expected.len());
            for (a, b) in pcm.iter().zip(expected) {
                assert!((a - b).abs() < 3e-7, "{name}");
            }
        }
    }
}

#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
#[test]
fn linked_vorbis_opus_player_uses_native_pcm_and_exact_navigation() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (name, index) in [
        ("vorbis-opus", 0),
        ("opus-vorbis", 0),
        ("vorbis-opus-vorbis", 0),
        ("opus-vorbis-opus", 0),
        ("discrete-opus", 0),
        ("multiplex-swap", 0),
        ("multiplex-swap", 1),
    ] {
        let path = root.join(format!("ogg-codec-chain-{name}.ogg"));
        let bytes = std::fs::read(&path).unwrap();
        let mut library = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            index,
        )
        .unwrap();
        let mut expected = Vec::new();
        library.decode_pcm(&mut expected).unwrap();
        let mut stream = fvid::playback_ogg_catalog::Reader::open(&path, index).unwrap();
        assert_eq!(stream.codec(), "A_OGG/VORBIS_OPUS");
        assert_eq!(fvid::audio::codec_name(stream.codec()), "Vorbis/Opus");
        assert_eq!(stream.audio_track_index(), index);
        assert_eq!(stream.channels(), library.channels());
        assert_eq!(stream.sample_rate(), library.sample_rate());
        assert_eq!(stream.audio_tracks()[index].channels, stream.channels());
        let _decoder = stream.make_decoder().unwrap();
        let width = stream.channels() as usize * 4;
        let end = expected.len() / width;
        let boundary = if stream.sample_rate() == 48000 {
            5311
        } else {
            411
        };
        for target in [
            0,
            19,
            boundary - 1,
            boundary,
            boundary + 1,
            end - 1,
            end,
            end + 1,
            0,
        ] {
            assert_eq!(stream.seek_to(target as i64), target.min(end) as i64);
            let mut cursor = target.min(end) as u64;
            let mut actual = Vec::new();
            while let Some(step) = stream.next_step().unwrap() {
                let AudioStep::Pcm(pcm) = step else {
                    panic!("codec transitions are library-owned PCM steps")
                };
                assert_eq!(pcm.pts, cursor);
                assert_eq!(
                    (pcm.timebase_num, pcm.timebase_den),
                    (1, stream.timescale())
                );
                cursor += pcm.data.len() as u64 / width as u64;
                actual.extend(pcm.data);
            }
            assert_eq!(cursor, end as u64);
            assert_eq!(
                actual,
                expected[target.min(end) * width..],
                "{name} source {index} target {target}"
            );
        }
        stream.rewind();
        assert!(stream.next_step().unwrap().is_some());
    }
}

#[test]
fn linked_codec_origin_fixtures_keep_gap_navigation_and_matching_video_pcm() {
    use fvid::playback_webm_audio::WebmAudioReader;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (name, ogg, video, gap, end) in [
        (
            "initial-trim-opus",
            "vorbis-timestamps-initial-trim.ogg",
            "hevc-vorbis-initial-trim-tail.mkv",
            0,
            1280,
        ),
        (
            "positive-origin-opus",
            "vorbis-timestamps-positive-origin.ogg",
            "hevc-vorbis-positive-origin-tail.mkv",
            23,
            1320,
        ),
    ] {
        let bytes = std::fs::read(root.join(ogg)).unwrap();
        let mut baseline =
            fvid_media::owned_ogg_vorbis::Program::open(bytes.as_slice(), Default::default())
                .unwrap();
        let mut expected = Vec::new();
        baseline.decode_pcm(&mut expected).unwrap();
        let bytes = std::fs::read(root.join(video)).unwrap();
        let mut companion =
            WebmAudioReader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
        let mut decoder = companion.make_decoder().unwrap();
        let mut actual = Vec::new();
        let mut first = None;
        while let Some(packet) = companion.next_packet().unwrap() {
            if let Some(decoded) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(pcm) = companion
                    .present_decoded(decoded.packet, decoded.source_pts)
                    .unwrap()
                {
                    first.get_or_insert(pcm.pts);
                    actual.extend(pcm.data);
                }
            }
        }
        assert_eq!(first, Some(gap as u64 * 125000));
        assert_eq!(actual, expected);
        let path = root.join(format!("ogg-codec-chain-{name}.ogg"));
        let mut stream = fvid::playback_ogg_catalog::Reader::open(&path, 0).unwrap();
        for target in [0, 19, 23, 24, 393, 433, 434, 500, end - 1, end, 0] {
            assert_eq!(stream.seek_to(target), target);
            let mut cursor = target.max(gap) as u64;
            while let Some(step) = stream.next_step().unwrap() {
                let AudioStep::Pcm(pcm) = step else {
                    panic!("native PCM step")
                };
                assert_eq!(pcm.pts, cursor);
                cursor += pcm.data.len() as u64 / 24;
            }
            assert_eq!(cursor, end as u64);
        }
    }
}

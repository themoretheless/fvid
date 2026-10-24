use std::io::Cursor;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-framing-track")
}

fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}

#[test]
fn reproduction_reserved_flags_spurious_continuations_and_unended_vorbis_links() {
    let manifest = manifest();
    for entry in manifest["sha256"].as_object().unwrap() {
        let data = bytes(entry.0);
        assert!(data.len() <= 8192, "{} exceeds bounded fixture size", entry.0);
    }
    let expects_fixed = std::env::var_os("FVID_OGG_FRAMING_TRACK_PRE_FIX").is_none();

    for case in manifest["malformed_ogg"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let needle = case["error"].as_str().unwrap();
        let res = fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default());
        if expects_fixed {
            let err = res
                .expect_err(&format!("{name} must be refused"))
                .to_string();
            assert!(err.contains(needle), "{name}: expected {needle:?} in {err:?}");
        } else {
            assert!(
                res.is_ok(),
                "{name}: expected pre-fix parser to silently accept malformed page"
            );
        }
    }

    for case in manifest["malformed_vorbis"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let needle = case["error"].as_str().unwrap();
        let res =
            fvid_media::owned_ogg_vorbis::Program::open(Cursor::new(bytes(name)), Default::default());
        if expects_fixed {
            let err = match res {
                Ok(_) => panic!("{name} must be refused"),
                Err(e) => e.to_string(),
            };
            assert!(err.contains(needle), "{name}: expected {needle:?} in {err:?}");
        } else {
            assert!(
                res.is_ok(),
                "{name}: expected pre-fix Vorbis program to accept unended first link"
            );
        }
    }
    #[cfg(feature = "player")]
    {
        use fvid::audio::AudioStream;
        let reader = fvid::playback_ogg_opus::OggOpusAudioReader::open_track(
            Cursor::new(bytes("parallel-opus.ogg")),
            Default::default(),
            1,
        )
        .unwrap();
        if expects_fixed {
            assert_eq!(reader.audio_track_index(), 1);
        } else {
            assert_eq!(
                reader.audio_track_index(),
                0,
                "expected pre-fix OggOpusAudioReader::audio_track_index() to default to 0"
            );
        }
    }
}

#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;

#[cfg(feature = "player")]
#[test]
fn acceptance_ogg_framing_vorbis_chain_eos_and_opus_track_index() {
    use fvid::audio::{AudioStep, AudioStream};
    use std::time::Duration;

    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("video.mkv")),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());

    // 1. Malformed Ogg framing and unended Vorbis chain must be refused before playback admission.
    for case in manifest()["malformed_ogg"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let needle = case["error"].as_str().unwrap();
        let err = match fvid::playback_ogg_opus::OggOpusAudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => panic!("{name} must be refused before audio playback"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains(needle), "{name}: expected {needle:?} in {err:?}");
    }
    for case in manifest()["malformed_vorbis"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let needle = case["error"].as_str().unwrap();
        let err = match fvid::playback_ogg_audio::OggAudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        ) {
            Ok(_) => panic!("{name} must be refused before Vorbis playback"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains(needle), "{name}: expected {needle:?} in {err:?}");
    }

    // 2. Healthy Vorbis chained control with EOS on both links decodes and seeks cleanly.
    let mut vorbis = fvid::playback_ogg_audio::OggAudioReader::open(
        Cursor::new(bytes("vorbis-chained-control.ogg")),
        Default::default(),
    )
    .unwrap();
    assert_eq!(vorbis.audio_track_index(), 0);
    assert_eq!(vorbis.duration(), Some(Duration::from_nanos(822 * 1_000_000_000 / 8000)));
    let mut vorbis_dec = vorbis.make_decoder().unwrap();
    let mut vorbis_frames = 0usize;
    while let Some(step) = vorbis.next_step().unwrap() {
        match step {
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => vorbis.reset_decoder(&mut vorbis_dec).unwrap(),
            AudioStep::Pcm(p) => vorbis_frames += p.data.len() / 8,
            AudioStep::Encoded(p) => {
                if let Some(out) = vorbis_dec
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(pcm) = vorbis.present_decoded(out.packet, p.pts).unwrap() {
                        vorbis_frames += pcm.data.len() / 8;
                    }
                }
            }
        }
    }
    assert_eq!(vorbis_frames, 822);

    // 3. Healthy 0-segment continuation bridge page preserves split packet and exact PCM/seek.
    let bridge = fvid_media::owned_ogg::Ogg::parse(
        &bytes("continued-empty-bridge.ogg"),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(bridge.chains().len(), 1);
    assert_eq!(bridge.chains()[0].packets.len(), 3);
    assert_eq!(bridge.chains()[0].packets[2].data, bytes("split20.packet"));
    let mut bridge_prog = fvid_media::owned_ogg_opus::Program::open(
        Cursor::new(bytes("continued-empty-bridge.ogg")),
        Default::default(),
    )
    .unwrap();
    let mut bridge_pcm = Vec::new();
    let bridge_stats = bridge_prog.decode_pcm(&mut bridge_pcm).unwrap();
    assert_eq!(bridge_stats.sample_frames, 960);
    assert_eq!(bridge_pcm.len(), 960 * 4);

    // 4. Multi-track Opus reports selected audio_track_index() on both track 0 and track 1,
    // and matches independent reference PCM across playback, mid-stream seek, and EOF seek.
    let parallel = bytes("parallel-opus.ogg");
    for (nth, channels, head_name, pkt_name, pkt_frames) in [
        (0usize, 1u16, "mono.head", "raw20.packet", 960u64),
        (1usize, 2u16, "stereo.head", "raw10.packet", 480u64),
    ] {
        let prog = fvid_media::owned_ogg_opus::Program::open_track(
            Cursor::new(&parallel),
            Default::default(),
            nth,
        )
        .unwrap();
        assert_eq!(prog.audio_tracks().len(), 2);
        assert_eq!(prog.audio_track_index(), nth);
        assert_eq!(prog.channels(), channels);

        let prog_src = fvid_media::owned_ogg_opus::Program::open_source_track(
            Cursor::new(&parallel),
            Default::default(),
            Some(nth),
        )
        .unwrap();
        assert_eq!(prog_src.audio_track_index(), nth);
        assert_eq!(prog_src.channels(), channels);

        let reader = fvid::playback_ogg_opus::OggOpusAudioReader::open_track(
            Cursor::new(&parallel),
            Default::default(),
            nth,
        )
        .unwrap();
        assert_eq!(reader.audio_tracks().len(), 2);
        assert_eq!(reader.audio_track_index(), nth);
        assert_eq!(reader.channels(), channels);

        let mut ref_dec = fvid::codec::make_audio_decoder(
            "A_OPUS",
            &bytes(head_name),
            48000,
            channels,
            0,
        )
        .unwrap();
        let mut expected = Vec::new();
        for idx in 0..2u64 {
            let pts_ns = (idx * pkt_frames * 1_000_000_000 / 48000) as i64;
            let dur_ns = pkt_frames * 1_000_000_000 / 48000;
            let out = ref_dec
                .decode_packet(&bytes(pkt_name), pts_ns, dur_ns)
                .unwrap()
                .unwrap();
            expected.push((
                Duration::from_nanos(pts_ns as u64),
                out.packet.data,
            ));
        }
        let mid = Duration::from_nanos(pkt_frames * 1_000_000_000 / 48000);
        let end = Duration::from_nanos(2 * pkt_frames * 1_000_000_000 / 48000);
        assert_eq!(reader.duration(), Some(end));
        capture::worker_check_with_seek_expected(
            Box::new(
                fvid::playback_ogg_opus::OggOpusAudioReader::open_track(
                    Cursor::new(&parallel),
                    Default::default(),
                    nth,
                )
                .unwrap(),
            ),
            &expected,
            mid,
            &expected[1..],
        );
        capture::worker_check_with_seek(
            Box::new(reader),
            &expected,
            Some(end),
        );
    }
}

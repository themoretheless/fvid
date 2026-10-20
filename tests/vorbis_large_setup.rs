#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn check(variant: &str) {
    let bytes = fixture(&format!("vorbis-large-{variant}.ogg"));
    // Independently establish valid framing, setup admission and native PCM
    // before entering the player's former fixed quota.
    let mut library =
        fvid_media::owned_ogg_vorbis::Program::open(bytes.as_slice(), Default::default()).unwrap();
    let setup = &library.ogg().chains()[0].packets[2].data;
    let admission = fvid_codecs::vorbis_admission::inspect(setup).unwrap();
    assert!(if variant == "entries" {
        admission.limits.entries > 1_000_000
    } else {
        admission.limits.multiplicands > 1_000_000
    });
    assert_eq!(
        admission.limits.nodes, 1,
        "only the single active codeword belongs to the tree"
    );
    assert!(
        admission.bytes < 5 * 1024 * 1024,
        "large declared counts need modest actual setup storage"
    );
    let mut expected = Vec::new();
    library.decode_pcm(&mut expected).unwrap();
    assert_eq!(expected, vec![0; 411 * 8]);
    for container in ["ogg", "mkv"] {
        let mut stream: Box<dyn AudioStream> = if container == "ogg" {
            Box::new(
                fvid::playback_ogg_audio::OggAudioReader::open(
                    bytes.as_slice(),
                    Default::default(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    std::io::Cursor::new(fixture(&format!("hevc-vorbis-large-{variant}.mkv"))),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        for target in [0usize, 19, 390, 411, 0] {
            let ticks = target as i64 * i64::from(stream.timescale()) / 8000;
            assert_eq!(stream.seek_to(ticks), ticks);
            let mut decoder = stream.make_decoder().unwrap();
            let mut pcm = Vec::new();
            while let Some(step) = stream.next_step().unwrap() {
                match step {
                    AudioStep::ResetDecoder => stream.reset_decoder(&mut decoder).unwrap(),
                    AudioStep::Encoded(packet) => {
                        if let Some(decoded) = decoder
                            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                            .unwrap()
                        {
                            if let Some(presented) = stream
                                .present_decoded(decoded.packet, decoded.source_pts)
                                .unwrap()
                            {
                                assert!(presented.pts >= ticks as u64);
                                pcm.extend(presented.data);
                            }
                        }
                    }
                    _ => panic!("unexpected sparse Vorbis step"),
                }
            }
            assert_eq!(
                pcm,
                expected[target * 8..],
                "{variant} {container} target {target}"
            );
        }
    }
}

#[test]
fn large_sparse_vorbis_entries_reach_both_player_containers() {
    check("entries");
}
#[test]
fn large_vorbis_lookup_values_reach_both_player_containers() {
    check("multiplicands");
}

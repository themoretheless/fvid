//! The layered owner must retain ordinary base-only I/P/B reconstruction too.
use fvid::{
    codec::{
        hevc_cabac::SliceType,
        hevc_decoder::{HevcDecoder, LayeredDecoder, LayeredOutputQueue},
    },
    container::webm::WebmReader,
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn inferred_base_dpb_reconstructs_reordered_inter_pictures_against_owned_oracles() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("hevc-matroska-reorder.json")).unwrap())
            .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 16);
    let short: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-inferred-base-inter/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(short["cases"].as_array().unwrap().len(), 2);
    for c in short["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["cases"].as_array().unwrap())
    {
        let name = c["file"].as_str().unwrap();
        let mut source = WebmReader::open(
            Cursor::new(std::fs::read(root().join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        source.scan_all().unwrap();
        let track = source.tracks.iter().find(|t| t.kind == 1).unwrap();
        let (number, cfg) = (track.number, track.codec_private.clone());
        let headers = HevcDecoder::from_configuration(&cfg, BUDGET).unwrap();
        let tid = (headers.parameters().0.ordering.len() - 1) as u8;
        let mut packets = Vec::new();
        let mut has_b = false;
        let mut has_p = false;
        let mut idr_packets = 0;
        for i in 0..source.packets.len() {
            if source.packets[i].track == number {
                let packet = source.read_packet(i).unwrap();
                let slice_headers = headers.slice_headers(&packet).unwrap();
                idr_packets += usize::from(slice_headers.iter().any(|h| h.nal.is_idr()));
                for h in slice_headers {
                    has_b |= h.slice_type == SliceType::B;
                    has_p |= h.slice_type == SliceType::P;
                }
                packets.push(packet);
            }
        }
        assert!(
            has_b && has_p,
            "{name}: witness must exercise real P and B syntax"
        );
        assert!(
            idr_packets >= 2,
            "{name}: witness needs repeated IDR boundaries"
        );
        let expected: Vec<_> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| std::fs::read(root().join(f.as_str().unwrap())).unwrap())
            .collect();
        let mut decoder = LayeredDecoder::from_configuration(&cfg, 0, BUDGET).unwrap();
        let mut queue = LayeredOutputQueue::default();
        for _ in 0..2 {
            let mut output = Vec::new();
            for (i, packet) in packets.iter().enumerate() {
                let (frames, _) = decoder
                    .decode_queued_packet(&mut queue, packet, tid)
                    .unwrap_or_else(|e| {
                        panic!(
                            "{name} packet{i}, SPS ordering {:?}, queued {:?}: {e}",
                            headers.parameters().0.ordering,
                            queue
                                .clone()
                                .drain()
                                .iter()
                                .map(|p| p.poc)
                                .collect::<Vec<_>>()
                        )
                    });
                output.extend(frames);
            }
            output.extend(queue.drain());
            assert_eq!(output.len(), expected.len(), "{name}");
            assert!(
                output.windows(2).any(|p| p[1].poc < p[0].poc),
                "{name}: the old epoch must end before a new lower POC"
            );
            for (p, expected) in output.iter().zip(&expected) {
                assert_eq!(p.layer_id, 0);
                assert!(p.binding.vps.extension.is_none());
                assert!(p.signalled_output_layer().unwrap());
                let time = fvid::playback::video::mp4::MediaTime {
                    ticks: 0,
                    timescale: 1,
                };
                let frame = fvid::playback::video::mp4::LayeredVideoFrame {
                    decoded: p.clone(),
                    sample_index: 0,
                    presentation_time: time,
                    duration: time,
                }
                .into_video_frame()
                .unwrap();
                let packed = frame.packed.unwrap();
                assert_eq!(&packed.frame.data, expected, "{name} POC{}", p.poc);
            }
            decoder.reset_queued(&mut queue);
        }
    }
}

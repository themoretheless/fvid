#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use fvid::audio::{AudioDecode, AudioStream};
use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};
fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-checkpoint")
}
struct Tracked {
    inner: Box<dyn AudioStream>,
    resumed: Arc<AtomicI64>,
    reads: Arc<AtomicUsize>,
}
impl AudioStream for Tracked {
    fn codec(&self) -> &str {
        self.inner.codec()
    }
    fn timescale(&self) -> u32 {
        self.inner.timescale()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn extra_data(&self) -> &[u8] {
        self.inner.extra_data()
    }
    fn audio_tracks(&self) -> Vec<fvid::audio::AudioTrack> {
        self.inner.audio_tracks()
    }
    fn duration(&self) -> Option<Duration> {
        self.inner.duration()
    }
    fn preroll_target(&self) -> Option<i64> {
        self.inner.preroll_target()
    }
    fn resume_preroll(&mut self, pts: i64) -> bool {
        if !self.inner.resume_preroll(pts) {
            return false;
        }
        self.resumed.store(pts, Ordering::SeqCst);
        self.reads.store(0, Ordering::SeqCst);
        true
    }
    fn present_decoded(
        &self,
        p: fvid::audio::AudioPacket,
        pts: i64,
    ) -> fvid::Result<Option<fvid::audio::AudioPacket>> {
        self.inner.present_decoded(p, pts)
    }
    fn next_packet(&mut self) -> fvid::Result<Option<fvid::audio::EncodedPacket>> {
        let packet = self.inner.next_packet()?;
        if packet.is_some() {
            self.reads.fetch_add(1, Ordering::SeqCst);
        }
        Ok(packet)
    }
    fn rewind(&mut self) {
        self.inner.rewind();
    }
    fn seek_to(&mut self, pts: i64) -> i64 {
        self.inner.seek_to(pts)
    }
}
#[test]
fn worker_restores_raw_and_matroska_clocks_without_redecoding_entire_prefix() {
    for (name, raw) in [("long", true), ("long", false), ("fractional", false)] {
        let bytes =
            std::fs::read(root().join(format!("{name}.{}", if raw { "mp3" } else { "mkv" })))
                .unwrap();
        let mut reader: Box<dyn AudioStream> = if raw {
            Box::new(
                fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default())
                    .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    std::io::Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let mut decoder = reader.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let pcm = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
                .unwrap();
            expected.push((pcm.presentation_time(), pcm.data));
        }
        assert_eq!(expected.len(), 160);
        let (checkpoint, reads_expected) = if name == "long" { (84, 76) } else { (77, 83) };
        let expected_resume = if raw {
            96768
        } else {
            expected[checkpoint].0.as_nanos() as i64
        };
        let target = expected[100].0;
        reader.rewind();
        let resumed = Arc::new(AtomicI64::new(-1));
        let reads = Arc::new(AtomicUsize::new(0));
        let tracked = Tracked {
            inner: reader,
            resumed: resumed.clone(),
            reads: reads.clone(),
        };
        audio_capture::worker_check_with_seek(Box::new(tracked), &expected, Some(target));
        assert_eq!(
            resumed.load(Ordering::SeqCst),
            expected_resume,
            "{name} raw {raw}"
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            reads_expected,
            "{name} raw {raw}"
        );
    }
    for name in ["long", "fractional", "backwards"] {
        let bytes = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
        let mut video = fvid::playback_webm::WebmVideoReader::open_software(
            std::io::Cursor::new(bytes),
            32 << 20,
        )
        .unwrap();
        let mut count = 0;
        while video.read_frame().unwrap() {
            assert_eq!(video.dimensions(), [16, 16]);
            count += 1;
        }
        assert_eq!(count, 160);
    }
}
#[test]
fn backwards_clock_keeps_full_replay_and_all_presented_occurrences() {
    let bytes = std::fs::read(root().join("backwards.mkv")).unwrap();
    let mut stream = fvid::playback_webm_audio::WebmAudioReader::open(
        std::io::Cursor::new(bytes),
        Default::default(),
    )
    .unwrap();
    let mut decoder = stream.make_decoder().unwrap();
    let mut baseline = Vec::new();
    let mut late_snapshot = None;
    while let Some(p) = stream.next_packet().unwrap() {
        if baseline.len() == 84 {
            late_snapshot = decoder.checkpoint();
        }
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        let pcm = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
            .unwrap();
        baseline.push((p.pts, pcm.data));
    }
    assert_eq!(baseline.len(), 160);
    assert!(baseline.windows(2).any(|p| p[1].0 < p[0].0));
    assert_eq!(stream.preroll_target(), None);
    let floor = 1_200_000_000;
    stream.seek_to(floor);
    decoder.reset();
    assert_eq!(baseline[84].0, 1_016_000_000);
    assert!(baseline[..84].iter().any(|(pts, _)| *pts > floor));
    assert!(!stream.resume_preroll(1_016_000_000));
    let mut expected = Vec::new();
    for (pts, data) in baseline {
        let frames = data.len() / 4;
        let skip = ((i128::from(floor) - i128::from(pts)).max(0) as u128 * 48000)
            .div_ceil(1_000_000_000)
            .min(frames as u128) as usize;
        expected.extend_from_slice(&data[skip * 4..]);
    }
    let mut actual = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            actual.extend(pcm.data);
        }
    }
    assert_eq!(actual, expected);
    // Reproduce the incorrect timestamp-only shortcut: a later low-PTS
    // checkpoint skips earlier high-PTS occurrences that should be presented.
    stream.seek_to(floor);
    assert_eq!(stream.seek(1_016_000_000), 1_016_000_000);
    decoder.restore(&late_snapshot.unwrap()).unwrap();
    let mut wrong = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            wrong.extend(pcm.data);
        }
    }
    assert_ne!(
        wrong, expected,
        "fixture must expose lost earlier high-PTS occurrences"
    );
    assert_eq!(expected.len() - wrong.len(), 40 * 1152 * 4);
    assert_eq!(wrong, expected[40 * 1152 * 4..]);
}
#[test]
fn checkpoints_preserve_real_reservoir_overlap_and_reject_mismatched_geometry() {
    let sources = [
        include_bytes!("fixtures/mp3/tone.mp3").to_vec(),
        std::fs::read(
            root()
                .parent()
                .unwrap()
                .join("mp3-free/48000-1ch-control.mp3"),
        )
        .unwrap(),
    ];
    for bytes in sources {
        let mut reader =
            fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        let mut decoder = reader.make_decoder().unwrap();
        let mut packets = Vec::new();
        let mut snapshots = Vec::new();
        let mut expected = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            snapshots.push(decoder.checkpoint().unwrap());
            expected.push(
                decoder
                    .decode_encoded(&p.data, p.pts as u64, p.duration as u64)
                    .unwrap()
                    .unwrap()
                    .data,
            );
            packets.push(p);
        }
        for start in (0..packets.len()).rev() {
            decoder.reset();
            decoder.restore(&snapshots[start]).unwrap();
            for i in start..packets.len() {
                assert_eq!(
                    decoder
                        .decode_encoded(
                            &packets[i].data,
                            packets[i].pts as u64,
                            packets[i].duration as u64
                        )
                        .unwrap()
                        .unwrap()
                        .data,
                    expected[i]
                );
            }
        }
        let mut other = fvid::codec::mp3_decoder::Mp3Decoder::new(44100, 2).unwrap();
        assert!(other.restore(&snapshots[1]).is_err());
        reader.seek_to(2400);
        assert!(!reader.resume_preroll(2305));
        let after = (reader.preroll_target().unwrap() / 1152 + 1) * 1152;
        assert!(!reader.resume_preroll(after));
        assert!(!reader.resume_preroll(-1));
        assert!(reader.resume_preroll(2304));
        assert_eq!(reader.next_packet().unwrap().unwrap().pts, 2304);
        decoder.reset();
        decoder.restore(&snapshots[2]).unwrap();
        let setup = fvid::container::adts::esds_for(&[0x12, 0x10]).unwrap();
        let mut aac = fvid::codec::aac_decoder::AacDecoder::new(&setup, 44100, 2).unwrap();
        let aac_state = aac.checkpoint().unwrap();
        assert!(
            decoder
                .restore(&aac_state)
                .unwrap_err()
                .to_string()
                .contains("codec mismatch")
        );
        assert!(
            aac.restore(&snapshots[2])
                .unwrap_err()
                .to_string()
                .contains("codec mismatch")
        );
        assert_eq!(
            decoder
                .decode_encoded(
                    &packets[2].data,
                    packets[2].pts as u64,
                    packets[2].duration as u64
                )
                .unwrap()
                .unwrap()
                .data,
            expected[2]
        );
    }
}

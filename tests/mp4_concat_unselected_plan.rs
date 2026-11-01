use fvid::{
    container::{mp4_concat, webm::WebmReader},
    media::CopyOptions,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-unselected-plan")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
const CASES: [&str; 3] = ["exact.mp4", "coarse-1000.mp4", "coarse-1001.mp4"];
#[test]
fn native_sources_keep_exact_video_and_real_decodable_aac_clocks() {
    for n in CASES {
        let data = bytes(n);
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(&data),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        let frame = r.read_frame().unwrap().unwrap();
        assert_eq!(
            (frame.presentation_time.ticks, frame.duration.ticks),
            (0, 1)
        );
        let y: Vec<_> = frame
            .picture
            .y
            .iter()
            .chain(&frame.picture.cb)
            .chain(&frame.picture.cr)
            .flat_map(|x| x.to_le_bytes())
            .collect();
        assert_eq!(y, bytes("video.yuv16"));
        assert!(r.read_frame().unwrap().is_none());
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 3072);
        let expected = bytes("audio.f32le");
        assert_eq!(pcm.len(), expected.len());
        for (a, b) in pcm.chunks_exact(4).zip(expected.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(a.is_finite() && (a - b).abs() < 2e-6, "{n}:{a}!={b}");
        }
    }
}
#[test]
fn excluded_coarse_aac_cannot_block_selected_video_concat_and_seek() {
    let mut failures = Vec::new();
    for n in CASES {
        let path = root().join(n);
        let options = CopyOptions {
            streams: vec![0],
            ..Default::default()
        };
        let mut segments = match mp4_concat::open_with_options(&[path.clone(), path], &options) {
            Ok(Some(s)) => s,
            other => {
                failures.push(format!(
                    "{n}:{}",
                    other.err().map_or("declined".into(), |e| e.to_string())
                ));
                continue;
            }
        };
        let mut out = Cursor::new(Vec::new());
        assert_eq!(
            mp4_concat::write_with_options(&mut segments, &mut out, &options)
                .unwrap()
                .packets,
            2
        );
        let data = out.into_inner();
        let mut demux = WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
        demux.scan_all().unwrap();
        assert_eq!(demux.tracks.len(), 1);
        assert_eq!(demux.packets.len(), 2);
        assert_eq!(
            (demux.packets[0].pts_ns, demux.packets[1].pts_ns),
            (0, 40_000_000)
        );
        let mut source =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        let expected = source
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        assert_eq!(video.duration(), Some(Duration::from_millis(80)));
        for _ in 0..2 {
            assert_eq!(
                video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                expected
            );
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        video.rewind().unwrap();
        for ms in [0, 39, 40, 79, 80, 120] {
            assert_eq!(
                video
                    .seek_raw(Duration::from_millis(ms))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                expected
            );
            let first = if ms < 40 { 0 } else { 40_000_000 };
            assert_eq!(
                video.frame_interval(),
                Some((first, first + 40_000_000, 1_000_000_000))
            );
        }
    }
    assert!(
        failures.is_empty(),
        "excluded audio blocked video concat:{failures:?}"
    );
}
#[test]
fn selected_coarse_audio_still_requires_a_supported_remux_clock() {
    for n in &CASES[1..] {
        let path = root().join(n);
        let error =
            match mp4_concat::open_with_options(&[path.clone(), path], &CopyOptions::default()) {
                Err(e) => e,
                Ok(_) => panic!("selected coarse AAC silently admitted"),
            };
        assert!(
            error.to_string().contains(if n.contains("1000") {
                "AAC packet remux requires a contiguous frame clock"
            } else {
                "AAC time is not sample aligned"
            }),
            "{n}:{error}"
        );
    }
    let path = root().join(CASES[0]);
    let mut segments =
        mp4_concat::open_with_options(&[path.clone(), path], &CopyOptions::default())
            .unwrap()
            .unwrap();
    let mut out = Cursor::new(Vec::new());
    assert_eq!(
        mp4_concat::write_with_options(&mut segments, &mut out, &CopyOptions::default())
            .unwrap()
            .packets,
        8
    );
}

#[test]
fn writing_an_unadmitted_track_refuses_before_output() {
    let path = root().join("coarse-1001.mp4");
    let selected = CopyOptions {
        streams: vec![0],
        ..Default::default()
    };
    let mut segments = mp4_concat::open_with_options(&[path.clone(), path], &selected)
        .unwrap()
        .unwrap();
    let mut output = Cursor::new(Vec::new());
    assert!(
        mp4_concat::write_with_options(&mut segments, &mut output, &CopyOptions::default())
            .is_err(),
        "expanded writer selection was not admitted"
    );
    assert!(
        output.get_ref().is_empty(),
        "unadmitted selection wrote a partial header"
    );
}

#[test]
fn public_plan_and_publication_respect_selected_video_and_packet_prefix() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-unselected-plan-public-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for (n, name) in CASES.into_iter().enumerate() {
        for (limit_index, limit) in [None, Some(1)].into_iter().enumerate() {
            let path = root().join(name);
            let inputs = [path.clone(), path];
            let options = CopyOptions {
                streams: vec![0],
                max_packets: limit,
                ..Default::default()
            };
            let plan = fvid::media::plan_concat(&inputs, &options)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(plan.streams.len(), 1);
            let output = dir.join(format!("{n}-{limit_index}.mkv"));
            let stats = fvid::media::concat(&inputs, &output, &options).unwrap();
            assert_eq!(stats.packets, limit.unwrap_or(2));
            let mut demux = WebmReader::open(
                Cursor::new(std::fs::read(output).unwrap()),
                Default::default(),
            )
            .unwrap();
            demux.scan_all().unwrap();
            assert_eq!(demux.tracks.len(), 1);
            assert_eq!(demux.packets.len() as u64, stats.packets);
            assert_eq!(demux.packets[0].pts_ns, 0);
            if stats.packets == 2 {
                assert_eq!(demux.packets[1].pts_ns, 40_000_000);
            }
        }
    }
}

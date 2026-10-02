use fvid::{
    container::{mp4, mp4_matroska, webm},
    native_geometry::VideoGeometry,
    playback_native::NativeReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn remux(bytes: &[u8]) -> Vec<u8> {
    let mut input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    let stats = mp4_matroska::write(&mut input, &mut output, None, None).unwrap();
    assert!(!stats.done);
    assert!(stats.packets > 0);
    output.into_inner()
}
fn pixels(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<(u64, u64)>) {
    let mut reader = NativeReader::software(Cursor::new(bytes), usize::MAX).unwrap();
    let mut frames = Vec::new();
    let mut times = Vec::new();
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        frames.push(
            VideoGeometry::default()
                .apply_display(&frame, w, h, reader.rotation())
                .unwrap()
                .data,
        );
        let (a, b, scale) = reader.frame_interval().unwrap();
        times.push((
            (a * 1_000_000_000 / u128::from(scale)) as u64,
            (b * 1_000_000_000 / u128::from(scale)) as u64,
        ));
    }
    (frames, times)
}
fn compare_video(bytes: &[u8], result: &[u8]) {
    let (expected, source_times) = pixels(bytes);
    let (actual, result_times) = pixels(result);
    assert_eq!(expected.len(), actual.len());
    assert!(expected == actual, "decoded video pixels differ");
    let origin = source_times[0].0;
    assert_eq!(
        result_times,
        source_times
            .iter()
            .map(|(a, b)| (a - origin, b - origin))
            .collect::<Vec<_>>()
    );
    let mut player = NativeReader::software(Cursor::new(result), usize::MAX).unwrap();
    for i in [0, actual.len() / 2, actual.len() - 1, 0] {
        let frame = player
            .seek_raw(Duration::from_nanos(result_times[i].0))
            .unwrap()
            .unwrap();
        let [w, h] = player.dimensions();
        assert!(
            VideoGeometry::default()
                .apply_display(&frame, w, h, player.rotation())
                .unwrap()
                .data
                == actual[i],
            "seek {i}"
        );
    }
}
#[test]
fn owned_remux_preserves_video_edits_frames_and_metadata() {
    for name in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "hevc/hdr10.mp4",
        "display/par-2x1.mp4",
        "audio/two-audio.mp4",
    ] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let output = remux(&bytes);
        compare_video(&bytes, &output);
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut mkv = webm::WebmReader::open(Cursor::new(&output), Default::default()).unwrap();
        mkv.scan_all().unwrap();
        assert_eq!(input.tracks().len(), mkv.tracks.len(), "{name}");
        assert_eq!(input.tags(), &mkv.tags);
        let mut next = vec![0usize; input.tracks().len()];
        let mut previous_dts = i128::MIN;
        for packet in &mkv.packets {
            let ti = packet.track as usize - 1;
            let track = &input.tracks()[ti];
            let sample = track.samples.get(next[ti]).unwrap();
            next[ti] += 1;
            let origin = track.edits.first().map_or(0, |e| i128::from(e.media_time));
            let pts =
                (i128::from(sample.pts) - origin) * 1_000_000_000 / i128::from(track.timescale);
            assert!(
                (i128::from(packet.pts_ns) - i128::from(mkv.tracks[ti].codec_delay_ns) - pts).abs()
                    <= 1,
                "{name}: track {ti} PTS {} expected {pts}, sample {} origin {origin} scale {}",
                packet.pts_ns,
                sample.pts,
                track.timescale
            );
            let dts =
                (i128::from(sample.dts) - origin) * 1_000_000_000 / i128::from(track.timescale);
            assert!(
                dts >= previous_dts.saturating_sub(1),
                "{name}: interleave order"
            );
            previous_dts = dts;
        }
        for (i, track) in input.tracks().iter().enumerate() {
            assert_eq!(mkv.tracks[i].name, track.name);
            let packets: Vec<_> = mkv
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == i as u64 + 1)
                .map(|(j, _)| j)
                .collect();
            assert!(!packets.is_empty());
            let mut src = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
            for (index, packet) in packets.into_iter().enumerate() {
                let mut data = Vec::new();
                src.read_packet(i, index, &mut data).unwrap();
                assert_eq!(
                    mkv.read_packet(packet).unwrap(),
                    data,
                    "{name} track {i} packet {index}"
                );
            }
        }
    }
}
#[test]
fn edited_video_keeps_invisible_dependencies_and_exact_window() {
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let mut bytes = std::fs::read(fixture(name)).unwrap();
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let track = input
            .tracks()
            .iter()
            .find(|t| t.handler == *b"vide")
            .unwrap();
        let mut times: Vec<_> = (0..track.samples.len())
            .map(|i| track.samples.get(i).unwrap().pts)
            .collect();
        times.sort();
        let begin = times[2];
        let end = times[times.len() - 2];
        let duration = u32::try_from(
            (end - begin) as u128 * u128::from(input.movie_timescale())
                / u128::from(track.timescale),
        )
        .unwrap();
        if !bytes.windows(4).any(|w| w == b"elst") {
            fn find_box(bytes: &[u8], mut at: usize, end: usize, id: &[u8; 4]) -> (usize, usize) {
                while at < end {
                    let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
                    assert!(size >= 8);
                    if &bytes[at + 4..at + 8] == id {
                        return (at, size);
                    }
                    at += size;
                }
                panic!("missing box");
            }
            let (moov, ml) = find_box(&bytes, 0, bytes.len(), b"moov");
            let (mdat, _) = find_box(&bytes, 0, bytes.len(), b"mdat");
            let (trak, tl) = find_box(&bytes, moov + 8, moov + ml, b"trak");
            let mut edit = 36u32.to_be_bytes().to_vec();
            edit.extend(b"edts");
            edit.extend(28u32.to_be_bytes());
            edit.extend(b"elst");
            edit.extend([0; 4]);
            edit.extend(1u32.to_be_bytes());
            edit.extend([0; 8]);
            edit.extend(0x10000u32.to_be_bytes());
            bytes[moov..moov + 4].copy_from_slice(&((ml + 36) as u32).to_be_bytes());
            bytes[trak..trak + 4].copy_from_slice(&((tl + 36) as u32).to_be_bytes());
            if moov < mdat {
                let offsets: Vec<_> = bytes[moov..moov + ml]
                    .windows(4)
                    .enumerate()
                    .filter(|(_, id)| *id == b"stco" || *id == b"co64")
                    .map(|(i, _)| moov + i)
                    .collect();
                assert!(!offsets.is_empty());
                for at in offsets {
                    let wide = &bytes[at..at + 4] == b"co64";
                    let count =
                        u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
                    for i in 0..count {
                        let pos = at + 12 + i * if wide { 8 } else { 4 };
                        if wide {
                            let value = u64::from_be_bytes(bytes[pos..pos + 8].try_into().unwrap());
                            bytes[pos..pos + 8].copy_from_slice(&(value + 36).to_be_bytes());
                        } else {
                            let value = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap());
                            bytes[pos..pos + 4].copy_from_slice(&(value + 36).to_be_bytes());
                        }
                    }
                }
            }
            bytes.splice(trak + tl..trak + tl, edit);
        }
        let at = bytes.windows(4).position(|w| w == b"elst").unwrap();
        assert_eq!(bytes[at + 4], 0);
        assert_eq!(&bytes[at + 8..at + 12], &1u32.to_be_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&duration.to_be_bytes());
        bytes[at + 16..at + 20].copy_from_slice(&i32::try_from(begin).unwrap().to_be_bytes());
        let output = remux(&bytes);
        compare_video(&bytes, &output);
        let mut mkv = webm::WebmReader::open(Cursor::new(&output), Default::default()).unwrap();
        mkv.scan_all().unwrap();
        assert!(mkv.packets.iter().any(|p| p.invisible));
        assert!(mkv.packets[0].invisible && mkv.packets[0].keyframe);
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory(label: &str) -> Directory {
    let p = std::env::temp_dir().join(format!("fvid-mp4-matroska-{label}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Directory(p)
}
#[test]
fn cli_and_api_remux_all_tracks_with_atomic_publication() {
    let dir = directory("publish");
    let source = fixture("audio/two-audio.mp4");
    assert!(fvid::native_export::is_native_mp4_matroska(&source).unwrap());
    let destination = dir.0.join("all.mkv");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "remux"])
        .arg(&source)
        .arg(&destination)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"backend\":\"fvid\""));
    assert!(
        String::from_utf8_lossy(&run.stderr)
            .lines()
            .last()
            .unwrap()
            .contains("\"done\":true")
    );
    let bytes = std::fs::read(&source).unwrap();
    let result = std::fs::read(&destination).unwrap();
    assert_eq!(result, remux(&bytes));
    let mut reader = webm::WebmReader::open(Cursor::new(&result), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 3);
    assert!(fvid::native_export::remux_mp4_matroska(&source, &destination, None, None).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), result);
    let cancel = fvid::media_control::CancelFlag::default();
    let stop = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 2 {
            stop.cancel();
        }
    });
    let cancelled = dir.0.join("cancelled.mkv");
    assert!(
        fvid::native_export::remux_mp4_matroska(&source, &cancelled, Some(&cancel), Some(&hook))
            .is_err()
    );
    assert!(!cancelled.exists());
    assert!(
        !std::fs::read_dir(&dir.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    #[cfg(feature = "media")]
    {
        let api = dir.0.join("api.mkv");
        assert_eq!(
            fvid::media::remux(&source, &api, &Default::default())
                .unwrap()
                .backend,
            "fvid"
        );
        assert_eq!(std::fs::read(api).unwrap(), result);
    }
}
#[test]
fn rotation_and_anamorphic_pixels_survive_mp4_matroska_mapping() {
    for rotation in [90, 180, 270] {
        let mut bytes = std::fs::read(fixture("display/par-2x1.mp4")).unwrap();
        let matrix = bytes.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
        let values: [i32; 4] = match rotation {
            90 => [0, 65536, -65536, 0],
            180 => [-65536, 0, 0, -65536],
            _ => [0, -65536, 65536, 0],
        };
        for (offset, value) in [0, 4, 12, 16].into_iter().zip(values) {
            bytes[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let output = remux(&bytes);
        compare_video(&bytes, &output);
        let input = NativeReader::software(Cursor::new(&bytes), usize::MAX).unwrap();
        let result = NativeReader::software(Cursor::new(&output), usize::MAX).unwrap();
        assert_eq!(input.rotation(), rotation);
        assert_eq!(result.rotation(), rotation);
        assert_eq!(input.pixel_aspect(), result.pixel_aspect());
        assert_eq!(input.dimensions(), result.dimensions());
    }
}
#[test]
fn unsupported_tracks_and_invalid_edits_fail_before_output() {
    let bytes = std::fs::read(fixture("audio/pcm-tags.mov")).unwrap();
    let mut input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert!(!mp4_matroska::eligible(&input));
    let mut out = Cursor::new(Vec::new());
    assert!(mp4_matroska::write(&mut input, &mut out, None, None).is_err());
    assert!(out.get_ref().is_empty());
    let original = std::fs::read(fixture("audio/aac-native-edit.m4a")).unwrap();
    for empty in [true, false] {
        let mut bytes = original.clone();
        let at = bytes.windows(4).position(|w| w == b"elst").unwrap();
        if empty {
            bytes[at + 12..at + 16].copy_from_slice(&0u32.to_be_bytes());
        } else {
            bytes[at + 16..at + 20].copy_from_slice(&(-1i32).to_be_bytes());
        }
        let mut input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let mut out = Cursor::new(Vec::new());
        assert!(mp4_matroska::write(&mut input, &mut out, None, None).is_err());
        assert!(out.get_ref().is_empty());
    }
}

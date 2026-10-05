//! Explicit external Matroska video/pixel and AAC reference comparisons.
use fvid::container::{
    adts,
    matroska_write::{Encoding, PacketWriter, TrackSpec},
    mp4, webm,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Exercise the packet layer's unedited media timeline. Mapping the movie edit
/// list belongs to the future high-level MP4 remuxer, not this writer test.
fn mux(name: &str) -> Vec<u8> {
    let mut source = mp4::Mp4Reader::open(
        std::fs::File::open(fixture(name)).unwrap(),
        Default::default(),
    )
    .unwrap();
    let track_index = source
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let video = source.tracks()[track_index].clone();
    let bytes = std::fs::read(fixture("audio/aac-stereo.aac")).unwrap();
    let mut audio = adts::StreamReader::open(bytes.as_slice()).unwrap();
    let config = audio.configuration();
    let encoding = match &video.codec {
        b"avc1" | b"avc3" => Encoding::Avc {
            configuration: &video.configuration,
            width: video.width.into(),
            height: video.height.into(),
        },
        b"hvc1" | b"hev1" => Encoding::Hevc {
            configuration: &video.configuration,
            width: video.width.into(),
            height: video.height.into(),
        },
        _ => panic!("fixture codec"),
    };
    let tracks = [
        TrackSpec {
            encoding,
            name: "picture",
            language: "und",
        },
        TrackSpec {
            encoding: Encoding::Aac {
                configuration: &config.asc,
                sample_rate: config.sample_rate,
                channels: config.channels,
            },
            name: "sound",
            language: "rus",
        },
    ];
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(&mut output, &tracks).unwrap();
    let mut expected = Vec::new();
    let mut buffer = Vec::new();
    let (mut vi, mut ai) = (0, 0u64);
    let mut audio_packet = audio.next_packet().unwrap();
    while vi < video.samples.len() || audio_packet.is_some() {
        let apts = ai * 1024 * 1_000_000_000 / u64::from(config.sample_rate);
        let sample = video.samples.get(vi);
        if audio_packet.is_some()
            && sample
                .as_ref()
                .is_none_or(|s| apts <= s.dts * 1_000_000_000 / u64::from(video.timescale))
        {
            let data = audio_packet.take().unwrap();
            let end = (ai + 1) * 1024 * 1_000_000_000 / u64::from(config.sample_rate);
            writer
                .write_packet(1, apts, end - apts, true, &data)
                .unwrap();
            expected.push((2, apts as i64, true, data, end - apts));
            ai += 1;
            audio_packet = audio.next_packet().unwrap();
        } else {
            let sample = sample.unwrap();
            source.read_packet(track_index, vi, &mut buffer).unwrap();
            let pts =
                u64::try_from(sample.pts).unwrap() * 1_000_000_000 / u64::from(video.timescale);
            let duration = (sample.pts as u64 + u64::from(sample.duration)) * 1_000_000_000
                / u64::from(video.timescale)
                - pts;
            writer
                .write_packet(0, pts, duration, sample.sync, &buffer)
                .unwrap();
            expected.push((1, pts as i64, sample.sync, buffer.clone(), duration));
            vi += 1;
        }
    }
    let event = writer.finish().unwrap();
    assert_eq!(event.packets, expected.len() as u64);
    assert_eq!(
        event.payload_bytes,
        expected.iter().map(|v| v.3.len() as u64).sum::<u64>()
    );
    assert!(!event.done);
    let mut reader =
        webm::WebmReader::open(Cursor::new(output.get_ref()), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 2);
    assert_eq!(reader.tracks[0].codec_private, video.configuration);
    assert_eq!(reader.tracks[0].name, "picture");
    assert_eq!(reader.tracks[1].language, "rus");
    assert_eq!(reader.tracks[1].codec_private, config.asc);
    assert_eq!(reader.packets.len(), expected.len());
    for (i, (track, pts, sync, data, duration)) in expected.iter().enumerate() {
        let packet = &reader.packets[i];
        assert_eq!(
            (packet.track, packet.pts_ns, packet.keyframe),
            (*track, *pts, *sync)
        );
        assert_eq!(packet.duration_ns, Some(*duration));
        assert_eq!(reader.read_packet(i).unwrap(), *data);
    }
    let times: Vec<_> = expected.iter().filter(|v| v.0 == 1).map(|v| v.1).collect();
    assert!(
        times.windows(2).any(|p| p[0] > p[1]),
        "fixture must exercise B-frame reordering"
    );
    output.into_inner()
}

fn video_with_decode_only_edges(name: &str, hidden: bool) -> Vec<u8> {
    use fvid::container::matroska_write::PacketOptions;
    let mut source = mp4::Mp4Reader::open(
        std::fs::File::open(fixture(name)).unwrap(),
        Default::default(),
    )
    .unwrap();
    let index = source
        .tracks()
        .iter()
        .position(|t| t.handler == *b"vide")
        .unwrap();
    let track = source.tracks()[index].clone();
    let mut times: Vec<_> = (0..track.samples.len())
        .map(|i| track.samples.get(i).unwrap().pts)
        .collect();
    times.sort();
    assert!(times.len() > 5);
    let begin = times[2];
    let end = times[times.len() - 2];
    let encoding = match &track.codec {
        b"avc1" | b"avc3" => Encoding::Avc {
            configuration: &track.configuration,
            width: track.width.into(),
            height: track.height.into(),
        },
        _ => Encoding::Hevc {
            configuration: &track.configuration,
            width: track.width.into(),
            height: track.height.into(),
        },
    };
    let mut output = Cursor::new(Vec::new());
    let mut writer = PacketWriter::new(
        &mut output,
        &[TrackSpec {
            encoding,
            name: "",
            language: "und",
        }],
    )
    .unwrap();
    let mut packet = Vec::new();
    for i in 0..track.samples.len() {
        let sample = track.samples.get(i).unwrap();
        source.read_packet(index, i, &mut packet).unwrap();
        let start = sample.pts as u64 * 1_000_000_000 / u64::from(track.timescale);
        let finish = (sample.pts as u64 + u64::from(sample.duration)) * 1_000_000_000
            / u64::from(track.timescale);
        writer
            .write_packet_with_options(
                0,
                start,
                finish - start,
                sample.sync,
                &packet,
                PacketOptions {
                    invisible: hidden && (sample.pts < begin || sample.pts >= end),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    writer.finish().unwrap();
    output.into_inner()
}

fn independent_decoder_preserves_all_video_frames_and_aac_pcm() {
    use std::process::Command;
    let ffmpeg = std::env::var("FVID_REFERENCE_FFMPEG").expect("reference executable");
    let directory = std::env::temp_dir().join(format!("fvid-mkv-video-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        // Read Matroska produced independently, not just our own writer.
        let external = directory.join("external.mkv");
        let result = Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-i"])
            .arg(fixture(name))
            .args(["-map", "0:v:0", "-c:v", "copy", "-an"])
            .arg(&external)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let own_pixels = |data: Vec<u8>| {
            let mut r =
                fvid::playback_native::NativeReader::software(Cursor::new(data), usize::MAX)
                    .unwrap();
            let mut pixels = Vec::new();
            while let Some(frame) = r.read_frame_raw().unwrap() {
                let [w, h] = r.dimensions();
                pixels.extend(
                    fvid::native_geometry::VideoGeometry::default()
                        .apply(&frame, w, h)
                        .unwrap()
                        .data,
                );
            }
            pixels
        };
        assert!(
            own_pixels(std::fs::read(fixture(name)).unwrap())
                == own_pixels(std::fs::read(&external).unwrap()),
            "external Matroska {name}"
        );
        let dest = directory.join("output.mkv");
        std::fs::write(&dest, mux(name)).unwrap();
        let decode = |path: &Path, audio: bool| {
            let mut command = Command::new(&ffmpeg);
            command.args(["-v", "error", "-i"]).arg(path);
            if audio {
                command.args(["-map", "0:a:0", "-f", "f32le", "-c:a", "pcm_f32le"]);
            } else {
                command.args([
                    "-map",
                    "0:v:0",
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    if name.contains("main10") {
                        "yuv420p10le"
                    } else {
                        "yuv420p"
                    },
                ]);
            }
            let out = command.arg("pipe:1").output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(!out.stdout.is_empty());
            out.stdout
        };
        assert!(
            decode(&fixture(name), false) == decode(&dest, false),
            "video {name}"
        );
        assert!(
            decode(&fixture("audio/aac-stereo.aac"), true) == decode(&dest, true),
            "AAC {name}"
        );
    }
}


fn owned_invisible_reconstruction_matches_independent_pixels() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let directory = std::env::temp_dir().join(format!("fvid-invisible-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let decode = |hidden| {
            let path = directory.join(if hidden { "hidden.mkv" } else { "full.mkv" });
            std::fs::write(&path, video_with_decode_only_edges(name, hidden)).unwrap();
            let output = std::process::Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(path)
                .args([
                    "-map",
                    "0:v:0",
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    if name.contains("main10") {
                        "yuv420p10le"
                    } else {
                        "yuv420p"
                    },
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        let full = decode(false);
        // FFmpeg 9.0.2 ignores the container's invisible bit. Use its full
        // decoded picture sequence as the independent pixel oracle instead.
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(video_with_decode_only_edges(name, true)),
            usize::MAX,
        )
        .unwrap();
        let mut hidden = Vec::new();
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            let [w, h] = reader.dimensions();
            hidden.extend(
                fvid::native_geometry::VideoGeometry::default()
                    .apply(&frame, w, h)
                    .unwrap()
                    .data,
            );
        }
        let source = mp4::Mp4Reader::open(
            std::fs::File::open(fixture(name)).unwrap(),
            Default::default(),
        )
        .unwrap();
        let video = source
            .tracks()
            .iter()
            .find(|t| t.handler == *b"vide")
            .unwrap();
        let frame_bytes = full.len() / video.samples.len();
        assert_eq!(full.len() % video.samples.len(), 0);
        assert!(
            hidden == full[2 * frame_bytes..full.len() - 2 * frame_bytes],
            "{name}: independent reference pixels: full={}, hidden={}, frame={frame_bytes}",
            full.len(),
            hidden.len()
        );
    }
}


fn main() {
    std::env::var("FVID_REFERENCE_FFMPEG").expect("set FVID_REFERENCE_FFMPEG");
    independent_decoder_preserves_all_video_frames_and_aac_pcm();
    owned_invisible_reconstruction_matches_independent_pixels();
    println!("Matroska video, AAC and invisible reconstruction reference suites passed");
}

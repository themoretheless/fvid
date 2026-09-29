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
                channels: config.channels.into(),
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

#[test]
fn avc_hevc_main_and_main10_interleave_with_aac_without_changing_packets() {
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        mux(name);
    }
}

#[test]
fn owned_player_preserves_pixels_reorders_b_frames_and_seeks() {
    use fvid::{native_geometry::VideoGeometry, playback_native::NativeReader};
    use std::time::Duration;
    for name in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let bytes = mux(name);
        let decode = |bytes: Vec<u8>| {
            let mut r = NativeReader::software(Cursor::new(bytes), usize::MAX).unwrap();
            let mut frames = Vec::new();
            while let Some(frame) = r.read_frame_raw().unwrap() {
                let [w, h] = r.dimensions();
                frames.push(VideoGeometry::default().apply(&frame, w, h).unwrap().data);
            }
            frames
        };
        let expected = decode(std::fs::read(fixture(name)).unwrap());
        assert!(expected == decode(bytes.clone()), "pixels for {name}");
        let mut r = NativeReader::software(Cursor::new(bytes), usize::MAX).unwrap();
        let mut times = Vec::new();
        let mut rgb = Vec::new();
        while r.read_frame().unwrap() {
            times.push(r.frame_interval().unwrap());
            rgb.push(r.rgb().to_vec());
        }
        assert_eq!(times.len(), expected.len());
        assert!(times.windows(2).all(|t| t[0].1 == t[1].0));
        assert!(!r.read_frame().unwrap());
        for i in [0, times.len() / 2, times.len() - 1, 0] {
            let target = Duration::from_nanos(times[i].0 as u64);
            let frame = r.seek_raw(target).unwrap().unwrap();
            let [w, h] = r.dimensions();
            assert_eq!(
                VideoGeometry::default().apply(&frame, w, h).unwrap().data,
                expected[i],
                "seek {name} frame {i}"
            );
            assert_eq!(r.frame_interval().unwrap(), times[i]);
            r.seek(target).unwrap();
            assert_eq!(r.rgb(), rgb[i]);
        }
        r.rewind().unwrap();
        assert!(r.read_frame().unwrap());
        assert_eq!(r.rgb(), rgb[0]);
    }
}

#[test]
fn damaged_nal_packet_requires_rewind_and_small_budgets_fail() {
    use fvid::playback_native::NativeReader;
    let mut data = mux("video.mp4");
    let mut demux = webm::WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
    demux.scan_all().unwrap();
    let first = demux.packets.iter().find(|p| p.track == 1).unwrap().offset as usize;
    drop(demux);
    data[first..first + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut reader = NativeReader::software(Cursor::new(data), usize::MAX).unwrap();
    assert!(reader.read_frame_raw().is_err());
    let error = reader.read_frame_raw().err().unwrap().to_string();
    assert!(error.contains("rewind"));
    reader.rewind().unwrap();
    assert!(reader.read_frame_raw().is_err());
    match NativeReader::software(Cursor::new(mux("video.mp4")), 1) {
        Err(_) => {}
        Ok(mut reader) => assert!(reader.read_frame_raw().is_err()),
    }
}

#[test]
fn invalid_packet_poisoning_and_empty_tracks_prevent_finalization() {
    let asc = [0x12, 0x10];
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: &asc,
            sample_rate: 44100,
            channels: 2,
        },
        name: "",
        language: "",
    };
    for (track, pts, duration, data) in [
        (1, 0, 1, vec![1]),
        (0, u64::MAX, 1, vec![1]),
        (0, 0, 0, vec![1]),
        (0, 0, 1, vec![]),
    ] {
        let mut output = Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(&mut output, std::slice::from_ref(&spec)).unwrap();
        writer.write_packet(0, 0, 1, true, &[1]).unwrap();
        assert!(
            writer
                .write_packet(track, pts, duration, true, &data)
                .is_err()
        );
        assert!(writer.write_packet(0, 1, 1, true, &[1]).is_err());
        assert!(writer.finish().is_err());
    }
    let mut output = Cursor::new(Vec::new());
    assert!(PacketWriter::new(&mut output, &[]).is_err());
    assert!(output.get_ref().is_empty());
    assert!(
        PacketWriter::new(&mut output, &[spec])
            .unwrap()
            .finish()
            .is_err()
    );
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
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

#[test]
fn failed_output_cannot_be_finalized_or_reused() {
    use std::{
        cell::Cell,
        io::{self, Seek, SeekFrom, Write},
        rc::Rc,
    };
    struct Sink {
        data: Cursor<Vec<u8>>,
        fail: Rc<Cell<bool>>,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail.get() {
                Err(io::Error::other("injected write failure"))
            } else {
                self.data.write(bytes)
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Sink {
        fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
            self.data.seek(at)
        }
    }
    let fail = Rc::new(Cell::new(false));
    let mut sink = Sink {
        data: Cursor::new(Vec::new()),
        fail: fail.clone(),
    };
    let spec = TrackSpec {
        encoding: Encoding::Aac {
            configuration: &[0x12, 0x10],
            sample_rate: 44100,
            channels: 2,
        },
        name: "",
        language: "",
    };
    let mut writer = PacketWriter::new(&mut sink, &[spec]).unwrap();
    writer.write_packet(0, 0, 1, true, &[1]).unwrap();
    fail.set(true);
    assert!(writer.write_packet(0, 1, 1, true, &[1]).is_err());
    fail.set(false);
    assert!(writer.write_packet(0, 1, 1, true, &[1]).is_err());
    assert!(writer.finish().is_err());
}

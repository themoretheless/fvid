#![cfg(all(target_os = "macos", feature = "videotoolbox"))]

#[test]
#[ignore = "requires an available physical VideoToolbox decoder"]
fn videotoolbox_session_is_hardware_and_decodes_actual_frames() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for bytes in [
        include_bytes!("fixtures/video.mp4").as_slice(),
        include_bytes!("fixtures/hevc/main-ipb.mp4").as_slice(),
    ] {
        let mut reader = NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
        // Session creation now requires hardware and queries the hardware-use
        // property. A software fallback fails this assertion, rather than passing.
        assert!(reader.hardware_accelerated(), "hardware decoder required");
        let mut frames = 0;
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            let RawFrame::Planar8(frame) = frame else {
                panic!("expected hardware planar output");
            };
            assert_eq!(frame.y.len(), frame.width * frame.height);
            assert_eq!(frame.cb.len(), frame.chroma_width * frame.chroma_height);
            assert_eq!(frame.cr.len(), frame.cb.len());
            frames += 1;
        }
        assert!(frames > 1);
    }
}

#[test]
#[ignore = "requires a physical VideoToolbox Main10 decoder"]
fn hardware_main10_keeps_source_precision() {
    use fvid::playback_native::{NativeReader, RawFrame};
    let bytes = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let mut hardware = NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
    assert!(
        hardware.hardware_accelerated(),
        "Main10 hardware decoder required"
    );
    let mut software =
        NativeReader::software(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
    let mut frames = 0;
    let mut low_bits = false;
    loop {
        let hw = hardware.read_frame_raw().unwrap();
        let sw = software.read_frame_raw().unwrap();
        assert_eq!(hw.is_some(), sw.is_some());
        let Some(RawFrame::Avc { picture: hw, .. }) = hw else {
            if sw.is_none() {
                break;
            }
            panic!("Main10 hardware output was narrowed to 8 bits");
        };
        let Some(RawFrame::Avc { picture: sw, .. }) = sw else {
            panic!("expected Main10 software reference");
        };
        assert_eq!(hw.bit_depth, 10);
        assert_eq!(hw.coded_width, sw.coded_width);
        assert_eq!(hw.coded_height, sw.coded_height);
        assert_eq!(hw.crop, sw.crop);
        low_bits |= hw.y.iter().any(|v| v & 3 != 0);
        assert_eq!(hw.y, sw.y, "luma frame {frames}");
        assert_eq!(hw.cb, sw.cb, "Cb frame {frames}");
        assert_eq!(hw.cr, sw.cr, "Cr frame {frames}");
        frames += 1;
    }
    assert_eq!(frames, 17);
    assert!(
        low_bits,
        "fixture must prove low bits survive hardware decode"
    );
}

#[test]
#[ignore = "requires physical VideoToolbox decoding"]
fn native_reader_shared_surfaces_preserve_order_timing_and_seek() {
    use fvid::playback_native::{NativeReader, RawFrame};
    let reference_bytes = |raw: RawFrame| match raw {
        RawFrame::Planar8(p) => [p.y.as_slice(), p.cb.as_slice(), p.cr.as_slice()].concat(),
        RawFrame::Avc { picture, .. } => picture
            .y
            .iter()
            .chain(&picture.cb)
            .chain(&picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect(),
        RawFrame::Surface { surface, colour } => {
            fvid::playback_native::surface_to_packed(&surface, colour)
                .unwrap()
                .frame
                .data
        }
        _ => panic!("unexpected decoded storage"),
    };
    for bytes in [
        include_bytes!("fixtures/display/par-2x1.mp4").as_slice(),
        include_bytes!("fixtures/hevc/main-ipb.mp4").as_slice(),
        include_bytes!("fixtures/hevc/main10-ipb.mp4").as_slice(),
    ] {
        let make = || NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
        let mut shared = make();
        let mut baseline = make();
        assert!(baseline.hardware_accelerated());
        assert!(shared.enable_shared_surfaces().unwrap());
        let mut held = Vec::new();
        loop {
            let frame = shared.read_frame_raw().unwrap();
            let reference = baseline.read_frame_raw().unwrap();
            assert_eq!(frame.is_some(), reference.is_some());
            let Some(frame) = frame else {
                break;
            };
            assert_eq!(shared.current_pts(), baseline.current_pts());
            assert_eq!(shared.frame_interval(), baseline.frame_interval());
            assert_eq!(shared.dimensions(), baseline.dimensions());
            let RawFrame::Surface { surface, colour } = frame else {
                panic!("expected shared surface");
            };
            assert!(surface.storage_bytes() >= surface.width() * surface.height());
            let rgb = RawFrame::Surface {
                surface: surface.clone(),
                colour,
            }
            .into_rgb(64 * 1024 * 1024)
            .unwrap();
            let packed = fvid::playback_native::surface_to_packed(&surface, colour).unwrap();
            assert!(
                packed.frame.data == reference_bytes(reference.unwrap()),
                "source planes differ"
            );
            held.push((surface, colour, rgb));
        }
        assert!(held.len() >= 10);
        assert!(shared.enable_shared_surfaces().is_err());
        let target = std::time::Duration::from_millis(200);
        let a = shared.seek_raw(target).unwrap().unwrap();
        let b = baseline.seek_raw(target).unwrap().unwrap();
        assert!(matches!(&a, RawFrame::Surface { .. }));
        assert_eq!(shared.current_pts(), baseline.current_pts());
        assert!(
            reference_bytes(a) == reference_bytes(b),
            "seek planes differ"
        );
        drop(shared);
        for (surface, colour, rgb) in held {
            assert_eq!(
                RawFrame::Surface { surface, colour }
                    .into_rgb(64 * 1024 * 1024)
                    .unwrap(),
                rgb
            );
        }
    }
}

#[test]
#[cfg(feature = "player")]
#[ignore = "requires physical VideoToolbox decoding"]
fn playback_queue_keeps_hardware_surfaces_from_first_frame_and_after_seek() {
    use fvid::playback_native::NativeReader;
    use fvid::playback_thread::{Event, Pixels, Playback};
    use std::time::{Duration, Instant};
    for bytes in [
        include_bytes!("fixtures/display/par-2x1.mp4").as_slice(),
        include_bytes!("fixtures/hevc/main10-ipb.mp4").as_slice(),
    ] {
        for (rotation, matrix_values) in [
            (0, [0x10000i32, 0, 0, 0x10000]),
            (90, [0, 0x10000, -0x10000, 0]),
            (180, [-0x10000, 0, 0, -0x10000]),
            (270, [0, -0x10000, 0x10000, 0]),
        ] {
            let mut turned = bytes.to_vec();
            let matrix = turned.windows(4).position(|v| v == b"tkhd").unwrap() + 44;
            for (offset, value) in [0, 4, 12, 16].into_iter().zip(matrix_values) {
                turned[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
            }
            let mut reader =
                NativeReader::new(std::io::Cursor::new(turned), 64 * 1024 * 1024).unwrap();
            assert_eq!(reader.rotation(), rotation);
            assert!(reader.enable_shared_surfaces().unwrap());
            let first = reader.read_frame_raw().unwrap().unwrap();
            let first_pts = reader.current_pts();
            let grade = fvid::color::Grade::new(
                Default::default(),
                &Default::default(),
                fvid::color::Settings {
                    size: 17,
                    ..Default::default()
                },
                Some(fvid::color::Lut::Three(fvid::color::Lut3d::from_fn(
                    5,
                    |rgb| [rgb[2], rgb[0], rgb[1]],
                ))),
            );
            assert!(grade.is_gpu_grade());
            assert!(!grade.is_shader_look());
            let mut playback = Playback::start_from_frame(reader, first, Some(grade));
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut count = 0;
            loop {
                assert!(Instant::now() < deadline, "queue did not finish");
                match playback.poll() {
                    Some(Event::Frame(frame)) => {
                        if count == 0 {
                            assert_eq!(frame.pts, first_pts);
                        }
                        let Pixels::Surface(surface) = frame.pixels else {
                            panic!("frame was downloaded on converter thread");
                        };
                        let (w, h) = (surface.surface.width(), surface.surface.height());
                        assert_eq!(surface.rotation, rotation);
                        assert_eq!(
                            if rotation == 90 || rotation == 270 {
                                [h, w]
                            } else {
                                [w, h]
                            },
                            frame.dimensions
                        );
                        assert!(surface.grade.as_ref().unwrap().is_gpu_grade());
                        count += 1;
                    }
                    Some(Event::Ended(_)) => break,
                    Some(Event::Error(error)) => panic!("{error}"),
                    None => std::thread::sleep(Duration::from_millis(1)),
                }
            }
            assert!(count >= 10);
            playback.seek(Duration::from_millis(200));
            let generation = playback.generation();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                assert!(Instant::now() < deadline, "seek did not yield a surface");
                match playback.poll() {
                    Some(Event::Frame(frame)) if frame.generation == generation => {
                        assert!(matches!(frame.pixels, Pixels::Surface(_)));
                        break;
                    }
                    Some(Event::Error(error)) => panic!("{error}"),
                    _ => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }
    }
}

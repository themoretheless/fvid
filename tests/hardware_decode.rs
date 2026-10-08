#![cfg(all(target_os = "macos", feature = "videotoolbox"))]

#[test]
fn encoded_timestamps_check_signed_container_bounds() {
    use fvid_vt::EncodedTime;
    assert_eq!(
        EncodedTime {
            value: -1001,
            timescale: 30000
        }
        .nanoseconds()
        .unwrap(),
        -33_366_666
    );
    assert_eq!(
        EncodedTime {
            value: i64::MAX,
            timescale: 1_000_000_000
        }
        .nanoseconds()
        .unwrap(),
        i64::MAX
    );
    assert_eq!(
        EncodedTime {
            value: i64::MIN,
            timescale: 1_000_000_000
        }
        .nanoseconds()
        .unwrap(),
        i64::MIN
    );
    assert!(
        EncodedTime {
            value: i64::MAX,
            timescale: 1
        }
        .nanoseconds()
        .is_err()
    );
    assert!(
        EncodedTime {
            value: 0,
            timescale: 0
        }
        .nanoseconds()
        .is_err()
    );
}

#[test]
#[ignore = "requires physical Main10 VideoToolbox encoder"]
fn hardware_encoder_preserves_main10_profile() {
    use fvid::playback_native::{NativeReader, RawFrame};
    let bytes = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let mut reader =
        NativeReader::new(std::io::Cursor::new(bytes.as_slice()), 64 * 1024 * 1024).unwrap();
    assert!(reader.enable_shared_surfaces().unwrap());
    let RawFrame::Surface { surface, .. } = reader.read_frame_raw().unwrap().unwrap() else {
        panic!("P010 surface required")
    };
    assert_eq!(surface.depth(), 10);
    let mut wrong = fvid_vt::Encoder::new(
        fvid_vt::EncoderCodec::Hevc,
        surface.width(),
        surface.height(),
    )
    .unwrap();
    assert!(wrong.encode(&surface, 0, 1, 25).is_err());
    let mut encoder = fvid_vt::Encoder::new_with_depth(
        fvid_vt::EncoderCodec::Hevc,
        surface.width(),
        surface.height(),
        10,
    )
    .unwrap();
    let mut decoder = None;
    for index in 0..3 {
        let encoded = encoder.encode(&surface, index, 1, 25).unwrap();
        if decoder.is_none() {
            let config =
                fvid::codec::config::HevcConfig::parse(&encoded.decoder_configuration).unwrap();
            assert_eq!(config.profile, 2);
            assert_eq!(config.bit_depth_luma, 10);
            let of_type = |kind| {
                encoded
                    .parameter_sets
                    .iter()
                    .filter(|p| (p[0] >> 1) & 63 == kind)
                    .map(Vec::as_slice)
                    .collect::<Vec<_>>()
            };
            decoder = Some(
                fvid_vt::Session::new_hevc_surface(
                    &of_type(32),
                    &of_type(33),
                    &of_type(34),
                    encoded.nal_length_size,
                    10,
                    false,
                )
                .unwrap(),
            );
        }
        let decoded = decoder
            .as_mut()
            .unwrap()
            .decode_surface(&encoded.data)
            .unwrap()
            .unwrap();
        assert_eq!(decoded.depth(), 10);
        assert_eq!(
            [decoded.width(), decoded.height()],
            [surface.width(), surface.height()]
        );
    }
    let hdr = fvid::color::hdr::HdrMetadata {
        mastering: Some(
            fvid::color::hdr::MasteringDisplay::from_corners(
                (0.68, 0.32),
                (0.265, 0.69),
                (0.15, 0.06),
                (0.3127, 0.329),
                1000.0,
                0.005,
            )
            .unwrap(),
        ),
        light: fvid::color::tonemap::ContentLight {
            max_cll: 1000.0,
            max_fall: 400.0,
        },
    };
    let colour = fvid::color::hdr::ColourDescription {
        primaries: 9,
        transfer: 16,
        matrix: 9,
        full_range: false,
    };
    let options = fvid::hardware_export::TrackOptions {
        video: Some(fvid::hardware_export::VideoMetadata {
            colour: Some(colour),
            hdr,
            ..Default::default()
        }),
        ..Default::default()
    };
    let frames = (0..3).map(|index| {
        Ok(fvid::hardware_export::HardwareFrame {
            surface: surface.clone(),
            pts: fvid_vt::EncodedTime {
                value: index,
                timescale: 25,
            },
            duration: fvid_vt::EncodedTime {
                value: 1,
                timescale: 25,
            },
            force_keyframe: false,
        })
    });
    let mut output = std::io::Cursor::new(Vec::new());
    let stats = fvid::hardware_export::write_video_with_options(
        &mut output,
        frames,
        fvid_vt::EncoderCodec::Hevc,
        1_000_000,
        options,
    )
    .unwrap();
    assert_eq!(stats.frames, 3);
    output.set_position(0);
    let mut exported = NativeReader::new(output, 64 * 1024 * 1024).unwrap();
    assert_eq!(exported.colour(), colour);
    assert_eq!(exported.hdr(), hdr);
    for _ in 0..3 {
        match exported.read_frame_raw().unwrap().unwrap() {
            RawFrame::Planar(frame) => assert_eq!(frame.depth, 10),
            RawFrame::Avc { picture, .. } => assert_eq!(picture.bit_depth, 10),
            _ => panic!("Main10 export must retain source precision"),
        }
        let NativeReader::Webm(ref reader) = exported else {
            panic!("Matroska reader required")
        };
        let coded = reader.bitstream_hdr();
        assert_eq!(coded.light,hdr.light);
        assert_eq!(
            fvid::color::hdr::mdcv_payload(&coded.mastering.unwrap()),
            fvid::color::hdr::mdcv_payload(&hdr.mastering.unwrap()),
            "HEVC SEI must retain explicitly supplied output HDR independently of Matroska metadata"
        );
    }
    assert!(exported.read_frame_raw().unwrap().is_none());
}

#[test]
#[ignore = "requires physical VideoToolbox decoder and encoder"]
fn hardware_encoder_accepts_retained_nv12_without_pixel_download() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for codec in [fvid_vt::EncoderCodec::H264, fvid_vt::EncoderCodec::Hevc] {
        let bytes = include_bytes!("fixtures/display/par-2x1.mp4");
        let mut reader =
            NativeReader::new(std::io::Cursor::new(bytes.as_slice()), 64 * 1024 * 1024).unwrap();
        assert!(reader.enable_shared_surfaces().unwrap());
        let RawFrame::Surface { surface, .. } = reader.read_frame_raw().unwrap().unwrap() else {
            panic!("native surface required")
        };
        drop(reader);
        assert!(
            fvid_vt::Encoder::new_with_bitrate(codec, surface.width(), surface.height(), 8, 0)
                .is_err()
        );
        let mut encoder = fvid_vt::Encoder::new_with_bitrate(
            codec,
            surface.width(),
            surface.height(),
            8,
            1_000_000,
        )
        .unwrap();
        let mut decoder = None;
        let mut encoded = Vec::new();
        for index in 0..3 {
            let frame = encoder
                .encode_with_keyframe(
                    &surface,
                    [0, 1, 4][index as usize],
                    [1, 3, 2][index as usize],
                    25,
                    index == 2,
                )
                .unwrap();
            assert_eq!(
                frame.pts.value as i128 * 25,
                [0, 1, 4][index as usize] as i128 * frame.pts.timescale as i128
            );
            assert_eq!(
                frame.duration.value as i128 * 25,
                [1, 3, 2][index as usize] as i128 * frame.duration.timescale as i128
            );
            if index == 0 || index == 2 {
                assert!(frame.key_frame);
            }
            if index == 2 {
                decoder = None;
            }
            if decoder.is_none() {
                let of_type = |kind| {
                    frame
                        .parameter_sets
                        .iter()
                        .filter(|p| match codec {
                            fvid_vt::EncoderCodec::H264 => p[0] & 31 == kind,
                            fvid_vt::EncoderCodec::Hevc => (p[0] >> 1) & 63 == kind,
                        })
                        .map(Vec::as_slice)
                        .collect::<Vec<_>>()
                };
                decoder = Some(
                    match codec {
                        fvid_vt::EncoderCodec::H264 => fvid_vt::Session::new_avc_surface(
                            &of_type(7),
                            &of_type(8),
                            frame.nal_length_size,
                            false,
                        ),
                        fvid_vt::EncoderCodec::Hevc => fvid_vt::Session::new_hevc_surface(
                            &of_type(32),
                            &of_type(33),
                            &of_type(34),
                            frame.nal_length_size,
                            8,
                            false,
                        ),
                    }
                    .unwrap(),
                );
            }
            let decoded = decoder
                .as_mut()
                .unwrap()
                .decode_surface(&frame.data)
                .unwrap()
                .expect("encoded access unit must decode");
            assert_eq!(
                [decoded.width(), decoded.height()],
                [surface.width(), surface.height()]
            );
            assert_eq!(decoded.depth(), 8);
            let mut offset = 0;
            let mut count = 0;
            while offset < frame.data.len() {
                let length =
                    u32::from_be_bytes(frame.data[offset..offset + 4].try_into().unwrap()) as usize;
                assert!(length > 0);
                offset += 4 + length;
                assert!(offset <= frame.data.len());
                count += 1;
            }
            assert!(count > 0);
            encoded.push(frame);
        }
        use fvid::container::matroska_write::{Encoding, PacketWriter, TrackSpec};
        let configuration = &encoded[0].decoder_configuration;
        let encoding = match codec {
            fvid_vt::EncoderCodec::H264 => Encoding::Avc {
                configuration,
                width: surface.width() as u32,
                height: surface.height() as u32,
            },
            fvid_vt::EncoderCodec::Hevc => Encoding::Hevc {
                configuration,
                width: surface.width() as u32,
                height: surface.height() as u32,
            },
        };
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = PacketWriter::new(
            &mut output,
            &[TrackSpec {
                encoding,
                name: "hardware",
                language: "und",
            }],
        )
        .unwrap();
        for frame in &encoded {
            assert_eq!(frame.decoder_configuration, *configuration);
            writer
                .write_packet(
                    0,
                    u64::try_from(frame.pts.nanoseconds().unwrap()).unwrap(),
                    u64::try_from(frame.duration.nanoseconds().unwrap()).unwrap(),
                    frame.key_frame,
                    &frame.data,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        output.set_position(0);
        let mut muxed = NativeReader::new(output, 64 * 1024 * 1024).unwrap();
        let mut count = 0;
        while muxed.read_frame_raw().unwrap().is_some() {
            let (pts, scale) = muxed.current_pts().expect("muxed frame timestamp");
            assert_eq!(pts as i128 * 25, [0, 1, 4][count] as i128 * scale as i128);
            count += 1;
        }
        assert_eq!(count, 3, "muxed hardware stream must decode all frames");
        let frames = (0..3).map(|index| {
            Ok(fvid::hardware_export::HardwareFrame {
                surface: surface.clone(),
                pts: fvid_vt::EncodedTime {
                    value: [0, 1, 4][index],
                    timescale: 25,
                },
                duration: fvid_vt::EncodedTime {
                    value: [1, 3, 2][index],
                    timescale: 25,
                },
                force_keyframe: index == 2,
            })
        });
        let mut exported = std::io::Cursor::new(Vec::new());
        let colour = fvid::color::hdr::ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let options = fvid::hardware_export::TrackOptions {
            rotation: 90,
            video: Some(fvid::hardware_export::VideoMetadata {
                pixel_aspect: (2, 1),
                colour: Some(colour),
                ..Default::default()
            }),
            ..Default::default()
        };
        let stats = fvid::hardware_export::write_video_with_options(
            &mut exported,
            frames,
            codec,
            1_000_000,
            options,
        )
        .unwrap();
        assert_eq!(stats.frames, 3);
        assert!(stats.compressed_bytes > 0);
        exported.set_position(0);
        let mut reader = NativeReader::new(exported, 64 * 1024 * 1024).unwrap();
        assert_eq!(reader.rotation(), 90);
        // Display-space pixel aspect reverses for a quarter-turn rotation.
        assert_eq!(reader.pixel_aspect(), (1, 2));
        assert_eq!(reader.colour(), colour);
        for pts in [0, 1, 4] {
            assert!(reader.read_frame_raw().unwrap().is_some());
            let (actual, scale) = reader.current_pts().unwrap();
            assert_eq!(actual as i128 * 25, pts as i128 * scale as i128);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        let frame = &encoded[0];
        let of_type = |kind| {
            frame
                .parameter_sets
                .iter()
                .filter(|p| match codec {
                    fvid_vt::EncoderCodec::H264 => p[0] & 31 == kind,
                    fvid_vt::EncoderCodec::Hevc => (p[0] >> 1) & 63 == kind,
                })
                .map(Vec::as_slice)
                .collect::<Vec<_>>()
        };
        let mut full_decoder = match codec {
            fvid_vt::EncoderCodec::H264 => fvid_vt::Session::new_avc_surface(
                &of_type(7),
                &of_type(8),
                frame.nal_length_size,
                true,
            ),
            fvid_vt::EncoderCodec::Hevc => fvid_vt::Session::new_hevc_surface(
                &of_type(32),
                &of_type(33),
                &of_type(34),
                frame.nal_length_size,
                8,
                true,
            ),
        }
        .unwrap();
        let full = full_decoder.decode_surface(&frame.data).unwrap().unwrap();
        assert!(full.full_range());
        assert!(!surface.full_range());
        let frames = [surface.clone(), full]
            .into_iter()
            .enumerate()
            .map(|(index, surface)| {
                Ok(fvid::hardware_export::HardwareFrame {
                    surface,
                    pts: fvid_vt::EncodedTime {
                        value: index as i64,
                        timescale: 25,
                    },
                    duration: fvid_vt::EncodedTime {
                        value: 1,
                        timescale: 25,
                    },
                    force_keyframe: false,
                })
            });
        let error = fvid::hardware_export::write_video(
            &mut std::io::Cursor::new(Vec::new()),
            frames,
            codec,
            1_000_000,
        )
        .unwrap_err();
        assert!(error.to_string().contains("range changed"), "{error}");
    }
}

#[test]
#[ignore = "requires an available physical VideoToolbox decoder"]
fn sequential_1d_grade_keeps_main10_surface_in_playback_queue() {
    use fvid::color::{Grade, Lut, Lut1d, Primaries, Settings};
    use fvid::playback_native::NativeReader;
    use fvid::playback_thread::{Event, Pixels, Playback};
    use std::time::{Duration, Instant};
    let bytes = include_bytes!("fixtures/hevc/main10-ipb.mp4");
    let mut reader =
        NativeReader::new(std::io::Cursor::new(bytes.as_slice()), 64 * 1024 * 1024).unwrap();
    assert!(reader.enable_shared_surfaces().unwrap());
    let first = reader.read_frame_raw().unwrap().unwrap();
    let grade = Grade::new(
        Default::default(),
        &Default::default(),
        Settings {
            gamut: Some(Primaries::BT2020),
            size: 17,
            ..Default::default()
        },
        Some(Lut::One(Lut1d {
            data: [
                vec![0.0, 0.25, 1.0],
                vec![0.0, 0.75, 1.0],
                vec![1.0, 0.5, 0.0],
            ],
            domain_min: [-0.1; 3],
            domain_max: [1.1; 3],
        })),
    );
    assert!(grade.is_gpu_grade() && !grade.is_shader_look());
    let playback = Playback::start_from_frame(reader, first, Some(grade));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut count = 0;
    loop {
        assert!(Instant::now() < deadline);
        match playback.poll() {
            Some(Event::Frame(frame)) => {
                let Pixels::Surface(surface) = frame.pixels else {
                    panic!("1D grade downloaded the surface")
                };
                assert_eq!(surface.surface.depth(), 10);
                assert!(surface.grade.as_ref().unwrap().is_gpu_grade());
                count += 1;
            }
            Some(Event::Ended(_)) => break,
            Some(Event::Error(error)) => panic!("{error}"),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    assert_eq!(count, 17);
}

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

#[test]
#[ignore = "requires an available physical VideoToolbox VP9 decoder"]
fn webm_hardware_decode_and_shared_surfaces() {
    use fvid::playback_native::{NativeReader, RawFrame};
    let bytes = include_bytes!("fixtures/short/vp9-motion.webm");
    let mut reader = NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
    if reader.hardware_accelerated() {
        assert!(reader.enable_shared_surfaces().unwrap());
        if let Some(RawFrame::Surface { surface, .. }) = reader.read_frame_raw().unwrap() {
            assert!(surface.width() > 0);
            assert!(surface.height() > 0);
        }
    }
}

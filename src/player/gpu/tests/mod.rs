use super::*;

    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn hardware_export_copies_mp4_audio_with_gapless_timing() {
        use crate::container::{matroska_write, mp4::Mp4Reader, webm::WebmReader};
        use crate::hardware_export::*;
        let (device, queue) = headless().unwrap();
        let surface = fvid_vt::Surface::metal_black(&device, &queue, 32, 32, 8, false).unwrap();
        let destination = std::env::temp_dir().join(format!(
            "fvid-audio-export-{}-{}.mkv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(destination.clone());
        for bytes in [
            include_bytes!("../../../../tests/fixtures/audio/two-audio.mp4").as_slice(),
            include_bytes!("../../../../tests/fixtures/audio/aac-native-edit.m4a").as_slice(),
        ] {
            let input = Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
            let audio_indices: Vec<_> = input
                .tracks()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.handler == *b"soun")
                .map(|(i, _)| i)
                .collect();
            assert!(!audio_indices.is_empty());
            let frames = (0..3).map(|i| {
                Ok(HardwareFrame {
                    surface: surface.clone(),
                    pts: EncodedTime {
                        value: i,
                        timescale: 25,
                    },
                    duration: EncodedTime {
                        value: 1,
                        timescale: 25,
                    },
                    force_keyframe: false,
                })
            });
            write_video_file_with_mp4_audio(
                &destination,
                frames,
                EncoderCodec::H264,
                1_000_000,
                TrackOptions::default(),
                input,
            )
            .unwrap();
            let mut output = WebmReader::open(
                std::fs::File::open(&destination).unwrap(),
                Default::default(),
            )
            .unwrap();
            output.scan_all().unwrap();
            assert_eq!(output.tracks.len(), audio_indices.len() + 1);
            for (additional, source_index) in audio_indices.into_iter().enumerate() {
                let mut source =
                    Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
                let mut expected = std::io::Cursor::new(Vec::new());
                matroska_write::write_mp4_aac(&mut source, source_index, &mut expected, None, None)
                    .unwrap();
                expected.set_position(0);
                let mut reference = WebmReader::open(expected, Default::default()).unwrap();
                reference.scan_all().unwrap();
                let track = additional + 1;
                assert_eq!(
                    output.tracks[track].codec_private,
                    reference.tracks[0].codec_private
                );
                assert_eq!(
                    output.tracks[track].codec_delay_ns,
                    reference.tracks[0].codec_delay_ns
                );
                let actual: Vec<_> = output
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == output.tracks[track].number)
                    .map(|(i, p)| (i, p.pts_ns, p.duration_ns, p.discard_padding_ns))
                    .collect();
                assert_eq!(actual.len(), reference.packets.len());
                for ((i, pts, duration, padding), j) in
                    actual.into_iter().zip(0..reference.packets.len())
                {
                    assert_eq!(
                        (pts, duration, padding),
                        (
                            reference.packets[j].pts_ns,
                            reference.packets[j].duration_ns,
                            reference.packets[j].discard_padding_ns
                        )
                    );
                    assert_eq!(
                        output.read_packet(i).unwrap(),
                        reference.read_packet(j).unwrap()
                    );
                }
                for interval in [
                    None,
                    Some((
                        std::time::Duration::from_millis(10),
                        std::time::Duration::from_millis(20),
                    )),
                ] {
                    let source =
                        Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
                    let exported = WebmReader::open(
                        std::fs::File::open(&destination).unwrap(),
                        Default::default(),
                    )
                    .unwrap();
                    let mut source_pcm = Vec::new();
                    let mut exported_pcm = Vec::new();
                    let mut control = crate::native_media::DecodeProgress::new(None, None).unwrap();
                    let source_stats = crate::native_media::decode_mp4_audio_reader_controlled(
                        source,
                        &mut source_pcm,
                        interval,
                        Some(source_index),
                        &mut control,
                    )
                    .unwrap();
                    let mut control = crate::native_media::DecodeProgress::new(None, None).unwrap();
                    let exported_stats =
                        crate::native_media::decode_matroska_audio_reader_controlled(
                            exported,
                            &mut exported_pcm,
                            interval,
                            Some(track),
                            &mut control,
                        )
                        .unwrap();
                    assert!(source_stats.sample_frames > 0);
                    assert_eq!(source_stats.sample_frames, exported_stats.sample_frames);
                    assert_eq!(
                        source_pcm, exported_pcm,
                        "PCM differs for source track {source_index}, interval {interval:?}"
                    );
                }
            }
            std::fs::remove_file(&destination).unwrap();
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn hardware_video_mux_interleaves_subtitles_and_preserves_tail() {
        use crate::container::matroska_write::Encoding;
        use crate::hardware_export::*;
        let (device, queue) = headless().unwrap();
        let surface = fvid_vt::Surface::metal_black(&device, &queue, 32, 32, 8, false).unwrap();
        let frames = (0..3).map(|i| {
            Ok(HardwareFrame {
                surface: surface.clone(),
                pts: EncodedTime {
                    value: i,
                    timescale: 25,
                },
                duration: EncodedTime {
                    value: 1,
                    timescale: 25,
                },
                force_keyframe: false,
            })
        });
        let payload = b"0,0,Default,,0,0,0,,caption";
        let mut audio_source = crate::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(
                include_bytes!("../../../../tests/fixtures/audio/two-audio.mp4").as_slice(),
            ),
            crate::container::mp4::Limits::default(),
        )
        .unwrap();
        let audio_index = audio_source
            .tracks()
            .iter()
            .position(|t| t.codec == *b"mp4a")
            .unwrap();
        let audio = &audio_source.tracks()[audio_index];
        let asc = crate::codec::config::aac_specific_config(&audio.configuration)
            .unwrap()
            .to_vec();
        let sample_rate = audio.sample_rate;
        let channels = audio.channels;
        let mut audio_payload = Vec::new();
        audio_source
            .read_packet(audio_index, 0, &mut audio_payload)
            .unwrap();
        let times = [20_000_000, 60_000_000, 300_000_000];
        let packets = times
            .into_iter()
            .map(|pts_ns| {
                Ok(AdditionalPacket {
                    track: 0,
                    pts_ns,
                    duration_ns: 100_000_000,
                    key_frame: true,
                    data: payload.to_vec(),
                    options: PacketOptions::default(),
                })
            })
            .chain(std::iter::once(Ok(AdditionalPacket {
                track: 1,
                pts_ns: 400_000_000,
                duration_ns: 21_333_333,
                key_frame: true,
                data: audio_payload.clone(),
                options: PacketOptions {
                    discard_padding_ns: 1_000_000,
                    invisible: false,
                },
            })));
        let track = AdditionalTrack {
            encoding: Encoding::Ass {
                configuration: b"[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Default,Arial,20\n[Events]\n",
            },
            name: "captions",
            language: "eng",
        };
        let mut output = std::io::Cursor::new(Vec::new());
        let stats = write_video_with_tracks(
            &mut output,
            frames,
            EncoderCodec::H264,
            1_000_000,
            TrackOptions::default(),
            vec![
                track,
                AdditionalTrack {
                    encoding: Encoding::Aac {
                        configuration: &asc,
                        sample_rate,
                        channels,
                    },
                    name: "audio",
                    language: "eng",
                },
            ],
            vec![TrackOptions::default(), TrackOptions::default()],
            packets,
        )
        .unwrap();
        assert_eq!(stats.frames, 3);
        output.set_position(0);
        let mut reader = crate::container::webm::WebmReader::open(
            output,
            crate::container::webm::Limits::default(),
        )
        .unwrap();
        assert_eq!(reader.tracks.len(), 3);
        assert_eq!(reader.tracks[1].codec, "S_TEXT/ASS");
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[2].codec, "A_AAC");
        assert_eq!(reader.tracks[2].codec_private, asc);
        let (audio_packet_index, audio_packet) = reader
            .packets
            .iter()
            .enumerate()
            .find(|(_, p)| p.track == reader.tracks[2].number)
            .unwrap();
        assert_eq!(audio_packet.pts_ns, 400_000_000);
        assert_eq!(audio_packet.duration_ns, Some(21_333_333));
        assert_eq!(audio_packet.discard_padding_ns, 1_000_000);
        assert_eq!(
            reader.read_packet(audio_packet_index).unwrap(),
            audio_payload
        );
        let subtitles: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == reader.tracks[1].number)
            .map(|(i, p)| (i, p.pts_ns, p.duration_ns))
            .collect();
        assert_eq!(subtitles.len(), 3);
        for ((index, pts, duration), expected) in subtitles.into_iter().zip(times) {
            assert_eq!(pts, expected as i64);
            assert_eq!(duration, Some(100_000_000));
            assert_eq!(reader.read_packet(index).unwrap(), payload);
        }
        assert!(
            reader
                .packets
                .windows(2)
                .all(|p| p[0].pts_ns <= p[1].pts_ns)
        );
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn native_crop_scaling_matches_cropped_rotated_packed_source() {
        use crate::playback_native::{NativeReader, RawFrame, surface_to_packed};
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new(
            "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
        )
        .unwrap();
        for bytes in [
            include_bytes!("../../../../tests/fixtures/display/par-2x1.mp4").as_slice(),
            include_bytes!("../../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
        ] {
            let mut reader =
                NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
            assert!(reader.enable_shared_surfaces().unwrap());
            let RawFrame::Surface { surface, colour } = reader.read_frame_raw().unwrap().unwrap()
            else {
                panic!("native surface required")
            };
            let source = surface_to_packed(&surface, colour).unwrap();
            let depth = source.depth;
            let sample_bytes = if depth == 8 { 1 } else { 2 };
            let mut renderer =
                MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722).unwrap();
            for rotation in [0, 90, 180, 270] {
                let rotated = source.rotated(rotation).unwrap();
                let sw = rotated.frame.width;
                let sh = rotated.frame.height;
                let [x, y, w, h] = [2, 2, sw - 4, sh - 4];
                let planes = rotated.plane_data().unwrap();
                let mut data = Vec::new();
                for (index, plane) in planes.into_iter().enumerate() {
                    let divisor = if index == 0 { 1 } else { 2 };
                    let stride = sw / divisor;
                    for row in y / divisor..(y + h) / divisor {
                        let begin = (row * stride + x / divisor) * sample_bytes;
                        data.extend_from_slice(&plane[begin..begin + w / divisor * sample_bytes]);
                    }
                }
                let cropped = PackedPlanar::new(
                    crate::native_geometry::GeometryFrame {
                        width: w,
                        height: h,
                        subsampling: Some([2, 2]),
                        data,
                    },
                    depth,
                    colour,
                )
                .unwrap();
                let output = [w * 2 + 2, h * 2 + 2];
                renderer.set_window([0.0, 0.0, 1.0, 1.0]).unwrap();
                let expected = renderer
                    .render_packed_sized(&device, &queue, &cropped, 0, None, output)
                    .unwrap()
                    .download()
                    .unwrap();
                renderer
                    .set_window([
                        x as f32 / sw as f32,
                        y as f32 / sh as f32,
                        (x + w) as f32 / sw as f32,
                        (y + h) as f32 / sh as f32,
                    ])
                    .unwrap();
                let actual = renderer
                    .render_surface_sized(
                        &device, &queue, &surface, colour, 0, None, rotation, output,
                    )
                    .unwrap()
                    .download()
                    .unwrap();
                for (a, b) in [&actual.y, &actual.cb, &actual.cr].into_iter().zip([
                    &expected.y,
                    &expected.cb,
                    &expected.cr,
                ]) {
                    let codes = |p: &[u8]| -> Vec<u16> {
                        if depth == 8 {
                            p.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            p.chunks_exact(2)
                                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                                .collect()
                        }
                    };
                    let worst = codes(a)
                        .into_iter()
                        .zip(codes(b))
                        .map(|(a, b)| a.abs_diff(b))
                        .max()
                        .unwrap();
                    assert!(
                        worst <= 1,
                        "depth={depth} rotation={rotation} worst={worst}"
                    );
                }
            }
        }
    }
    mod odd_crop;
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_crop_matches_independent_rgb_crop_in_all_planes() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new(
            "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
        )
        .unwrap();
        let source: Vec<u8> = (0..11 * 9)
            .flat_map(|i| {
                [
                    (i * 73 % 256) as u8,
                    (i * 31 % 256) as u8,
                    (i * 113 % 256) as u8,
                ]
            })
            .collect();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for [x, y, w, h] in [[2, 2, 7, 5], [1, 1, 8, 6], [0, 0, 11, 9]] {
                    let cropped: Vec<u8> = (y..y + h)
                        .flat_map(|row| {
                            source[(row * 11 + x) * 3..(row * 11 + x + w) * 3]
                                .iter()
                                .copied()
                        })
                        .collect();
                    for output_size in [
                        [w, h],
                        [w * 2 + 1, h * 2 + 1],
                        [w.div_ceil(2), h.div_ceil(2)],
                    ] {
                        renderer.set_window([0.0, 0.0, 1.0, 1.0]).unwrap();
                        let expected = renderer
                            .render_rgb_sized(&device, &queue, &cropped, [w, h], output_size)
                            .unwrap()
                            .download()
                            .unwrap();
                        renderer
                            .set_window([
                                x as f32 / 11.0,
                                y as f32 / 9.0,
                                (x + w) as f32 / 11.0,
                                (y + h) as f32 / 9.0,
                            ])
                            .unwrap();
                        let actual = renderer
                            .render_rgb_sized(&device, &queue, &source, [11, 9], output_size)
                            .unwrap()
                            .download()
                            .unwrap();
                        for (a, b) in [&actual.y, &actual.cb, &actual.cr].into_iter().zip([
                            &expected.y,
                            &expected.cb,
                            &expected.cr,
                        ]) {
                            let codes = |p: &[u8]| -> Vec<u16> {
                                if depth == 8 {
                                    p.iter().map(|&v| u16::from(v)).collect()
                                } else {
                                    p.chunks_exact(2)
                                        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                        .collect()
                                }
                            };
                            assert!(
                                codes(a)
                                    .into_iter()
                                    .zip(codes(b))
                                    .all(|(a, b)| a.abs_diff(b) <= 1),
                                "crop {x}:{y}:{w}:{h} output={output_size:?} depth={depth} full={full}"
                            );
                        }
                    }
                }
                assert!(renderer.set_window([0.0, 0.0, f32::NAN, 1.0]).is_err());
                assert!(renderer.set_window([0.5, 0.0, 0.5, 1.0]).is_err());
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn native_shader_export_stream_encodes_muxes_and_reopens() {
        use crate::hardware_export::{
            EncodedTime, EncoderCodec, HardwareFrame, ShaderFrame, TrackOptions,
        };
        use crate::playback_native::{NativeReader, RawFrame};
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let mut reader = NativeReader::new(
            std::io::Cursor::new(
                include_bytes!("../../../../tests/fixtures/display/par-2x1.mp4").as_slice(),
            ),
            64 * 1024 * 1024,
        )
        .unwrap();
        assert!(reader.enable_shared_surfaces().unwrap());
        let RawFrame::Surface { surface, colour } = reader.read_frame_raw().unwrap().unwrap()
        else {
            panic!("native decoder surface required")
        };
        drop(reader);
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        for (depth, codec) in [(8, EncoderCodec::H264), (10, EncoderCodec::Hevc)] {
            let mut renderer =
                MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722).unwrap();
            let pts = [0, 1, 4];
            let duration = [1, 3, 2];
            let frames = (0..3).map(|i| {
                Ok(ShaderFrame {
                    frame: HardwareFrame {
                        surface: surface.clone(),
                        pts: EncodedTime {
                            value: pts[i],
                            timescale: 25,
                        },
                        duration: EncodedTime {
                            value: duration[i],
                            timescale: 25,
                        },
                        force_keyframe: i == 2,
                    },
                    colour,
                    grade: None,
                    rotation: 90,
                })
            });
            let mut output = std::io::Cursor::new(Vec::new());
            let stats = crate::hardware_export::write_shader_video(
                &mut output,
                frames,
                &mut renderer,
                &device,
                &queue,
                [32, 48],
                codec,
                2_000_000,
                TrackOptions::default(),
            )
            .unwrap();
            assert_eq!(stats.frames, 3);
            assert!(stats.compressed_bytes > 0);
            output.set_position(0);
            let mut reopened = NativeReader::new(output, 64 * 1024 * 1024).unwrap();
            for expected in pts {
                let _frame = reopened.read_frame_raw().unwrap().unwrap();
                assert_eq!(reopened.dimensions(), [32, 48]);
                let (ticks, scale) = reopened.current_pts().unwrap();
                assert_eq!(ticks as f64 / scale as f64, expected as f64 / 25.0);
            }
            assert!(reopened.read_frame_raw().unwrap().is_none());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_rgb_scaling_matches_independent_bilinear_source_sampling() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new(
            "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
        )
        .unwrap();
        let rgb: Vec<u8> = (0..7 * 5)
            .flat_map(|i| {
                [
                    (i * 73 % 256) as u8,
                    (i * 31 % 256) as u8,
                    (i * 113 % 256) as u8,
                ]
            })
            .collect();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for [w, h] in [[3, 2], [4, 3], [12, 8], [11, 9], [7, 5]] {
                    let sample = |x: usize, y: usize| {
                        let sx = ((x as f64 + 0.5) * 7.0 / w as f64 - 0.5).clamp(0.0, 6.0);
                        let sy = ((y as f64 + 0.5) * 5.0 / h as f64 - 0.5).clamp(0.0, 4.0);
                        let ix = sx.floor() as usize;
                        let iy = sy.floor() as usize;
                        let fx = sx - ix as f64;
                        let fy = sy - iy as f64;
                        std::array::from_fn::<_, 3, _>(|c| {
                            let read = |x, y| f64::from(rgb[(y * 7 + x) * 3 + c]) / 255.0;
                            let top = read(ix, iy) * (1.0 - fx) + read((ix + 1).min(6), iy) * fx;
                            let bottom = read(ix, (iy + 1).min(4)) * (1.0 - fx)
                                + read((ix + 1).min(6), (iy + 1).min(4)) * fx;
                            top * (1.0 - fy) + bottom * fy
                        })
                    };
                    let planes = renderer
                        .render_rgb_sized(&device, &queue, &rgb, [7, 5], [w, h])
                        .unwrap()
                        .download()
                        .unwrap();
                    for (channel, plane) in
                        [&planes.y, &planes.cb, &planes.cr].into_iter().enumerate()
                    {
                        let pw = if channel == 0 { w } else { w.div_ceil(2) };
                        let codes: Vec<u16> = if depth == 8 {
                            plane.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            plane
                                .chunks_exact(2)
                                .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                .collect()
                        };
                        for (i, code) in codes.into_iter().enumerate() {
                            let x = i % pw;
                            let y = i / pw;
                            let color = if channel == 0 {
                                sample(x, y)
                            } else {
                                let mut sum = [0.0; 3];
                                for dy in 0..2 {
                                    for dx in 0..2 {
                                        let value = sample(
                                            (x * 2 + dx).min(w - 1),
                                            (y * 2 + dy).min(h - 1),
                                        );
                                        for c in 0..3 {
                                            sum[c] += value[c] * 0.25;
                                        }
                                    }
                                }
                                sum
                            };
                            let luma = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
                            let value = match channel {
                                0 => luma,
                                1 => (color[2] - luma) / (2.0 * (1.0 - 0.0722)),
                                _ => (color[0] - luma) / (2.0 * (1.0 - 0.2126)),
                            };
                            let factor = if depth == 8 { 1.0 } else { 4.0 };
                            let max = if depth == 8 { 255.0 } else { 1023.0 };
                            let expected = if channel == 0 {
                                (if full { 0.0 } else { 16.0 * factor })
                                    + value * (if full { max } else { 219.0 * factor })
                            } else {
                                128.0 * factor + value * (if full { max } else { 224.0 * factor })
                            };
                            assert!(
                                (f64::from(code) - expected.clamp(0.0, max).round()).abs() <= 1.0,
                                "{w}x{h} depth={depth} full={full} channel={channel} at={x},{y}: {code} vs {expected}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_scales_native_output_and_aligns_chroma_to_output_pixels() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x,uv.y,step(0.7,uv.x+uv.y)); }").unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let input =
                    fvid_vt::Surface::metal_black(&device, &queue, 7, 5, depth, full).unwrap();
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for size in [[3, 2], [4, 3], [12, 8], [11, 9]] {
                    let reference = renderer
                        .render_rgb(&device, &queue, &vec![0; size[0] * size[1] * 3], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    let rgb = renderer
                        .render_rgb_sized(&device, &queue, &[0; 7 * 5 * 3], [7, 5], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    assert_eq!(rgb.y, reference.y);
                    assert_eq!(rgb.cb, reference.cb);
                    assert_eq!(rgb.cr, reference.cr);
                    for rotation in [0, 90, 180, 270] {
                        let output = renderer
                            .render_surface_sized(
                                &device,
                                &queue,
                                &input,
                                AvcColour::default(),
                                0,
                                None,
                                rotation,
                                size,
                            )
                            .unwrap();
                        assert_eq!([output.width(), output.height()], size);
                        let actual = output.download().unwrap();
                        assert_eq!(actual.y, reference.y);
                        assert_eq!(actual.cb, reference.cb);
                        assert_eq!(actual.cr, reference.cr);
                    }
                }
                assert!(
                    renderer
                        .render_rgb_sized(&device, &queue, &[0; 3], [1, 1], [0, 1])
                        .is_err()
                );
                assert!(
                    renderer
                        .render_rgb_sized(&device, &queue, &[0; 3], [1, 1], [usize::MAX, 1])
                        .is_err()
                );
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_rotates_output_geometry_and_chroma_sampling() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x,uv.y,step(0.7,uv.x+uv.y)); }").unwrap();
        for [w, h] in [[8, 6], [7, 5]] {
            for depth in [8, 10] {
                let input =
                    fvid_vt::Surface::metal_black(&device, &queue, w, h, depth, false).unwrap();
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722)
                        .unwrap();
                for rotation in [0, 90, 180, 270] {
                    let size = if rotation == 90 || rotation == 270 {
                        [h, w]
                    } else {
                        [w, h]
                    };
                    let reference = renderer
                        .render_rgb(&device, &queue, &vec![0; size[0] * size[1] * 3], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    let output = renderer
                        .render_surface_rotated(
                            &device,
                            &queue,
                            &input,
                            AvcColour::default(),
                            0,
                            None,
                            rotation,
                        )
                        .unwrap();
                    assert_eq!([output.width(), output.height()], size);
                    let actual = output.download().unwrap();
                    assert_eq!(actual.y, reference.y, "Y rotation {rotation}");
                    assert_eq!(actual.cb, reference.cb, "Cb rotation {rotation}");
                    assert_eq!(actual.cr, reference.cr, "Cr rotation {rotation}");
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_chroma_averages_aligned_pixels_including_odd_edges() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x, uv.y, step(0.7,uv.x+uv.y)); }").unwrap();
        for [w, h] in [[8usize, 6], [7, 5]] {
            for depth in [8, 10] {
                for full in [false, true] {
                    let mut renderer =
                        MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                            .unwrap();
                    let planes = renderer
                        .render_rgb(&device, &queue, &vec![0; w * h * 3], [w, h])
                        .unwrap()
                        .download()
                        .unwrap();
                    let rgb = |x: usize, y: usize| {
                        let u = (x.min(w - 1) as f64 + 0.5) / w as f64;
                        let v = (y.min(h - 1) as f64 + 0.5) / h as f64;
                        [u * u, v, if u + v >= 0.7 { 1.0 } else { 0.0 }]
                    };
                    let factor = if depth == 8 { 1.0 } else { 4.0 };
                    let max = if depth == 8 { 255.0 } else { 1023.0 };
                    let yuv = |rgb: [f64; 3]| {
                        let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                        [
                            y,
                            (rgb[2] - y) / (2.0 * (1.0 - 0.0722)),
                            (rgb[0] - y) / (2.0 * (1.0 - 0.2126)),
                        ]
                    };
                    for (channel, data) in
                        [&planes.y, &planes.cb, &planes.cr].into_iter().enumerate()
                    {
                        let codes: Vec<u16> = if depth == 8 {
                            data.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            data.chunks_exact(2)
                                .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                .collect()
                        };
                        let width = if channel == 0 { w } else { w.div_ceil(2) };
                        for (i, code) in codes.into_iter().enumerate() {
                            let (x, y) = (i % width, i / width);
                            let value = if channel == 0 {
                                yuv(rgb(x, y))[0]
                            } else {
                                let mut average = [0.0; 3];
                                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                                    for (sum, v) in
                                        average.iter_mut().zip(rgb(x * 2 + dx, y * 2 + dy))
                                    {
                                        *sum += v / 4.0;
                                    }
                                }
                                yuv(average)[channel]
                            };
                            let expected = if channel == 0 {
                                (if full { 0.0 } else { 16.0 * factor })
                                    + value * (if full { max } else { 219.0 * factor })
                            } else {
                                128.0 * factor + value * (if full { max } else { 224.0 * factor })
                            };
                            assert!(
                                (f64::from(code) - expected.clamp(0.0, max).round()).abs() <= 1.0,
                                "{w}x{h} depth={depth} full={full} plane={channel} at={x},{y}: {code} vs {expected}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn display_shader_converts_to_native_encoder_yuv() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                let surface = renderer
                    .render_rgb(&device, &queue, &vec![80; 64 * 64 * 3], [64, 64])
                    .unwrap();
                let planes = surface.download().unwrap();
                let native =
                    fvid_vt::Surface::metal_black(&device, &queue, 64, 64, depth, full).unwrap();
                let native_output = renderer
                    .render_surface(&device, &queue, &native, AvcColour::default(), 1, None)
                    .unwrap();
                let native_planes = native_output.download().unwrap();
                assert_eq!(native_planes.y, planes.y);
                assert_eq!(native_planes.cb, planes.cb);
                assert_eq!(native_planes.cr, planes.cr);
                let codec = if depth == 8 {
                    fvid_vt::EncoderCodec::H264
                } else {
                    fvid_vt::EncoderCodec::Hevc
                };
                let mut encoder = fvid_vt::Encoder::new_with_depth(codec, 64, 64, depth).unwrap();
                assert!(
                    !encoder
                        .encode(&native_output, 0, 1, 25)
                        .unwrap()
                        .data
                        .is_empty()
                );
                let y: f64 = 0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.6;
                let cb = (0.6 - y) / (2.0 * (1.0 - 0.0722));
                let cr = (0.2 - y) / (2.0 * (1.0 - 0.2126));
                let factor = if depth == 8 { 1.0 } else { 4.0 };
                let max = if depth == 8 { 255.0 } else { 1023.0 };
                let y_range = if full { max } else { 219.0 * factor };
                let c_range = if full { max } else { 224.0 * factor };
                let expected: [f64; 3] = [
                    ((if full { 0.0 } else { 16.0 * factor }) + y * y_range).round(),
                    (128.0 * factor + cb * c_range).round(),
                    (128.0 * factor + cr * c_range).round(),
                ];
                for (plane, expected) in [&planes.y, &planes.cb, &planes.cr]
                    .into_iter()
                    .zip(expected)
                {
                    let codes: Vec<u16> = if depth == 8 {
                        plane.iter().map(|&v| u16::from(v)).collect()
                    } else {
                        plane
                            .chunks_exact(2)
                            .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                            .collect()
                    };
                    assert!(
                        codes
                            .iter()
                            .all(|&code| (f64::from(code) - expected).abs() <= 1.0),
                        "depth={depth} full={full} expected={expected} actual={:?}",
                        &codes[..8]
                    );
                }
            }
        }
    }
    #[test]
    fn encoder_shader_validates_nv12_p010_and_ranges() {
        let shader = ColorShader::new(include_str!("../../../../shaders/grayscale.wgsl")).unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let source = shader.encoder_source(depth, full, 0.2126, 0.0722).unwrap();
                let module = naga::front::wgsl::parse_str(&source).unwrap();
                assert_eq!(module.entry_points.len(), 3);
                assert!(module.entry_points.iter().any(|e| e.name == "encoder_y"));
                assert!(module.entry_points.iter().any(|e| e.name == "encoder_uv"));
            }
        }
        assert!(shader.encoder_source(12, false, 0.2126, 0.0722).is_err());
        assert!(shader.encoder_source(8, false, f32::NAN, 0.0722).is_err());
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn shader_renders_into_native_encoder_surface() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        for depth in [8, 10] {
            let (y, uv, scale) = if depth == 8 {
                (83u16, 147u16, 1.0 / 255.0)
            } else {
                (337, 593, 64.0 / 65535.0)
            };
            let source = format!(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{ let p = array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0)); return vec4(p[i],0.0,1.0); }} @fragment fn y() -> @location(0) vec4<f32> {{ return vec4({},0.0,0.0,1.0); }} @fragment fn uv() -> @location(0) vec4<f32> {{ return vec4({},{},0.0,1.0); }}",
                f64::from(y) * scale,
                f64::from(uv) * scale,
                f64::from(uv) * scale
            );
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let formats = if depth == 8 {
                [wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm]
            } else {
                [
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                ]
            };
            let pipelines: Vec<_> = formats
                .into_iter()
                .zip(["y", "uv"])
                .map(|(format, entry)| {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: None,
                        layout: None,
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: Default::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })
                .collect();
            let surface = fvid_vt::Surface::metal_render(
                &device,
                &queue,
                64,
                64,
                depth,
                false,
                |pass, index| {
                    pass.set_pipeline(&pipelines[index]);
                    pass.draw(0..3, 0..1);
                },
            )
            .unwrap();
            let pixels = surface.download().unwrap();
            let codes = |data: &[u8]| -> Vec<u16> {
                if depth == 8 {
                    data.iter().map(|&v| u16::from(v)).collect()
                } else {
                    data.chunks_exact(2)
                        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                        .collect()
                }
            };
            assert!(codes(&pixels.y).iter().all(|&v| v == y));
            let partial = fvid_vt::Surface::metal_render(
                &device,
                &queue,
                64,
                64,
                depth,
                false,
                |pass, index| {
                    let side = if index == 0 { 64 } else { 32 };
                    pass.set_scissor_rect(0, 0, side / 2, side);
                    pass.set_pipeline(&pipelines[index]);
                    pass.draw(0..3, 0..1);
                },
            )
            .unwrap()
            .download()
            .unwrap();
            assert!(codes(&partial.y).iter().enumerate().all(|(i, &v)| v
                == if i % 64 < 32 {
                    y
                } else if depth == 8 {
                    16
                } else {
                    64
                }));
            for plane in [&partial.cb, &partial.cr] {
                assert!(codes(plane).iter().enumerate().all(|(i, &v)| v
                    == if i % 32 < 16 {
                        uv
                    } else if depth == 8 {
                        128
                    } else {
                        512
                    }));
            }
            assert!(
                codes(&pixels.cb)
                    .iter()
                    .chain(codes(&pixels.cr).iter())
                    .all(|&v| v == uv)
            );
            let codec = if depth == 8 {
                fvid_vt::EncoderCodec::H264
            } else {
                fvid_vt::EncoderCodec::Hevc
            };
            let mut encoder = fvid_vt::Encoder::new_with_depth(codec, 64, 64, depth).unwrap();
            assert!(!encoder.encode(&surface, 0, 1, 25).unwrap().data.is_empty());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and Main10 VideoToolbox"]
    fn metal_p010_output_initializes_exact_codes_and_encodes_main10() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .expect("physical normalized16 GPU required");
        assert_eq!(device.adapter_info().backend, wgpu::Backend::Metal);
        for full in [false, true] {
            let surface = fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 10, full).unwrap();
            let pixels = surface.download().unwrap();
            assert_eq!(pixels.depth, 10);
            assert!(pixels.y.chunks_exact(2).all(|p| u16::from_le_bytes(p.try_into().unwrap()) == if full {0} else {64}));
            assert!(
                pixels
                    .cb
                    .chunks_exact(2)
                    .chain(pixels.cr.chunks_exact(2))
                    .all(|p| u16::from_le_bytes(p.try_into().unwrap()) == 512)
            );
            let mut encoder =
                fvid_vt::Encoder::new_with_depth(fvid_vt::EncoderCodec::Hevc, 64, 64, 10).unwrap();
            let encoded = encoder.encode(&surface, 0, 1, 25).unwrap();
            let config =
                crate::codec::config::HevcConfig::parse(&encoded.decoder_configuration).unwrap();
            assert_eq!(config.profile, 2);
            assert_eq!(config.bit_depth_luma, 10);
            assert!(!encoded.data.is_empty());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn metal_output_surface_initializes_native_encoder_planes() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_eq!(device.adapter_info().backend, wgpu::Backend::Metal);
        assert!(fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 10, false).is_err());
        for full in [false, true] {
            let surface = fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 8, full).unwrap();
            let pixels = surface.download().unwrap();
            assert!(pixels.y.iter().all(|&v| v == if full { 0 } else { 16 }));
            assert!(pixels.cb.iter().chain(&pixels.cr).all(|&v| v == 128));
            let mut encoder = fvid_vt::Encoder::new(fvid_vt::EncoderCodec::H264, 64, 64).unwrap();
            assert!(!encoder.encode(&surface, 0, 1, 25).unwrap().data.is_empty());
        }
    }
    #[test]
    #[ignore = "requires physical GPU"]
    fn rgb_frames_run_display_shader_and_gpu_adjustment() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let rgb: Vec<u8> = (0..4096)
            .flat_map(|i| [(i % 256) as u8, (i * 7 % 256) as u8, (i * 13 % 256) as u8])
            .collect();
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let shared = Arc::new(rgb.clone());
        let window = [0.0, 0.0, 1.0, 1.0];
        assert!(
            gpu.upload_shared_rgb(
                &device,
                &queue,
                &shared,
                [64, 64],
                window,
                IDENTITY_ADJUST,
                super::IDENTITY_VIEW
            )
            .unwrap()
        );
        assert!(
            !gpu.upload_shared_rgb(
                &device,
                &queue,
                &shared,
                [64, 64],
                window,
                IDENTITY_ADJUST,
                super::IDENTITY_VIEW
            )
            .unwrap()
        );
        let adjust = [1.2, -12.0, 0.8, 0.7, 0.3];
        assert!(
            !gpu.upload_shared_rgb(
                &device,
                &queue,
                &shared,
                [64, 64],
                window,
                adjust,
                super::IDENTITY_VIEW
            )
            .unwrap()
        );
        let changed = read_picture(&device, &queue, &gpu, 64, 64);
        let mut expected = Vec::new();
        crate::player::adjust_rgb(&rgb, &mut expected, &adjust);
        assert!(
            changed
                .chunks_exact(4)
                .zip(expected.chunks_exact(3))
                .all(|(a, b)| (0..3).all(|c| a[c].abs_diff(b[c]) <= 1))
        );
        let next = Arc::new(vec![90; rgb.len()]);
        assert!(
            gpu.upload_shared_rgb(
                &device,
                &queue,
                &next,
                [64, 64],
                window,
                IDENTITY_ADJUST,
                super::IDENTITY_VIEW
            )
            .unwrap()
        );
        assert!(
            read_picture(&device, &queue, &gpu, 64, 64)
                .chunks_exact(4)
                .all(|p| p == [90, 90, 90, 255])
        );
        for adjust in [IDENTITY_ADJUST, [1.2, -12.0, 0.8, 0.7, 0.3]] {
            gpu.upload_rgb(
                &device,
                &queue,
                &rgb,
                [64, 64],
                1,
                [0.0, 0.0, 1.0, 1.0],
                adjust,
            )
            .unwrap();
            let actual = read_picture(&device, &queue, &gpu, 64, 64);
            let mut expected = Vec::new();
            if adjust == IDENTITY_ADJUST {
                expected = rgb.clone();
            } else {
                crate::player::adjust_rgb(&rgb, &mut expected, &adjust);
            }
            let worst = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(3))
                .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
                .max()
                .unwrap();
            assert!(worst <= 1, "GPU RGB adjustment error {worst}");
        }
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        let mut custom = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        custom
            .upload_rgb(
                &device,
                &queue,
                &rgb,
                [64, 64],
                1,
                [0.0, 0.0, 1.0, 1.0],
                IDENTITY_ADJUST,
            )
            .unwrap();
        assert!(
            read_picture(&device, &queue, &custom, 64, 64)
                .chunks_exact(4)
                .all(|p| p == [51, 102, 153, 255])
        );
        let picture = frame(|_, _| 128, |_, _| 80);
        gpu.upload(
            &device,
            &queue,
            &picture,
            1,
            [0.0, 0.0, 1.0, 1.0],
            IDENTITY_ADJUST,
            None,
            0,
        );
        assert_eq!(
            read_picture(&device, &queue, &gpu, 64, 64),
            painted(&device, &queue, &picture, None)
        );
    }
    use crate::color::hdr::{ColourDescription, HdrMetadata};
    use crate::color::log::Log;
    use crate::color::tonemap::{ContentLight, DisplayTarget};
    use crate::color::{Lut, Settings};

    /// The shader is written by hand next to the byte array that feeds it, so
    /// the one thing a machine can check without a driver is that naga accepts
    /// the WGSL and that the parameter block the upload writes is exactly as
    /// wide as the uniform buffer allocated for it.
    #[test]
    fn the_video_shader_validates_and_the_params_fit_the_uniform() {
        use naga::proc::Layouter;
        use naga::valid::{Capabilities, ValidationFlags, Validator};
        let module = naga::front::wgsl::parse_str(super::SHADER).expect("player shader must parse");
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .expect("player shader must validate without optional capabilities");
        let mut layouter = Layouter::default();
        let params = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Params"))
            .map(|(handle, _)| handle)
            .expect("shader declares Params");
        layouter
            .update(module.to_ctx())
            .expect("every shader type must have a layout");
        assert_eq!(
            layouter[params].size as u64, super::UNIFORM_BYTES,
            "Params must match the uniform buffer the upload writes"
        );
    }

    #[test]
    fn display_shader_validates_and_rejects_extra_resources() {
        ColorShader::new(include_str!("../../../../shaders/grayscale.wgsl")).unwrap();
        assert!(
            ColorShader::new("fn process_color(rgb: vec3<f32>) -> vec3<f32> { return rgb; }")
                .is_err()
        );
        assert!(ColorShader::new("var<private> extra: f32; fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }").is_err());
        assert!(ColorShader::new(&" ".repeat(65537)).is_err());
        for backend in [crate::Backend::Cpu, crate::Backend::Cuda] {
            assert!(configuration(backend, 0).is_err());
        }
        assert!(configuration(crate::Backend::Auto, 1).is_err());
    }

    #[test]
    #[ignore = "requires a physical GPU; failure to initialize is a failure"]
    fn display_shader_executes_after_colour_conversion() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let picture = frame(|_, _| 96, |_, _| 192);
        let baseline = painted(&device, &queue, &picture, None);
        let shader = ColorShader::new(include_str!("../../../../shaders/grayscale.wgsl")).unwrap();
        let mut gpu = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        let shown = show(&device, &queue, &mut gpu, &picture, None, IDENTITY_ADJUST);
        for (source, output) in baseline.chunks_exact(4).zip(shown.chunks_exact(4)) {
            let expected = (f32::from(source[0]) * 0.2126
                + f32::from(source[1]) * 0.7152
                + f32::from(source[2]) * 0.0722)
                .round() as i32;
            for channel in &output[..3] {
                assert!((i32::from(*channel) - expected).abs() <= 1);
            }
            assert_eq!(output[0], output[1]);
            assert_eq!(output[1], output[2]);
            assert_eq!(output[3], 255);
        }
    }

    #[test]
    #[ignore = "requires a physical GPU; source precision must be tested by readback"]
    fn source_precision_planes_use_gpu_conversion_and_display_shader() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let grade = Arc::new(slog3(33, Interpolation::Tetrahedral));
        let mut serial = 1;
        for depth in [8, 10, 12, 16, 8] {
            for subsampling in [[2, 2], [2, 1], [1, 1], [1, 2]] {
                for full in [false, true] {
                    let (width, height) = (9usize, 7usize);
                    let scale = 1u32 << (depth - 8);
                    let luma = width * height;
                    let chroma = width.div_ceil(subsampling[0]) * height.div_ceil(subsampling[1]);
                    let mut data = Vec::new();
                    for i in 0..luma + chroma * 2 {
                        let value = if i < luma {
                            (16 + (i * 37) % 220) as u32 * scale + scale - 1
                        } else {
                            128 * scale
                        };
                        if depth == 8 {
                            data.push(value as u8);
                        } else {
                            data.extend((value as u16).to_le_bytes());
                        }
                    }
                    let frame = PackedPlanar::new(
                        crate::native_geometry::GeometryFrame {
                            width,
                            height,
                            subsampling: Some(subsampling),
                            data,
                        },
                        depth,
                        AvcColour {
                            kr: 0.2126,
                            kb: 0.0722,
                            full,
                        },
                    )
                    .unwrap();
                    let mut cpu = Vec::new();
                    frame.to_rgb(&mut cpu, width * height * 3).unwrap();
                    gpu.upload_packed(
                        &device,
                        &queue,
                        &frame,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        None,
                        0,
                    )
                    .unwrap();
                    serial += 1;
                    let shown = read_picture(&device, &queue, &gpu, width, height);
                    assert!(
                        worst_between(&shown, &cpu).into_iter().all(|v| v <= 1),
                        "depth {depth} {subsampling:?} full={full}"
                    );
                    if depth > 8 {
                        let narrowed = frame.to_planar8(width * height * 3).unwrap();
                        let mut quantized = Vec::new();
                        crate::playback_native::planar8_to_rgb(
                            &narrowed,
                            &mut quantized,
                            width * height * 3,
                        )
                        .unwrap();
                        assert!(
                            cpu.iter().zip(&quantized).any(|(a, b)| a != b),
                            "test must distinguish source precision from 8-bit narrowing"
                        );
                        assert!(
                            shown
                                .chunks_exact(4)
                                .zip(quantized.chunks_exact(3))
                                .any(|(a, b)| a[..3] != *b),
                            "GPU must preserve source low bits"
                        );
                    }
                    gpu.upload_packed(
                        &device,
                        &queue,
                        &frame,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        Some(&grade),
                        0,
                    )
                    .unwrap();
                    serial += 1;
                    let graded = read_picture(&device, &queue, &gpu, width, height);
                    grade.apply(&mut cpu);
                    assert!(
                        worst_between(&graded, &cpu).into_iter().all(|v| v <= 2),
                        "graded depth {depth}"
                    );
                    let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
                    let mut custom = build_with_source(
                        &device,
                        wgpu::TextureFormat::Rgba8Unorm,
                        shader.compiled_source(),
                    );
                    custom
                        .upload_packed(
                            &device,
                            &queue,
                            &frame,
                            serial,
                            [0.0, 0.0, 1.0, 1.0],
                            IDENTITY_ADJUST,
                            Some(&grade),
                            0,
                        )
                        .unwrap();
                    let pixels = read_picture(&device, &queue, &custom, width, height);
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|pixel| pixel == [51, 102, 153, 255])
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical VideoToolbox and Metal"]
    fn hardware_surfaces_render_with_colour_and_custom_shader() {
        use crate::codec::config::{AvcConfig, HevcConfig};
        use crate::container::mp4::{Limits, Mp4Reader};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            ..Default::default()
        }))
        .unwrap();
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3<f32>(0.2, 0.4, 0.6); }").unwrap();
        let mut custom = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        let grade = Arc::new(Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings {
                log: Some(Log::SLog3),
                size: 33,
                interpolation: Interpolation::Tetrahedral,
                ..Settings::video(DisplayTarget::sdr(100.0))
            },
            Some(Lut::Three(Lut3d::from_fn(17, |rgb| {
                [rgb[0] * rgb[0], rgb[1], 1.0 - rgb[2]]
            }))),
        ));
        assert!(grade.is_gpu_grade());
        let mut serial = 0;
        for bytes in [
            include_bytes!("../../../../tests/fixtures/display/par-2x1.mp4").as_slice(),
            include_bytes!("../../../../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
            include_bytes!("../../../../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
        ] {
            let mut reader =
                Mp4Reader::open(std::io::Cursor::new(bytes), Limits::default()).unwrap();
            let track = reader
                .tracks()
                .iter()
                .position(|t| t.handler == *b"vide")
                .unwrap();
            let config = reader.tracks()[track].configuration.clone();
            let count = reader.tracks()[track].samples.len();
            let mut session = if reader.tracks()[track].codec == *b"avc1" {
                let config = AvcConfig::parse(&config).unwrap();
                fvid_vt::Session::new_avc_surface(
                    &config.sps,
                    &config.pps,
                    config.length_size,
                    false,
                )
                .unwrap()
            } else {
                let config = HevcConfig::parse(&config).unwrap();
                let sets = |kind| {
                    config
                        .arrays
                        .iter()
                        .filter(|a| a.nal_type == kind)
                        .flat_map(|a| a.units.iter().copied())
                        .collect::<Vec<_>>()
                };
                fvid_vt::Session::new_hevc_surface(
                    &sets(32),
                    &sets(33),
                    &sets(34),
                    config.length_size,
                    config.bit_depth_luma,
                    false,
                )
                .unwrap()
            };
            let mut packet = Vec::new();
            let mut frames = 0;
            for index in 0..count {
                reader.read_packet(track, index, &mut packet).unwrap();
                let Some(surface) = session.decode_surface(&packet).unwrap() else {
                    continue;
                };
                let planes = surface.download().unwrap();
                let colour = AvcColour {
                    kr: 0.2126,
                    kb: 0.0722,
                    full: planes.full_range,
                };
                let mut data = planes.y;
                data.extend(planes.cb);
                data.extend(planes.cr);
                let packed = PackedPlanar::new(
                    crate::native_geometry::GeometryFrame {
                        width: planes.width,
                        height: planes.height,
                        subsampling: Some([2, 2]),
                        data,
                    },
                    planes.depth,
                    colour,
                )
                .unwrap();
                for rotation in [0, 90, 180, 270] {
                    let turned = packed.rotated(rotation).unwrap();
                    for grade in [None, Some(&grade)] {
                        for window in [[0.0, 0.0, 1.0, 1.0], [0.1, 0.2, 0.8, 0.9]] {
                            for adjust in [IDENTITY_ADJUST, [1.1, 2.0, 0.9, 0.8, 0.1]] {
                                serial += 1;
                                gpu.upload_packed(
                                    &device, &queue, &turned, serial, window, adjust, grade, 0,
                                )
                                .unwrap();
                                let reference = read_picture(
                                    &device,
                                    &queue,
                                    &gpu,
                                    turned.frame.width,
                                    turned.frame.height,
                                );
                                // Same serial intentionally exercises changing storage layout.
                                gpu.import_surface_rotated(
                                    &device, &queue, &surface, colour, serial, window, adjust,
                                    grade, rotation,
                                )
                                .unwrap();
                                let shown = read_picture(
                                    &device,
                                    &queue,
                                    &gpu,
                                    turned.frame.width,
                                    turned.frame.height,
                                );
                                assert!(
                                    shown
                                        .iter()
                                        .zip(&reference)
                                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                                    "native surface conversion differs: depth {}, frame {index}, rotation {rotation}, crop {window:?}, adjust {adjust:?}, graded {}, max {}",
                                    planes.depth,
                                    grade.is_some(),
                                    shown
                                        .iter()
                                        .zip(&reference)
                                        .map(|(a, b)| a.abs_diff(*b))
                                        .max()
                                        .unwrap()
                                );
                                gpu.upload_packed(
                                    &device, &queue, &turned, serial, window, adjust, grade, 0,
                                )
                                .unwrap();
                                assert_eq!(
                                    read_picture(
                                        &device,
                                        &queue,
                                        &gpu,
                                        turned.frame.width,
                                        turned.frame.height
                                    ),
                                    reference
                                );
                            }
                        }
                    }
                }
                serial += 1;
                custom
                    .import_surface(
                        &device,
                        &queue,
                        &surface,
                        colour,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        None,
                    )
                    .unwrap();
                assert!(
                    read_picture(&device, &queue, &custom, planes.width, planes.height)
                        .chunks_exact(4)
                        .all(|p| p == [51, 102, 153, 255])
                );
                frames += 1;
            }
            assert!(frames >= 10);
        }

        // Also verify 3-plane PLANAR_420 ('y420') hardware surface zero-copy Metal import
        // through VideoGpu::import_surface_rotated across all rotations.
        let bytes = include_bytes!("../../../../tests/fixtures/display/par-2x1.mp4").as_slice();
        let mut reader = Mp4Reader::open(std::io::Cursor::new(bytes), Limits::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"vide")
            .unwrap();
        let config = AvcConfig::parse(&reader.tracks()[track].configuration).unwrap();
        let mut session =
            fvid_vt::Session::new(&config.sps, &config.pps, config.length_size).unwrap();
        let mut packet = Vec::new();
        reader.read_packet(track, 0, &mut packet).unwrap();
        let surface = session.decode_surface(&packet).unwrap().unwrap();
        let planes = surface.download().unwrap();
        let colour = AvcColour {
            kr: 0.2126,
            kb: 0.0722,
            full: planes.full_range,
        };
        let mut data = planes.y;
        data.extend(planes.cb);
        data.extend(planes.cr);
        let packed = PackedPlanar::new(
            crate::native_geometry::GeometryFrame {
                width: planes.width,
                height: planes.height,
                subsampling: Some([2, 2]),
                data,
            },
            planes.depth,
            colour,
        )
        .unwrap();
        for rotation in [0, 90, 180, 270] {
            let turned = packed.rotated(rotation).unwrap();
            serial += 1;
            gpu.upload_packed(
                &device,
                &queue,
                &turned,
                serial,
                [0.0, 0.0, 1.0, 1.0],
                IDENTITY_ADJUST,
                None,
                0,
            )
            .unwrap();
            let reference = read_picture(
                &device,
                &queue,
                &gpu,
                turned.frame.width,
                turned.frame.height,
            );
            gpu.import_surface_rotated(
                &device,
                &queue,
                &surface,
                colour,
                serial,
                [0.0, 0.0, 1.0, 1.0],
                IDENTITY_ADJUST,
                None,
                rotation,
            )
            .unwrap();
            let shown = read_picture(
                &device,
                &queue,
                &gpu,
                turned.frame.width,
                turned.frame.height,
            );
            assert!(
                shown
                    .iter()
                    .zip(&reference)
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "3-plane PLANAR_420 surface conversion differs at rotation {rotation}"
            );
        }
    }
    #[test]
    fn rgba_shader_templates_validate_without_a_device() {
        ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {return rgb;}").unwrap();
    }
    #[test]
    #[ignore = "requires physical GPU"]
    fn rgba_frames_composite_after_adjustment_without_transparent_color_bleeding() {
        let (device,queue)=headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type,wgpu::DeviceType::Cpu);
        let mut reader=crate::playback_native::NativeReader::software(std::io::Cursor::new(include_bytes!("../../../../tests/fixtures/playback-errors/ffv1-alpha-rgb-8-coder1-v1.mkv").as_slice()),32<<20).unwrap();
        reader.read_frame_raw().unwrap().unwrap();
        let rgba=Arc::new(reader.read_frame_raw().unwrap().unwrap().into_rgba(32<<20).unwrap());
        assert_eq!(rgba[31],0);assert!(rgba[28..31].iter().any(|&v|v!=0));
        let mut gpu=build(&device,wgpu::TextureFormat::Rgba8Unorm);
        let window=[0.,0.,1.,1.];
        assert!(gpu.upload_shared_rgb(&device,&queue,&rgba,[4,2],window,IDENTITY_ADJUST,super::IDENTITY_VIEW).unwrap());
        assert!(!gpu.upload_shared_rgb(&device,&queue,&rgba,[4,2],window,IDENTITY_ADJUST,super::IDENTITY_VIEW).unwrap());
        let expected=crate::playback_native::rgba_to_rgb_on_black(&rgba,1024).unwrap();
        let actual=read_picture(&device,&queue,&gpu,4,2);
        for (a,b) in actual.chunks_exact(4).zip(expected.chunks_exact(3)) {
            assert_eq!(a[3],255);for c in 0..3 {assert!(a[c].abs_diff(b[c])<=1);}
        }
        let actual=read_picture(&device,&queue,&gpu,8,4);
        for y in 0..4 {for x in 0..8 {
            let sx=((x as f64+0.5)*0.5-0.5).clamp(0.,3.);let sy=((y as f64+0.5)*0.5-0.5).clamp(0.,1.);
            let (lx,ly)=(sx.floor() as usize,sy.floor() as usize);let (hx,hy)=((lx+1).min(3),(ly+1).min(1));
            let (fx,fy)=(sx.fract(),sy.fract());
            for c in 0..3 {
                let mut want=0.;for (xx,yy,weight) in [(lx,ly,(1.-fx)*(1.-fy)),(hx,ly,fx*(1.-fy)),(lx,hy,(1.-fx)*fy),(hx,hy,fx*fy)] {
                    let p=&rgba[(yy*4+xx)*4..];want+=f64::from(p[c])*f64::from(p[3])/255.*weight;
                }
                assert!(actual[(y*8+x)*4+c].abs_diff(want.round() as u8)<=1,"RGBA scaled {x},{y},{c}");
            }
        }}
        let opaque=&rgba[24..28];let hidden=&rgba[28..32];
        let wrong=(f64::from(opaque[0])*0.25+f64::from(hidden[0])*0.75)*(f64::from(opaque[3])*0.25)/255.;
        let correct=f64::from(opaque[0])*f64::from(opaque[3])/255.*0.25;
        assert!((correct-wrong).abs()>2.,"fixture must expose unassociated interpolation bleed");
        let adjust=[1.2,-12.,0.8,0.7,0.3];
        assert!(!gpu.upload_shared_rgb(&device,&queue,&rgba,[4,2],window,adjust,super::IDENTITY_VIEW).unwrap());
        let rgb=rgba.chunks_exact(4).flat_map(|p|p[..3].iter().copied()).collect::<Vec<_>>();let mut adjusted=Vec::new();
        crate::player::adjust_rgb(&rgb,&mut adjusted,&adjust);
        let actual=read_picture(&device,&queue,&gpu,4,2);
        for ((a,b),p) in actual.chunks_exact(4).zip(adjusted.chunks_exact(3)).zip(rgba.chunks_exact(4)) {
            for c in 0..3 {let want=((u32::from(b[c])*u32::from(p[3])+127)/255) as u8;assert!(a[c].abs_diff(want)<=1);}
        }
        let shader=ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {return vec3(0.2,0.4,0.6);}").unwrap();
        let mut custom=build_with_source(&device,wgpu::TextureFormat::Rgba8Unorm,shader.compiled_source());
        custom.upload_rgba(&device,&queue,&rgba,[4,2],1,window,IDENTITY_ADJUST).unwrap();
        let actual=read_picture(&device,&queue,&custom,4,2);
        for (a,p) in actual.chunks_exact(4).zip(rgba.chunks_exact(4)) {for c in 0..3 {
            let want=((u32::from([51u8,102,153][c])*u32::from(p[3])+127)/255) as u8;assert!(a[c].abs_diff(want)<=1);
        }}
    }
    /// A device with no window, so a test can own the texture it paints into.
    /// A machine that reports no adapter has nothing here to measure against.
    fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
        headless_features(wgpu::Features::empty())
    }
    pub(super) fn headless_features(features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        }))
        .ok()?;
        Some((device, queue))
    }

    /// The codes one frame's draw leaves behind, on a pipeline of its own.
    fn painted(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
    ) -> Vec<u8> {
        paint_adjust(device, queue, frame, grade, IDENTITY_ADJUST)
    }

    /// …and the same draw with a settings bundle on it, for a test that has to
    /// set a slider's maths against the grade's.
    fn paint_adjust(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
        adjust: [f32; 5],
    ) -> Vec<u8> {
        let mut gpu = build(device, wgpu::TextureFormat::Rgba8Unorm);
        show(device, queue, &mut gpu, frame, grade, adjust)
    }

    /// One draw of `frame` as `gpu` holds it: the product's own `upload` filling
    /// the parameter buffer, the product's own pipeline, into a plain 8-bit
    /// target so the shader's `srgb` branch stays off and the bytes it writes are
    /// the coded RGB the CPU conversion also hands back. The same `gpu` drawn
    /// twice is how a grade changed over a picture that has not moved looks.
    fn show(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut VideoGpu,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
        adjust: [f32; 5],
    ) -> Vec<u8> {
        let (width, height) = (frame.width, frame.height);
        gpu.upload(device, queue, frame, 0, [0.0, 0.0, 1.0, 1.0], adjust, grade, 0);
        read_picture(device, queue, gpu, width, height)
    }
    fn read_picture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &VideoGpu,
        width: usize,
        height: usize,
    ) -> Vec<u8> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fvid test picture"),
            size: wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu
            .bind_group
            .as_ref()
            .expect("an upload binds the planes and the grid");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fvid test picture"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fvid test picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        queue.submit([encoder.finish()]);
        let stride = (width * 4).next_multiple_of(256) as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fvid test readback"),
            size: stride * height as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fvid test readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(height as u32),
                },
            },
            wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (done, waited) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = done.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("the readback must finish");
        waited
            .recv()
            .expect("the mapping callback must run")
            .expect("the buffer must map");
        let mut out = vec![0u8; width * height * 4];
        {
            let mapped = readback.slice(..).get_mapped_range().expect("mapped");
            for row in 0..height {
                let from = row * stride as usize;
                let to = (row + 1) * stride as usize;
                let keep = row * width * 4;
                out[keep..keep + width * 4].copy_from_slice(&mapped[from..to][..width * 4]);
            }
        }
        readback.unmap();
        out
    }

    /// A 64 by 64 frame whose luma walks the whole code range, so the picture
    /// holds samples the limited-range mapping clamps at both ends.
    fn frame(cb: impl Fn(usize, usize) -> u8, cr: impl Fn(usize, usize) -> u8) -> Planar8 {
        let (width, height) = (64, 64);
        let y: Vec<u8> = (0..width * height).map(|i| (i % 256) as u8).collect();
        let mut cb_plane = Vec::new();
        let mut cr_plane = Vec::new();
        for row in 0..height / 2 {
            for column in 0..width / 2 {
                cb_plane.push(cb(column, row));
                cr_plane.push(cr(column, row));
            }
        }
        Planar8 {
            width,
            height,
            chroma_width: width / 2,
            chroma_height: height / 2,
            y,
            cb: cb_plane,
            cr: cr_plane,
            colour: crate::playback_native::AvcColour::default(),
        }
    }

    /// The two plane routes measured against each other on a picture whose
    /// chroma is flat: a field with no gradient has nothing for the sampler to
    /// place differently, so the shader and the CPU conversion are left doing
    /// the same arithmetic and have to land on the same bytes. Four corners of
    /// the chroma square stand in for the neutral one, because a chroma of 128
    /// zeroes the very gains a matrix is made of — and a flat field cannot hide
    /// a wrong one: writing `2.4` where the shader scales Cr by `2.0` moves
    /// these bytes and fails the test, while leaving a neutral field alone.
    #[test]
    fn a_frame_with_flat_chroma_pays_the_shader_the_cpus_bytes() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
            let frame = frame(move |_, _| blue, move |_, _| red);
            let painted = painted(&device, &queue, &frame, None);
            let mut cpu = Vec::new();
            crate::playback_native::planar8_to_rgb(&frame, &mut cpu, 64 * 64 * 3)
                .expect("converted");
            for (index, (shown, other)) in painted
                .as_chunks::<4>()
                .0
                .iter()
                .zip(cpu.as_chunks::<3>().0)
                .enumerate()
            {
                for channel in 0..3 {
                    assert_eq!(
                        shown[channel], other[channel],
                        "{blue}/{red} pixel {index} channel {channel}: {shown:?} against {other:?}"
                    );
                }
            }
        }
    }

    /// Where the two routes are known to part, and by how much: the sampler
    /// sits a chroma sample half a luma column and half a row away from where
    /// the CPU applies it, so on a ramp that steps one code per sample the
    /// worst weight error is three quarters of a chroma code. One of those
    /// moves red by 1.80 RGB codes, so the whole difference is predicted to
    /// stay under 1.35 and two codes leave a driver's rounding somewhere to
    /// live; measured here, `[1, 1, 0]` — blue never moves, because its
    /// component is flat in this frame. The bound is what the interpolation
    /// costs, not what the matrix is worth: writing `2.4` for the `2.0` that
    /// scales Cr shows up here as `[11, 6, 0]`.
    #[test]
    fn a_chroma_ramp_divides_the_routes_by_its_interpolation() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let frame = frame(|_, _| 128, |i, _| 96 + i as u8);
        let painted = painted(&device, &queue, &frame, None);
        let mut cpu = Vec::new();
        crate::playback_native::planar8_to_rgb(&frame, &mut cpu, 64 * 64 * 3).expect("converted");
        let mut worst = [0u32; 3];
        for (pair, other) in painted
            .as_chunks::<4>()
            .0
            .iter()
            .zip(cpu.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let delta = pair[channel] as i32 - other[channel] as i32;
                worst[channel] = worst[channel].max(delta.unsigned_abs());
            }
        }
        assert!(
            worst.iter().all(|d| *d <= 2),
            "chroma interpolation moved the routes by {worst:?}"
        );
    }

    /// A plain BT.709 picture as its file states it.
    fn bt709() -> ColourDescription {
        ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        }
    }

    /// A grade that reads the grid for every channel of every pixel: a 1 000-nit
    /// PQ master with highlights rolled onto a 100-nit panel. Its plan moves
    /// between primaries, so no set of byte tables can stand in for it.
    fn hdr10(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(
            ColourDescription {
                transfer: 16,
                primaries: 9,
                ..bt709()
            },
            &HdrMetadata {
                light: ContentLight {
                    max_cll: 1_000.0,
                    max_fall: 400.0,
                },
                ..Default::default()
            },
            settings,
            None,
        )
    }

    /// A grade that separates into byte tables: gamma-2.2 codes over the same
    /// primaries the panel shows, so one curve per channel is the whole plan and
    /// the CPU paints from 768 bytes rather than a grid. The shader has to read
    /// those exact bytes, which is what makes a byte-for-byte match between the
    /// two routes worth having here.
    fn gamma(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(
            ColourDescription {
                transfer: 4,
                ..bt709()
            },
            &HdrMetadata::default(),
            settings,
            None,
        )
    }

    /// A camera-log grade: its plan names no tone map and moves no primaries, so
    /// a cheap look at the plan says it separates into byte tables. It does not —
    /// the curve's `to_linear` is negative below the toe, and the gamut compress
    /// desaturates a negative channel toward luma — so `Grade::new` measures that
    /// and leaves the grade on the grid, which is the route the shader takes.
    fn slog3(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.log = Some(Log::SLog3);
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(bt709(), &HdrMetadata::default(), settings, None)
    }

    /// The bytes the CPU route leaves for the window: planes to packed codes,
    /// then the grade applied to them, which is what makes a plane picture
    /// packed at all.
    fn cpu_graded(frame: &Planar8, grade: &Grade) -> Vec<u8> {
        let mut rgb = Vec::new();
        crate::playback_native::planar8_to_rgb(frame, &mut rgb, frame.width * frame.height * 3)
            .expect("converted");
        grade.apply(&mut rgb);
        rgb
    }

    /// The whole frame's worst move per channel between a picture the shader
    /// graded and the same picture the CPU graded.
    fn worst_between(shader: &[u8], cpu: &[u8]) -> [u32; 3] {
        let mut worst = [0u32; 3];
        for (shown, other) in shader.as_chunks::<4>().0.iter().zip(cpu.as_chunks::<3>().0) {
            for channel in 0..3 {
                let delta = shown[channel] as i32 - other[channel] as i32;
                worst[channel] = worst[channel].max(delta.unsigned_abs());
            }
        }
        worst
    }

    /// The two grading routes on the same table: what a plane picture looks like
    /// when the fragment shader looks its codes up in the baked table, against
    /// what it looks like when the CPU converts it to packed RGB and grades that.
    ///
    /// They have to land on the same bytes, because the choice between them is
    /// made frame by frame by whatever else the window needs — a snapshot takes
    /// the CPU route while the picture on screen takes the shader. Both read the
    /// identical table: the shader is handed whichever of the two the grade itself
    /// reads, a grid's nodes or its byte tables, and the codes it feeds it are the
    /// bytes the CPU indexes by, which is what the `round(rgb · 255)` in front of
    /// the lookup is for. Nothing in the two routes is an approximation of the
    /// other, so this says `assert_eq!` rather than a tolerance: a grid that
    /// arrived with its nodes in another order, a read mode the uniform told the
    /// shader but not the CPU, a byte table row shifted by one texel, a lookup
    /// before the range maths instead of after it — each moves these bytes.
    #[test]
    fn the_shader_and_the_cpu_read_the_same_table_to_the_byte() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for interpolation in Interpolation::ALL {
            for size in [17, 33, 64] {
                // Each grade is named with the table it has to reach the shader
                // by, so the three cannot all quietly take one route and leave the
                // other binding untested.
                for (grade, kind) in [
                    (hdr10(size, interpolation), "grid"),
                    (gamma(size, interpolation), "tables"),
                    (slog3(size, interpolation), "grid"),
                ] {
                    let grade = Arc::new(grade);
                    let look = match grade.shader_look() {
                        Some(ShaderLook::Grid(_)) => "grid",
                        Some(ShaderLook::Tables(_)) => "tables",
                        None => "none",
                    };
                    assert_eq!(look, kind, "a {kind} grade reached the shader as {look}");
                    // Four corners of the chroma square rather than the neutral
                    // one, which is what a wrong matrix hides; see the flat
                    // chroma test above.
                    for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
                        let frame = frame(move |_, _| blue, move |_, _| red);
                        let shader = painted(&device, &queue, &frame, Some(&grade));
                        let cpu = cpu_graded(&frame, &grade);
                        let worst = worst_between(&shader, &cpu);
                        assert_eq!(
                            worst,
                            [0, 0, 0],
                            "{interpolation:?} at a {size} {kind}, chroma {blue}/{red}: the \
                             routes parted by {worst:?}"
                        );
                    }
                }
            }
        }
    }

    /// The table a texture mirrors changes while the picture does not: a caller
    /// moves `--grid` or names another look over a frame that is held on screen.
    /// One pipeline drawn twice is what the window shows then, and the second
    /// answer has to be the second grade — not the first one still sitting in the
    /// texture, nor in a bind group that points at it. A grid of the same edge
    /// length is written into the texture already bound, a different length is a
    /// new texture and a new bind group, a grade that reads bytes rather than
    /// nodes leaves the grid behind altogether, and no grade at all is the switch
    /// turned off, so every way out of here is walked.
    #[test]
    fn a_grade_changed_over_a_held_picture_moves_the_bound_table() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let frame = frame(|_, _| 64, |_, _| 224);
        let steps: [(String, Grade); 7] = [
            (
                "trilinear 33 grid".into(),
                hdr10(33, Interpolation::Trilinear),
            ),
            // Same edge length, other read: the table stays, the uniform moves.
            (
                "tetrahedral 33 grid".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
            (
                "tetrahedral 17 grid".into(),
                hdr10(17, Interpolation::Tetrahedral),
            ),
            // Back to a length a texture of this one has had before, which is a
            // new texture all the same.
            (
                "tetrahedral 33 grid again".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
            ("nearest 64 grid".into(), hdr10(64, Interpolation::Nearest)),
            // A grade with no grid at all: the row of bytes replaces the 64 cubed
            // texture, and the switch names the other binding.
            ("byte tables".into(), gamma(33, Interpolation::Tetrahedral)),
            (
                "tetrahedral 33 grid after bytes".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
        ];
        for (index, (what, grade)) in steps.iter().enumerate() {
            let grade = Arc::new(grade.clone());
            let shown = show(
                &device,
                &queue,
                &mut gpu,
                &frame,
                Some(&grade),
                IDENTITY_ADJUST,
            );
            let worst = worst_between(&shown, &cpu_graded(&frame, &grade));
            assert_eq!(worst, [0, 0, 0], "step {index}, {what}: {worst:?}");
        }
        let shown = show(&device, &queue, &mut gpu, &frame, None, IDENTITY_ADJUST);
        let mut plain = Vec::new();
        crate::playback_native::planar8_to_rgb(&frame, &mut plain, 64 * 64 * 3).expect("converted");
        assert_eq!(
            worst_between(&shown, &plain),
            [0, 0, 0],
            "the last grade left the picture after the lookup was turned off"
        );
    }

    /// The colour half of the two routes, with the conversion taken out of it:
    /// the shader's own ungraded bytes are graded on the CPU and set against the
    /// same picture the shader graded, over a chroma ramp that reaches every
    /// code.
    ///
    /// Comparing a graded shader against a graded CPU leaves the planar→RGB step
    /// in the number too, and the two routes are up to two codes apart there
    /// where the chroma ramp crosses them (measured by the chroma ramp test
    /// above). A steep curve multiplies a code of input into tens of output —
    /// the same three grades set against the whole CPU route were measured part
    /// by up to fifty-six codes, every one of them that multiplication and not a
    /// second colour decision. This test isolates the colour, and it agrees byte
    /// for byte at every code the ramp reaches, for a grid read three ways and
    /// for the byte tables.
    #[test]
    fn the_shader_grades_its_own_codes_as_the_cpu_would() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for interpolation in Interpolation::ALL {
            for size in [17, 33, 64] {
                for grade in [
                    hdr10(size, interpolation),
                    gamma(size, interpolation),
                    slog3(size, interpolation),
                ] {
                    let grade = Arc::new(grade);
                    let frame = frame(|_, _| 128, |i, _| 96 + i as u8);
                    let plain = painted(&device, &queue, &frame, None);
                    let mut want: Vec<u8> = plain
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                        .collect();
                    grade.apply(&mut want);
                    let shown = painted(&device, &queue, &frame, Some(&grade));
                    let worst = worst_between(&shown, &want);
                    assert_eq!(
                        worst,
                        [0, 0, 0],
                        "{interpolation:?} at a {size} grid over a chroma ramp: {worst:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a physical GPU for float 1D LUT qualification"]
    fn sequential_1d_grading_preserves_float_nodes_and_long_tables() {
        use crate::color::lut::Lut1d;
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let picture = frame(|_, _| 128, |i, _| (16 + i % 220) as u8);
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let mut serial = 1;
        for interpolation in Interpolation::ALL {
            for size in [1, 2, 5, 128, 129, 4096, 16385, 300_000] {
                let data = std::array::from_fn(|channel| {
                    (0..size)
                        .map(|i| {
                            let x = if size == 1 {
                                0.25
                            } else {
                                i as f32 / (size - 1) as f32
                            };
                            match channel {
                                0 => x * x,
                                1 => x * 1.2 - 0.1,
                                _ => 1.0 - x,
                            }
                        })
                        .collect()
                });
                let line = Lut1d {
                    data,
                    domain_min: [-0.2, 0.1, -0.5],
                    domain_max: [1.2, 0.9, 1.5],
                };
                let grade = Arc::new(Grade::new(
                    bt709(),
                    &HdrMetadata::default(),
                    Settings {
                        log: Some(Log::SLog3),
                        gamut: Some(crate::color::Primaries::BT2020),
                        size: 17,
                        interpolation,
                        ..Settings::video(DisplayTarget::sdr(100.0))
                    },
                    Some(Lut::One(line)),
                ));
                assert!(grade.is_gpu_grade());
                assert!(grade.shader_look().is_none());
                gpu.upload(
                    &device,
                    &queue,
                    &picture,
                    serial,
                    [0.0, 0.0, 1.0, 1.0],
                    IDENTITY_ADJUST,
                    None,
                    0,
                );
                let plain = read_picture(&device, &queue, &gpu, 64, 64);
                let mut expected: Vec<u8> = plain
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect();
                grade.apply(&mut expected);
                gpu.upload(
                    &device,
                    &queue,
                    &picture,
                    serial,
                    [0.0, 0.0, 1.0, 1.0],
                    IDENTITY_ADJUST,
                    Some(&grade),
                    0,
                );
                let shown = read_picture(&device, &queue, &gpu, 64, 64);
                let worst = worst_between(&shown, &expected);
                assert!(
                    worst.iter().all(|&v| v <= 1),
                    "{interpolation:?}, {size}: {worst:?}"
                );
                assert_ne!(plain, shown);
                serial += 1;
            }
        }
    }

    #[test]
    #[ignore = "requires a physical GPU for sequential LUT qualification"]
    fn sequential_3d_grading_preserves_authored_domains() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let frame = frame(|_, _| 128, |i, _| (16 + i % 220) as u8);
        for interpolation in Interpolation::ALL {
            for size in [5, 17] {
                let mut settings = Settings::video(DisplayTarget::sdr(100.0));
                settings.size = 17;
                settings.interpolation = interpolation;
                settings.log = Some(Log::SLog3);
                let mut look = Lut3d::from_fn(size, |rgb| {
                    [rgb[0] * rgb[0], rgb[1] * 1.2 - 0.1, 1.0 - rgb[2]]
                });
                look.domain_min = [-0.2, 0.1, -0.5];
                look.domain_max = [1.2, 0.9, 1.5];
                let grade = Arc::new(Grade::new(
                    bt709(),
                    &HdrMetadata::default(),
                    settings,
                    Some(Lut::Three(look)),
                ));
                assert!(grade.is_gpu_grade());
                assert!(grade.shader_look().is_none());
                let plain = painted(&device, &queue, &frame, None);
                let mut expected: Vec<u8> = plain
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect();
                grade.apply(&mut expected);
                let shown = painted(&device, &queue, &frame, Some(&grade));
                let worst = worst_between(&shown, &expected);
                assert!(
                    worst.iter().all(|&v| v <= 1),
                    "{interpolation:?}, {size}: {worst:?}"
                );
                assert_ne!(shown, plain);
            }
        }
    }

    /// The bytes the CPU route leaves for the window when a picture is graded
    /// and then stepped through VLC's settings: converted, graded, the bundle
    /// over the graded codes, which is the order the draw path has always used.
    fn cpu_graded_and_adjusted(frame: &Planar8, grade: &Grade, bundle: &[f32; 5]) -> Vec<u8> {
        let rgb = cpu_graded(frame, grade);
        if *bundle == IDENTITY_ADJUST {
            return rgb;
        }
        let mut adjusted = Vec::new();
        crate::player::adjust_rgb(&rgb, &mut adjusted, bundle);
        adjusted
    }

    /// VLC's picture settings and a grade, both asked for at once: the order the
    /// window shows them in, and the space the sliders work in.
    ///
    /// The grade is the colour management, so it runs first and the sliders are
    /// stepped over the codes it leaves — the picture the viewer is looking at,
    /// which is the only picture a brightness slider can promise to brighten.
    /// Contrast and gamma over log codes before the curve unwraps them would be
    /// swallowed or doubled by the curve instead, and the CPU draw path has
    /// always graded first, so the shader that keeps a plane picture's planes
    /// has to follow it here. Every step of both routes is the same arithmetic
    /// on the same bytes, including the byte the lookup lands on before the
    /// sliders run, so this is an `assert_eq!` on the whole frame rather than a
    /// tolerance: a slider applied before the grade, a turn that reads its own
    /// answer back, a grid's float answer fed to the matrix instead of its byte,
    /// or the identity bundle stepped through the round trip the CPU skips, each
    /// moves these codes.
    #[test]
    fn a_grade_and_a_slider_reach_the_window_in_the_cpus_order() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        // Every one of the five numbers away from the identity, so no term of
        // either half of the bundle is left unexercised: contrast, brightness
        // through the lum offset, gamma, saturation and a hue turn — and the
        // same bundle turned over, which is the other half of the turn's maths.
        for bundle in [
            [1.3, 14.0, 1.0 / 1.25, 0.6, 0.35],
            [0.75, -22.0, 1.0 / 0.8, 1.7, -0.5],
            IDENTITY_ADJUST,
        ] {
            for interpolation in Interpolation::ALL {
                // One grade read from the grid, one from the byte tables, one
                // whose plan a cheap look calls separable and the measurement
                // sends back to the grid.
                for grade in [
                    hdr10(33, interpolation),
                    gamma(33, interpolation),
                    slog3(33, interpolation),
                ] {
                    let grade = Arc::new(grade);
                    for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
                        let frame = frame(move |_, _| blue, move |_, _| red);
                        let shader = paint_adjust(&device, &queue, &frame, Some(&grade), bundle);
                        let worst = worst_between(
                            &shader,
                            &cpu_graded_and_adjusted(&frame, &grade, &bundle),
                        );
                        assert_eq!(
                            worst,
                            [0, 0, 0],
                            "bundle {bundle:?}, {interpolation:?}, chroma {blue}/{red}: the \
                             routes parted by {worst:?}"
                        );
                    }
                }
            }
        }
    }

    /// A slider moved over a picture the window is holding, with a grade on it:
    /// the frame, its planes and its table all stay, so the only thing that
    /// changes is the parameter block, and the switch that turns the bundle's
    /// round trip on rides in it. Drawn twice over one pipeline, the second
    /// answer has to be the second bundle's and the third the first one's again,
    /// because a stale switch would leave the picture either graded without the
    /// slider the viewer just moved, or stepped through the matrix when nothing
    /// asked for it.
    #[test]
    fn a_slider_moved_over_a_held_graded_picture_reaches_the_shader() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let frame = frame(|_, _| 64, |_, _| 224);
        let grade = Arc::new(hdr10(33, Interpolation::Tetrahedral));
        let bundle = [1.25, 10.0, 1.0 / 1.2, 0.8, -0.3];
        for step in [IDENTITY_ADJUST, bundle, IDENTITY_ADJUST] {
            let shown = show(&device, &queue, &mut gpu, &frame, Some(&grade), step);
            let worst = worst_between(&shown, &cpu_graded_and_adjusted(&frame, &grade, &step));
            assert_eq!(
                worst,
                [0, 0, 0],
                "step {step:?} over a held picture: {worst:?}"
            );
        }
    }

    /// The same bundle with no grade in front of it, which is the picture
    /// settings' own parity between the routes: the shader turns the planes it
    /// was given, the CPU solves BT.601 back out of the bytes the converter
    /// stored and turns that. Where both routes' samples survive the conversion
    /// the two are the same maths with one stored byte in the middle, so what
    /// separates them is what that rounding costs at either end — measured here,
    /// on four corners of the chroma square and a turn that scales no channel
    /// down, because a wrong turn cannot hide at a step or two.
    ///
    /// The picture is deliberately mild at both ends: a sample outside the coded
    /// range makes the routes incomparable rather than disagreeing, because the
    /// converter clamps it and the packed route then adjusts a luma the file
    /// never carried. The last two frames are exactly that, kept in the test as
    /// the record of it — thirty-two codes for a luma of 0 under a contrast of
    /// 1.3, and fifty-three for chroma at the top of the range — every one of
    /// them the shader's stored sample against the black or white the converter
    /// left behind, which no maths recovers afterwards.
    #[test]
    fn a_slider_over_a_plain_picture_costs_a_rounding_step() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let plains = |blue: u8, red: u8, luma: &dyn Fn(usize) -> u8| {
            let (width, height) = (64, 64);
            Planar8 {
                width,
                height,
                chroma_width: width / 2,
                chroma_height: height / 2,
                y: (0..width * height).map(luma).collect(),
                cb: vec![blue; width * height / 4],
                cr: vec![red; width * height / 4],
                colour: crate::playback_native::AvcColour::default(),
            }
        };
        let within = |blue, red| plains(blue, red, &|i| (60 + i % 91) as u8);
        for bundle in [
            [1.15, 8.0, 1.0 / 1.1, 0.7, 0.4],
            [1.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0, -1.0, 0.0],
            IDENTITY_ADJUST,
        ] {
            for (blue, red) in [(128u8, 128u8), (104, 152), (152, 104), (104, 104)] {
                let frame = within(blue, red);
                let shader = paint_adjust(&device, &queue, &frame, None, bundle);
                let mut rgb = Vec::new();
                crate::playback_native::planar8_to_rgb(&frame, &mut rgb, 64 * 64 * 3)
                    .expect("converted");
                let mut cpu = Vec::new();
                crate::player::adjust_rgb(&rgb, &mut cpu, &bundle);
                let worst = worst_between(&shader, &cpu);
                assert!(
                    worst.iter().all(|d| *d <= 2),
                    "bundle {bundle:?} over a {blue}/{red} chroma: the routes parted by {worst:?}"
                );
            }
        }
        // Where the converter does clamp, the routes are not comparable and this
        // is the record of that rather than a tolerance for it: the plane route
        // has the stored sample, the packed route has the black or white the
        // converter left instead, and no maths afterwards recovers it.
        for (what, frame, bundle) in [
            (
                "luma below black",
                plains(128, 128, &|i| (i % 256) as u8),
                [1.3, 14.0, 1.0 / 1.25, 1.0, 0.0],
            ),
            (
                "chroma at the top of the range",
                plains(64, 224, &|i| (60 + i % 91) as u8),
                [1.0, 0.0, 1.0, 0.7, 0.4],
            ),
        ] {
            let shader = paint_adjust(&device, &queue, &frame, None, bundle);
            let mut rgb = Vec::new();
            crate::playback_native::planar8_to_rgb(&frame, &mut rgb, 64 * 64 * 3)
                .expect("converted");
            let mut cpu = Vec::new();
            crate::player::adjust_rgb(&rgb, &mut cpu, &bundle);
            let worst = worst_between(&shader, &cpu);
            assert!(
                worst.iter().any(|d| *d > 20),
                "{what}: the clamped route was expected to part by tens, {worst:?}"
            );
        }
    }

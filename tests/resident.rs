#![cfg(any(feature = "gpu", feature = "cuda"))]
use fvid::{Backend, Crop, ExecutionOptions, Header, Plan, Transform, resident::GpuPipeline};

#[test]
fn invalid_resident_plans_fail_before_device_initialization() {
    let header = Header::parse(b"YUV4MPEG2 W8 H6 C420\n").unwrap();
    let gpu = ExecutionOptions {
        backend: Backend::Metal,
        device: 0,
    };
    assert!(GpuPipeline::new(&header, &[], gpu, 1024).is_err());
    assert!(GpuPipeline::new(&header, &[Transform::default(); 257], gpu, 1024).is_err());
    for backend in [Backend::Cpu, Backend::Auto] {
        assert!(
            GpuPipeline::new(
                &header,
                &[Transform::default()],
                ExecutionOptions { backend, device: 0 },
                1024
            )
            .is_err()
        );
    }
    let crop = Transform {
        crop: Some(Crop {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        }),
        ..Default::default()
    };
    let invalid_after_crop = Transform {
        crop: Some(Crop {
            x: 2,
            y: 0,
            width: 4,
            height: 4,
        }),
        ..Default::default()
    };
    assert!(GpuPipeline::new(&header, &[crop, invalid_after_crop], gpu, 1024).is_err());
}

fn check_hardware(backend: Backend) {
    let cases = [
        (8, 6, "420"),
        (8, 6, "422"),
        (9, 7, "444"),
        (65, 17, "444"),
        (638, 480, "420"),
        (1920, 1080, "420"),
        (3840, 2160, "420"),
    ];
    for (width, height, format) in cases {
        let header =
            Header::parse(format!("YUV4MPEG2 W{width} H{height} C{format}\n").as_bytes()).unwrap();
        let transforms = [
            Transform {
                crop: Some(Crop {
                    x: 2,
                    y: 2,
                    width: width - 4,
                    height: height - 4,
                }),
                ..Default::default()
            },
            Transform {
                horizontal: true,
                ..Default::default()
            },
            Transform {
                vertical: true,
                ..Default::default()
            },
        ];
        let options = ExecutionOptions { backend, device: 0 };
        let mut gpu = GpuPipeline::new(&header, &transforms, options, 256 * 1024 * 1024)
            .expect("physical GPU required; no fallback");
        if backend == Backend::Metal {
            assert!(
                gpu.uses_uma_zero_copy(),
                "Apple Silicon Metal must use UMA zero-copy compute buffers"
            );
        }
        if backend != Backend::Gl {
            assert!(
                gpu.uses_compute(),
                "Non-GL GPU backends must execute storage-buffer compute shaders"
            );
        }
        let admitted = gpu.controlled_memory_bytes();
        if width == 8 && format == "420" {
            assert!(GpuPipeline::new(&header, &transforms, options, admitted - 1).is_err());
            let exact = GpuPipeline::new(&header, &transforms, options, admitted).unwrap();
            assert_eq!(exact.controlled_memory_bytes(), admitted);
        }
        assert!(gpu.upload(&[]).is_err());
        for frame in 0..3 {
            let input: Vec<_> = (0..gpu.input_len())
                .map(|i| ((i * 37 + i / width * 19 + frame * 53) % 251) as u8)
                .collect();
            let mut expected = input.clone();
            let mut h = header.clone();
            for transform in transforms {
                let plan = Plan::new(&h, transform, usize::MAX).unwrap();
                if let Some(c) = transform.crop {
                    h.width = c.width;
                    h.height = c.height;
                }
                let mut next = vec![0; h.frame_len().unwrap()];
                plan.apply(&expected, &mut next).unwrap();
                expected = next;
            }
            let resident = gpu.upload(&input).unwrap().process().unwrap();
            let stats = resident.transfers();
            assert_eq!(stats.uploads, frame as u64 + 1);
            assert_eq!(stats.downloads, frame as u64);
            assert_eq!(stats.filter_passes, (frame as u64 + 1) * 3);
            assert_eq!(resident.header().width, width - 4);
            let mut output = vec![0; expected.len()];
            let stats = resident.download(&mut output).unwrap();
            assert_eq!(output, expected);
            assert_eq!(stats.downloads, stats.uploads);
            assert_eq!(stats.upload_bytes, stats.uploads * input.len() as u64);
            assert_eq!(stats.download_bytes, stats.downloads * output.len() as u64);
        }
        // Discard on-device output without readback, then reuse immediately.
        let input = vec![113; gpu.input_len()];
        {
            let _resident = gpu.upload(&input).unwrap().process().unwrap();
        }
        assert_eq!(gpu.transfers().downloads, 3);
        let mut output = vec![0; gpu.output_len()];
        gpu.upload(&input)
            .unwrap()
            .process()
            .unwrap()
            .download(&mut output)
            .unwrap();
        assert!(output.iter().all(|&b| b == 113));
        assert_eq!(
            (
                gpu.transfers().uploads,
                gpu.transfers().downloads,
                gpu.transfers().filter_passes
            ),
            (5, 4, 15)
        );
        eprintln!(
            "resident_verified backend={backend} device={:?} size={width}x{height} format={format} uploads=5 downloads=4 filter_passes=15",
            gpu.device_name()
        );
    }
}

#[test]
#[ignore = "requires physical Metal GPU; no CPU fallback"]
fn metal_resident_chain() {
    check_hardware(Backend::Metal);
}
#[test]
#[ignore = "requires NVIDIA GPU, CUDA driver and NVRTC; no CPU fallback"]
fn cuda_resident_chain() {
    check_hardware(Backend::Cuda);
}

#[test]
#[cfg(all(target_os = "macos", feature = "gpu"))]
#[ignore = "requires physical VideoToolbox decoder/encoder and Metal GPU"]
fn metal_resident_surface_import_and_export_roundtrip() {
    use fvid::resident::{ByteShader, GpuStage};
    let bytes = include_bytes!("fixtures/display/par-2x1.mp4");
    let at = |tag: &[u8]| bytes.windows(4).position(|v| v == tag).unwrap() + 4;
    let word = |pos| u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    let ac = at(b"avcC");
    let length_size = (bytes[ac + 4] & 3) + 1;
    let mut pos = ac + 6;
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for _ in 0..bytes[ac + 5] & 31 {
        let size = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        sps.push(&bytes[pos..pos + size]);
        pos += size;
    }
    let count = bytes[pos];
    pos += 1;
    for _ in 0..count {
        let size = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;
        pps.push(&bytes[pos..pos + size]);
        pos += size;
    }
    let stsz = at(b"stsz");
    let stco = at(b"stco");

    // Test both NV12 (2-plane) and PLANAR_420 (3-plane) hardware decoder surfaces!
    for tri_planar in [false, true] {
        let mut session = if tri_planar {
            fvid_vt::Session::new(&sps, &pps, length_size).unwrap()
        } else {
            fvid_vt::Session::new_avc_surface(&sps, &pps, length_size, false).unwrap()
        };
        let mut packet_at = word(stco + 8);
        let mut first_surface = None;
        for index in 0..word(stsz + 8) {
            let size = word(stsz + 12 + 4 * index);
            if let Some(surface) = session
                .decode_surface(&bytes[packet_at..packet_at + size])
                .unwrap()
            {
                first_surface = Some(surface);
                break;
            }
            packet_at += size;
        }
        let surface = first_surface.expect("decoded hardware surface");
        let w = surface.width();
        let h = surface.height();
        let header = Header::parse(format!("YUV4MPEG2 W{w} H{h} C420\n").as_bytes()).unwrap();
        let crop_transform = Transform {
            crop: Some(Crop {
                x: 2,
                y: 2,
                width: w - 4,
                height: h - 4,
            }),
            horizontal: true,
            vertical: true,
        };
        let shader = ByteShader::new(
            "fn process_byte(value: u32, _plane: u32, _x: u32, _y: u32) -> u32 {\n    return (value + 5u) & 255u;\n}",
        )
        .unwrap();
        let stages = [GpuStage {
            transform: crop_transform,
            shader: Some(shader),
            #[cfg(feature = "cuda")]
            cuda_shader: None,
        }];
        let options = ExecutionOptions {
            backend: Backend::Metal,
            device: 0,
        };
        let mut gpu = GpuPipeline::with_stages(&header, &stages, options, 64 * 1024 * 1024)
            .expect("Metal GPU pipeline");

        // CPU reference from downloaded surface planes
        let downloaded = surface.download().unwrap();
        let mut cpu_input = Vec::with_capacity(gpu.input_len());
        cpu_input.extend_from_slice(&downloaded.y);
        cpu_input.extend_from_slice(&downloaded.cb);
        cpu_input.extend_from_slice(&downloaded.cr);
        let plan = Plan::new(&header, crop_transform, usize::MAX).unwrap();
        let mut cpu_transformed = vec![0u8; gpu.output_len()];
        plan.apply(&cpu_input, &mut cpu_transformed).unwrap();
        for b in &mut cpu_transformed {
            *b = b.wrapping_add(5);
        }

        // Zero-copy GPU surface import -> resident compute stages -> zero-copy GPU surface export
        let (exported_surface, stats) = gpu
            .upload_surface(&surface)
            .unwrap()
            .process()
            .unwrap()
            .export_surface(false)
            .unwrap();
        assert_eq!(stats.uploads, 1);
        assert_eq!(stats.downloads, 1);
        assert_eq!(stats.filter_passes, 1);
        assert_eq!(exported_surface.width(), w - 4);
        assert_eq!(exported_surface.height(), h - 4);

        let exported_planes = exported_surface.download().unwrap();
        let mut exported_bytes = Vec::with_capacity(gpu.output_len());
        exported_bytes.extend_from_slice(&exported_planes.y);
        exported_bytes.extend_from_slice(&exported_planes.cb);
        exported_bytes.extend_from_slice(&exported_planes.cr);
        assert_eq!(exported_bytes, cpu_transformed);
    }
}

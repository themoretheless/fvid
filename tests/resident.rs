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

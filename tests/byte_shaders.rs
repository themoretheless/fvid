#![cfg(feature = "gpu")]
use fvid::resident::{ByteShader, GpuPipeline, GpuStage};
use fvid::{Backend, ExecutionOptions, Header, Plan, Transform};

#[test]
fn validates_filters_and_rejects_invalid_programs_without_gpu() {
    for source in [
        include_str!("../shaders/negate.wgsl"),
        include_str!("../shaders/boxblur.wgsl"),
    ] {
        ByteShader::new(source).unwrap();
    }
    for source in [
        "",
        "fn process_byte() -> u32 { return 1u; }",
        "fn process_byte(v:u32,p:u32,x:u32,y:u32)->u32 { return v; } @compute @workgroup_size(1) fn extra() {}",
        "var<private> hidden: u32; fn process_byte(v:u32,p:u32,x:u32,y:u32)->u32 { return v; }",
    ] {
        assert!(ByteShader::new(source).is_err(), "accepted: {source}");
    }
    assert!(ByteShader::new(&" ".repeat(65537)).is_err());
}

#[test]
fn rejects_wgsl_on_cuda_before_initializing_driver() {
    let header = Header::parse(b"YUV4MPEG2 W8 H6 C420\n").unwrap();
    let stage = GpuStage {
        shader: Some(ByteShader::new(include_str!("../shaders/negate.wgsl")).unwrap()),
        ..Default::default()
    };
    let error = GpuPipeline::with_stages(
        &header,
        &[stage],
        ExecutionOptions {
            backend: Backend::Cuda,
            device: 0,
        },
        usize::MAX,
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("CUDA does not execute WGSL"));
}

#[test]
#[ignore = "requires a physical GPU; set FVID_SHADER_BACKEND=metal|vulkan|dx12|gl"]
fn hardware_shader_chain_matches_independent_cpu_reference() {
    let backend: Backend = std::env::var("FVID_SHADER_BACKEND")
        .expect("explicit hardware backend required")
        .parse()
        .unwrap();
    assert!(matches!(
        backend,
        Backend::Metal | Backend::Vulkan | Backend::Dx12 | Backend::Gl
    ));
    for (width, height, format) in [(8, 6, "420"), (8, 6, "422"), (9, 7, "444"), (65, 17, "444")] {
        let header =
            Header::parse(format!("YUV4MPEG2 W{width} H{height} C{format}\n").as_bytes()).unwrap();
        let transform = Transform {
            horizontal: true,
            vertical: true,
            ..Default::default()
        };
        let stages = [
            GpuStage {
                transform,
                shader: Some(ByteShader::new(include_str!("../shaders/negate.wgsl")).unwrap()),
            },
            GpuStage {
                shader: Some(ByteShader::new(include_str!("../shaders/boxblur.wgsl")).unwrap()),
                ..Default::default()
            },
        ];
        let options = ExecutionOptions { backend, device: 0 };
        let mut gpu = GpuPipeline::with_stages(&header, &stages, options, 8 * 1024 * 1024)
            .expect("physical GPU required");
        assert!(
            GpuPipeline::with_stages(&header, &stages, options, gpu.controlled_memory_bytes() - 1)
                .is_err()
        );
        for frame in 0..3 {
            let input: Vec<u8> = (0..header.frame_len().unwrap())
                .map(|i| ((i * 37 + frame * 53) % 256) as u8)
                .collect();
            let mut flipped = vec![0; input.len()];
            Plan::new(&header, transform, usize::MAX)
                .unwrap()
                .apply(&input, &mut flipped)
                .unwrap();
            for value in &mut flipped[..width * height] {
                *value = 255 - *value;
            }
            let mut expected = flipped.clone();
            for y in 0..height {
                for x in 0..width {
                    let mut sum = 0u32;
                    for dy in -1isize..=1 {
                        for dx in -1isize..=1 {
                            let sx = (x as isize + dx).clamp(0, width as isize - 1) as usize;
                            let sy = (y as isize + dy).clamp(0, height as isize - 1) as usize;
                            sum += u32::from(flipped[sy * width + sx]);
                        }
                    }
                    expected[y * width + x] = (sum / 9) as u8;
                }
            }
            let resident = gpu.upload(&input).unwrap().process().unwrap();
            assert_eq!(resident.transfers().downloads, frame as u64);
            let mut output = vec![0; input.len()];
            let transfers = resident.download(&mut output).unwrap();
            assert_eq!(output, expected, "{format} frame {frame}");
            assert_eq!(transfers.uploads, frame as u64 + 1);
            assert_eq!(transfers.downloads, transfers.uploads);
            assert_eq!(transfers.filter_passes, transfers.uploads * 2);
        }
    }
}

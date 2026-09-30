use super::{GpuStage, TransferStats};
use crate::{Backend, Error, ExecutionOptions, Header, Plan, Result, Transform};

/// One device, one queue/stream, preallocated frames and immutable filter plans.
/// Consecutive transforms use coordinates relative to the preceding output.
/// A live frame prevents overwriting its storage:
/// ```compile_fail
/// use fvid::resident::GpuPipeline;
/// fn overwrite(pipeline: &mut GpuPipeline, input: &[u8]) {
///     let frame = pipeline.upload(input).unwrap().process().unwrap();
///     let _other = pipeline.upload(input);
///     let _ = frame.transfers();
/// }
/// ```
pub struct GpuPipeline {
    inner: Engine,
    input_len: usize,
    output_len: usize,
    output_header: Header,
}
enum Engine {
    #[cfg(feature = "gpu")]
    Wgpu(Box<crate::gpu::GpuProcessor>),
    #[cfg(feature = "cuda")]
    Cuda(Box<fvid_cuda::CudaPipeline>),
}
fn fail(message: &str) -> Error {
    Error::Gpu(message.into())
}

impl GpuPipeline {
    pub fn new(
        header: &Header,
        transforms: &[Transform],
        options: ExecutionOptions,
        memory_limit: usize,
    ) -> Result<Self> {
        let stages: Vec<_> = transforms.iter().copied().map(GpuStage::from).collect();
        Self::with_stages(header, &stages, options, memory_limit)
    }
    pub fn with_stages(
        header: &Header,
        stages: &[GpuStage],
        options: ExecutionOptions,
        memory_limit: usize,
    ) -> Result<Self> {
        if stages.is_empty() || stages.len() > 256 {
            return Err(fail("resident chain must contain 1..=256 transforms"));
        }
        if matches!(options.backend, Backend::Cpu | Backend::Auto) {
            return Err(fail("resident pipeline requires an explicit GPU backend"));
        }
        #[cfg(feature = "gpu")]
        if options.backend == Backend::Cuda && stages.iter().any(|stage| stage.shader.is_some()) {
            return Err(fail(
                "WGSL shaders require metal, vulkan, dx12 or gl; CUDA does not execute WGSL",
            ));
        }
        let input_len = header.frame_len()?;
        let mut current = header.clone();
        let mut plans = Vec::with_capacity(stages.len());
        for stage in stages {
            let plan = Plan::new(&current, stage.transform, usize::MAX)?;
            current.width = plan.width;
            current.height = plan.height;
            plans.push(plan);
        }
        let output_len = current.frame_len()?;
        let inner = match options.backend {
            Backend::Cuda => {
                #[cfg(feature = "cuda")]
                {
                    let params = plans
                        .iter()
                        .map(Plan::gpu_params)
                        .collect::<Result<Vec<_>>>()?;
                    Engine::Cuda(Box::new(
                        fvid_cuda::CudaPipeline::new(&params, options.device, memory_limit)
                            .map_err(Error::Gpu)?,
                    ))
                }
                #[cfg(not(feature = "cuda"))]
                return Err(fail("build with the cuda feature"));
            }
            _ => {
                #[cfg(feature = "gpu")]
                {
                    let plans: Vec<_> = plans.iter().collect();
                    Engine::Wgpu(Box::new(crate::gpu::GpuProcessor::new_shader_chain(
                        &plans,
                        &stages
                            .iter()
                            .map(|stage| stage.shader.as_ref())
                            .collect::<Vec<_>>(),
                        options.backend,
                        options.device,
                        memory_limit,
                    )?))
                }
                #[cfg(not(feature = "gpu"))]
                return Err(fail("build with the gpu feature"));
            }
        };
        #[allow(unreachable_code)]
        Ok(Self {
            inner,
            input_len,
            output_len,
            output_header: current,
        })
    }
    pub fn input_len(&self) -> usize {
        self.input_len
    }
    pub fn output_len(&self) -> usize {
        self.output_len
    }
    pub fn output_header(&self) -> &Header {
        &self.output_header
    }
    pub fn device_name(&self) -> &str {
        match &self.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => &p.name,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => p.device_name(),
        }
    }
    /// Includes resident textures/buffers, staging, uniforms and caller boundary buffers.
    /// Excludes driver/compiler overhead; this is not a strict RSS/VRAM bound.
    pub fn controlled_memory_bytes(&self) -> usize {
        match &self.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => p.controlled_memory,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => p.controlled_memory_bytes(),
        }
    }
    pub fn transfers(&self) -> TransferStats {
        match &self.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => p.transfers,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => {
                let s = p.transfers();
                TransferStats {
                    uploads: s.uploads,
                    downloads: s.downloads,
                    upload_bytes: s.upload_bytes,
                    download_bytes: s.download_bytes,
                    upload_staging_bytes: s.upload_bytes,
                    download_staging_bytes: s.download_bytes,
                    filter_passes: s.filter_passes,
                }
            }
        }
    }
    /// Upload once. The returned token prevents storage reuse until it is dropped.
    pub fn upload(&mut self, bytes: &[u8]) -> Result<UploadedFrame<'_>> {
        if bytes.len() != self.input_len {
            return Err(fail("resident input length mismatch"));
        }
        match &mut self.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => p.upload_frame(bytes)?,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => p.upload(bytes).map_err(Error::Gpu)?,
        }
        Ok(UploadedFrame { pipeline: self })
    }
}

pub struct UploadedFrame<'a> {
    pipeline: &'a mut GpuPipeline,
}
impl<'a> UploadedFrame<'a> {
    /// Submit every filter on the same GPU queue without intermediate readback.
    pub fn process(self) -> Result<ResidentFrame<'a>> {
        match &mut self.pipeline.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => p.dispatch_resident()?,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => p.process().map_err(Error::Gpu)?,
        }
        Ok(ResidentFrame {
            pipeline: self.pipeline,
        })
    }
}

/// Output stays on the device until an explicit download. GPU work may be pending.
/// Dropping this token discards the result without a host transfer; the pipeline
/// retains all allocations and its queue orders subsequent reuse safely.
pub struct ResidentFrame<'a> {
    pipeline: &'a mut GpuPipeline,
}
impl ResidentFrame<'_> {
    pub fn transfers(&self) -> TransferStats {
        self.pipeline.transfers()
    }
    pub fn header(&self) -> &Header {
        self.pipeline.output_header()
    }
    pub fn download(self, bytes: &mut [u8]) -> Result<TransferStats> {
        if bytes.len() != self.pipeline.output_len {
            return Err(fail("resident output length mismatch"));
        }
        match &mut self.pipeline.inner {
            #[cfg(feature = "gpu")]
            Engine::Wgpu(p) => p.download_frame(bytes)?,
            #[cfg(feature = "cuda")]
            Engine::Cuda(p) => p.download(bytes).map_err(Error::Gpu)?,
        }
        Ok(self.pipeline.transfers())
    }
}

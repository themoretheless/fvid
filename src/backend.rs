//! Backend selection is explicit; requested accelerators never silently become CPU.
use crate::{Error, Plan, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Cpu,
    Auto,
    Metal,
    Vulkan,
    Dx12,
    Gl,
    Cuda,
}
impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cpu => "cpu",
            Self::Auto => "auto",
            Self::Metal => "metal",
            Self::Vulkan => "vulkan",
            Self::Dx12 => "dx12",
            Self::Gl => "gl",
            Self::Cuda => "cuda",
        })
    }
}
impl std::str::FromStr for Backend {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "cpu" => Self::Cpu,
            "auto" => Self::Auto,
            "metal" => Self::Metal,
            "vulkan" => Self::Vulkan,
            "dx12" | "d3d12" | "directx" => Self::Dx12,
            "gl" | "opengl" | "gles" => Self::Gl,
            "cuda" => Self::Cuda,
            _ => {
                return Err(Error::Invalid(format!(
                    "unknown backend {s}; choose cpu, auto, metal, vulkan, dx12, gl, cuda"
                )));
            }
        })
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionOptions {
    pub backend: Backend,
    pub device: usize,
}
#[derive(Debug)]
pub struct DeviceInfo {
    pub backend: Backend,
    pub ordinal: usize,
    pub name: String,
    pub device_type: String,
}
#[derive(Debug)]
pub struct BackendDevices {
    pub backend: Backend,
    pub devices: Vec<DeviceInfo>,
    pub unavailable_reason: Option<String>,
}
pub fn devices() -> Vec<BackendDevices> {
    [
        Backend::Metal,
        Backend::Vulkan,
        Backend::Dx12,
        Backend::Gl,
        Backend::Cuda,
    ]
    .into_iter()
    .map(|backend| {
        let result = enumerate(backend);
        match result {
            Ok(devices) if !devices.is_empty() => BackendDevices {
                backend,
                devices,
                unavailable_reason: None,
            },
            Ok(_) => BackendDevices {
                backend,
                devices: Vec::new(),
                unavailable_reason: Some("no compatible hardware adapter detected".into()),
            },
            Err(e) => BackendDevices {
                backend,
                devices: Vec::new(),
                unavailable_reason: Some(e.to_string()),
            },
        }
    })
    .collect()
}
fn enumerate(backend: Backend) -> Result<Vec<DeviceInfo>> {
    if backend == Backend::Cuda {
        #[cfg(feature = "cuda")]
        return fvid_cuda::devices()
            .map(|v| {
                v.into_iter()
                    .map(|(ordinal, name)| DeviceInfo {
                        backend,
                        ordinal,
                        name,
                        device_type: "cuda".into(),
                    })
                    .collect()
            })
            .map_err(Error::Gpu);
        #[cfg(not(feature = "cuda"))]
        return Err(unavailable(backend, "build with --features cuda"));
    }
    #[cfg(feature = "gpu")]
    return crate::gpu::devices(backend);
    #[cfg(not(feature = "gpu"))]
    Err(unavailable(backend, "build with --features gpu"))
}
pub(crate) fn unavailable(backend: Backend, reason: &str) -> Error {
    Error::Gpu(format!("backend {backend} unavailable: {reason}"))
}

pub(crate) enum Processor {
    Cpu,
    #[cfg(feature = "gpu")]
    Gpu(Box<crate::gpu::GpuProcessor>),
    #[cfg(feature = "cuda")]
    Cuda(Box<fvid_cuda::CudaProcessor>),
}
impl Processor {
    pub fn new(
        plan: &Plan,
        options: ExecutionOptions,
        memory_limit: usize,
    ) -> Result<(Self, Backend, String, usize)> {
        if options.backend == Backend::Auto {
            if options.device != 0 {
                return Err(Error::Invalid(
                    "--device requires an explicit backend when not zero".into(),
                ));
            }
            let mut failures = Vec::new();
            for backend in [
                Backend::Metal,
                Backend::Vulkan,
                Backend::Dx12,
                Backend::Cuda,
                Backend::Gl,
            ] {
                match Self::new(plan, ExecutionOptions { backend, device: 0 }, memory_limit) {
                    Ok(found) => return Ok(found),
                    Err(e) => failures.push(e.to_string()),
                }
            }
            let (processor, backend, _, bytes) = Self::new(
                plan,
                ExecutionOptions {
                    backend: Backend::Cpu,
                    device: 0,
                },
                memory_limit,
            )?;
            return Ok((
                processor,
                backend,
                format!("CPU (auto fallback: {})", failures.join("; ")),
                bytes,
            ));
        }
        match options.backend {
            Backend::Cpu => {
                if options.device != 0 {
                    return Err(Error::Invalid("CPU backend only has device 0".into()));
                }
                let bytes = plan.input_len
                    + if plan.reuses_input() {
                        0
                    } else {
                        plan.output_len
                    };
                if bytes > memory_limit {
                    return Err(Error::Invalid(format!(
                        "CPU frame buffers need {bytes} bytes, exceeding memory limit {memory_limit}"
                    )));
                }
                Ok((Self::Cpu, Backend::Cpu, "CPU".into(), bytes))
            }
            Backend::Cuda => {
                #[cfg(feature = "cuda")]
                {
                    // Caller host + 2×(pinned+device) slots for depth-2 overlap + params.
                    let bytes = (plan.input_len + plan.output_len)
                        .checked_mul(5)
                        .and_then(|n| n.checked_add(128))
                        .ok_or_else(|| Error::Invalid("GPU memory size overflow".into()))?;
                    if bytes > memory_limit {
                        return Err(Error::Invalid(format!(
                            "CUDA frame buffers need {bytes} bytes, exceeding memory limit {memory_limit}"
                        )));
                    }
                    let processor = fvid_cuda::CudaProcessor::new(
                        plan.input_len,
                        plan.output_len,
                        plan.gpu_params()?,
                        options.device,
                    )
                    .map_err(Error::Gpu)?;
                    let name = processor.device_name().to_owned();
                    Ok((Self::Cuda(Box::new(processor)), Backend::Cuda, name, bytes))
                }
                #[cfg(not(feature = "cuda"))]
                Err(unavailable(Backend::Cuda, "build with --features cuda"))
            }
            backend => {
                #[cfg(feature = "gpu")]
                {
                    let processor =
                        crate::gpu::GpuProcessor::new(plan, backend, options.device, memory_limit)?;
                    let name = processor.name.clone();
                    let bytes = processor.controlled_memory;
                    Ok((Self::Gpu(Box::new(processor)), backend, name, bytes))
                }
                #[cfg(not(feature = "gpu"))]
                {
                    let _ = memory_limit;
                    Err(unavailable(backend, "build with --features gpu"))
                }
            }
        }
    }
    pub fn apply(&mut self, plan: &Plan, input: &[u8], output: &mut [u8]) -> Result<()> {
        match self {
            Self::Cpu => plan.apply(input, output),
            #[cfg(feature = "gpu")]
            Self::Gpu(gpu) => gpu.apply(input, output),
            #[cfg(feature = "cuda")]
            Self::Cuda(cuda) => cuda.apply(input, output).map_err(Error::Gpu),
        }
    }

    #[cfg(feature = "cuda")]
    pub fn cuda_submit(&mut self, input: &[u8], output: &mut [u8]) -> Result<bool> {
        match self {
            Self::Cuda(cuda) => cuda.submit(input, output).map_err(Error::Gpu),
            _ => Err(Error::Invalid(
                "cuda_submit requires the CUDA backend".into(),
            )),
        }
    }

    #[cfg(feature = "cuda")]
    pub fn cuda_flush(&mut self, output: &mut [u8]) -> Result<bool> {
        match self {
            Self::Cuda(cuda) => cuda.flush(output).map_err(Error::Gpu),
            _ => Err(Error::Invalid(
                "cuda_flush requires the CUDA backend".into(),
            )),
        }
    }
}

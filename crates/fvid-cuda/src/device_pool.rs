//! Process-wide CUDA context + loaded modules (one SharedDevice per ordinal).
//!
//! One-shot CLI still pays driver/context once per process. Long-lived callers
//! (MCP, repeated processors, resident pipelines) reuse the same context/modules.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cudarc::driver::{CudaContext, CudaFunction, CudaModule, CudaStream};

use crate::ptx_embed;

pub(crate) struct SharedDevice {
    pub context: Arc<CudaContext>,
    pub name: String,
    pub major: i32,
    pub minor: i32,
    /// Loaded on first Y4M/transform use (not at context create).
    transform: OnceLock<Result<Arc<CudaModule>, String>>,
    nv12: OnceLock<Result<Arc<CudaModule>, String>>,
}

impl SharedDevice {
    pub fn transform_kernel(&self) -> Result<CudaFunction, String> {
        let module = self.transform_module()?;
        module
            .load_function("fvid_transform")
            .map_err(|err| format!("CUDA transform function loading failed: {err}"))
    }

    pub fn nv12_kernel(&self) -> Result<CudaFunction, String> {
        let module = self.nv12_module()?;
        module
            .load_function("fvid_nv12_transform")
            .map_err(|err| format!("NV12 function loading failed: {err}"))
    }

    fn transform_module(&self) -> Result<Arc<CudaModule>, String> {
        self.transform
            .get_or_init(|| ptx_embed::load_transform(&self.context, self.major, self.minor))
            .clone()
    }

    fn nv12_module(&self) -> Result<Arc<CudaModule>, String> {
        self.nv12
            .get_or_init(|| ptx_embed::load_nv12(&self.context, self.major, self.minor))
            .clone()
    }

    pub fn new_stream(&self) -> Result<Arc<CudaStream>, String> {
        self.context
            .new_stream()
            .map_err(|err| format!("CUDA stream creation failed: {err}"))
    }
}

fn pool() -> &'static Mutex<HashMap<usize, Arc<SharedDevice>>> {
    static POOL: OnceLock<Mutex<HashMap<usize, Arc<SharedDevice>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn require_driver() -> Result<(), String> {
    // SAFETY: Loads only the CUDA driver's fixed standard library names through
    // cudarc. No caller-controlled library paths or pointers are accepted.
    if !unsafe { cudarc::driver::sys::is_culib_present() } {
        return Err("CUDA driver library was not found; install an NVIDIA driver and make libcuda.so.1 (Linux) or nvcuda.dll (Windows) available".into());
    }
    Ok(())
}

pub(crate) fn shared(ordinal: usize) -> Result<Arc<SharedDevice>, String> {
    require_driver()?;
    {
        let guard = pool()
            .lock()
            .map_err(|_| "CUDA device pool lock is poisoned".to_string())?;
        if let Some(existing) = guard.get(&ordinal) {
            return Ok(Arc::clone(existing));
        }
    }

    let count = CudaContext::device_count()
        .map_err(|err| format!("CUDA driver initialization failed: {err}"))?;
    if ordinal >= count as usize {
        return Err(format!(
            "CUDA device ordinal {ordinal} is unavailable ({count} devices found)"
        ));
    }
    let context = CudaContext::new(ordinal)
        .map_err(|err| format!("CUDA device {ordinal} initialization failed: {err}"))?;
    let name = context
        .name()
        .map_err(|err| format!("CUDA device name failed: {err}"))?;
    let (major, minor) = context
        .compute_capability()
        .map_err(|err| format!("CUDA compute capability lookup failed: {err}"))?;
    let device = Arc::new(SharedDevice {
        context,
        name,
        major,
        minor,
        transform: OnceLock::new(),
        nv12: OnceLock::new(),
    });

    let mut guard = pool()
        .lock()
        .map_err(|_| "CUDA device pool lock is poisoned".to_string())?;
    Ok(Arc::clone(guard.entry(ordinal).or_insert(device)))
}

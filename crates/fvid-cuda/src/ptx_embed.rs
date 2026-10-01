//! Checked-in multi-arch PTX (`crates/fvid-cuda/ptx/`), regenerated via
//! `cargo run --example emit_ptx` in this crate (NVRTC; no MSVC required).
//!
//! Runtime prefers the highest embedded compute capability ≤ the device, then
//! falls back to NVRTC from `.cu` source when nothing matches.

use std::ffi::CString;
use std::sync::Arc;

use cudarc::driver::result as cuda_result;
use cudarc::driver::sys as cuda;
use cudarc::driver::{CudaContext, CudaModule};
use cudarc::nvrtc::{CompileOptions, Ptx, compile_ptx_with_opts};

type Embedded = &'static [((u32, u32), &'static str)];

const TRANSFORM: Embedded = &[
    ((7, 5), include_str!("../ptx/transform_sm75.ptx")),
    ((8, 0), include_str!("../ptx/transform_sm80.ptx")),
    ((8, 6), include_str!("../ptx/transform_sm86.ptx")),
    ((8, 9), include_str!("../ptx/transform_sm89.ptx")),
    ((9, 0), include_str!("../ptx/transform_sm90.ptx")),
    ((10, 0), include_str!("../ptx/transform_sm100.ptx")),
    ((12, 0), include_str!("../ptx/transform_sm120.ptx")),
];

const NV12: Embedded = &[
    ((7, 5), include_str!("../ptx/nv12_sm75.ptx")),
    ((8, 0), include_str!("../ptx/nv12_sm80.ptx")),
    ((8, 6), include_str!("../ptx/nv12_sm86.ptx")),
    ((8, 9), include_str!("../ptx/nv12_sm89.ptx")),
    ((9, 0), include_str!("../ptx/nv12_sm90.ptx")),
    ((10, 0), include_str!("../ptx/nv12_sm100.ptx")),
    ((12, 0), include_str!("../ptx/nv12_sm120.ptx")),
];

fn pick(table: Embedded, major: i32, minor: i32) -> Option<&'static str> {
    let device = (major as u32, minor as u32);
    table
        .iter()
        .rev()
        .find(|(arch, _)| *arch <= device)
        .map(|(_, ptx)| *ptx)
}

fn load_module(context: &Arc<CudaContext>, ptx: Ptx) -> Result<Arc<CudaModule>, String> {
    context
        .load_module(ptx)
        .map_err(|err| format!("CUDA kernel loading failed: {err}"))
}

pub(crate) fn compile_source(
    source: &str,
    name: &str,
    major: i32,
    minor: i32,
) -> Result<Ptx, String> {
    // SAFETY: probes fixed NVRTC library names only.
    if !unsafe { cudarc::nvrtc::sys::is_culib_present() } {
        return Err(
            "CUDA NVRTC was not found and no precompiled PTX matched this GPU; install CUDA 13 NVRTC (nvrtc64_130_0.dll / libnvrtc.so.13) or regenerate crates/fvid-cuda/ptx"
                .into(),
        );
    }
    compile_ptx_with_opts(
        source,
        CompileOptions {
            options: vec![format!("--gpu-architecture=compute_{major}{minor}")],
            name: Some(name.into()),
            ..Default::default()
        },
    )
    .map_err(|err| {
        format!("CUDA kernel compilation failed; use an NVRTC version supporting this GPU: {err:?}")
    })
}

fn load(
    context: &Arc<CudaContext>,
    major: i32,
    minor: i32,
    table: Embedded,
    source: &str,
    compile_name: &str,
) -> Result<Arc<CudaModule>, String> {
    if let Some(src) = pick(table, major, minor) {
        match load_module(context, Ptx::from_src(src)) {
            Ok(module) => return Ok(module),
            Err(err) => {
                eprintln!("fvid-cuda: embedded PTX load failed ({err}); trying NVRTC");
            }
        }
    }
    let ptx = compile_source(source, compile_name, major, minor)?;
    load_module(context, ptx)
}

pub(crate) fn load_transform(
    context: &Arc<CudaContext>,
    major: i32,
    minor: i32,
) -> Result<Arc<CudaModule>, String> {
    load(
        context,
        major,
        minor,
        TRANSFORM,
        include_str!("transform.cu"),
        "fvid_transform.cu",
    )
}

/// Owns a driver module so `cu_function` stays valid until drop.
pub(crate) struct Nv12Module {
    context: Arc<CudaContext>,
    cu_module: cuda::CUmodule,
    pub cu_function: cuda::CUfunction,
}

// SAFETY: module/function handles are used with context bind; same as cudarc CudaModule.
unsafe impl Send for Nv12Module {}
unsafe impl Sync for Nv12Module {}

impl Drop for Nv12Module {
    fn drop(&mut self) {
        let _ = self.context.bind_to_thread();
        // SAFETY: uniquely owned module loaded in `load_nv12`.
        let _ = unsafe { cuda_result::module::unload(self.cu_module) };
    }
}

fn load_data_ptx(ptx: &Ptx) -> Result<cuda::CUmodule, String> {
    if let Some(bytes) = ptx.as_bytes() {
        // SAFETY: NVRTC image bytes; null-terminated PTX; context bound by caller.
        return unsafe { cuda_result::module::load_data(bytes.as_ptr() as *const _) }
            .map_err(|err| format!("CUDA NV12 module load failed: {err}"));
    }
    let src = ptx.to_src();
    let c_src = CString::new(src).map_err(|err| format!("PTX CString: {err}"))?;
    // SAFETY: null-terminated PTX text; context bound by caller.
    unsafe { cuda_result::module::load_data(c_src.as_ptr() as *const _) }
        .map_err(|err| format!("CUDA NV12 module load failed: {err}"))
}

pub(crate) fn load_nv12_shader(
    context: &Arc<CudaContext>,
    major: i32,
    minor: i32,
    shader: &crate::ByteShader,
) -> Result<Arc<Nv12Module>, String> {
    let source = shader.nv12_source()?;
    load_surface_source(context, major, minor, &source)
}

pub(crate) fn load_surface_source(
    context: &Arc<CudaContext>,
    major: i32,
    minor: i32,
    source: &str,
) -> Result<Arc<Nv12Module>, String> {
    let ptx = compile_source(source, "fvid_surface_shader.cu", major, minor)?;
    context.bind_to_thread().map_err(|e| e.to_string())?;
    let cu_module = load_data_ptx(&ptx)?;
    let name = CString::new("fvid_nv12_transform").expect("static name");
    // SAFETY: module was loaded on the bound context; static symbol names the generated kernel.
    let function = unsafe { cuda_result::module::get_function(cu_module, name) };
    match function {
        Ok(cu_function) => Ok(Arc::new(Nv12Module {
            context: Arc::clone(context),
            cu_module,
            cu_function,
        })),
        Err(error) => {
            // SAFETY: this unpublished module is uniquely owned after failed symbol lookup.
            let _ = unsafe { cuda_result::module::unload(cu_module) };
            Err(format!("NV12 shader function loading failed: {error}"))
        }
    }
}

pub(crate) fn load_nv12(
    context: &Arc<CudaContext>,
    major: i32,
    minor: i32,
) -> Result<Arc<Nv12Module>, String> {
    context
        .bind_to_thread()
        .map_err(|err| format!("CUDA bind failed: {err}"))?;
    let cu_module = if let Some(src) = pick(NV12, major, minor) {
        match load_data_ptx(&Ptx::from_src(src)) {
            Ok(m) => m,
            Err(err) => {
                eprintln!("fvid-cuda: embedded NV12 PTX load failed ({err}); trying NVRTC");
                let ptx = compile_source(include_str!("nv12.cu"), "fvid_nv12.cu", major, minor)?;
                load_data_ptx(&ptx)?
            }
        }
    } else {
        let ptx = compile_source(include_str!("nv12.cu"), "fvid_nv12.cu", major, minor)?;
        load_data_ptx(&ptx)?
    };
    let name = CString::new("fvid_nv12_transform").expect("static name");
    // SAFETY: module just loaded; name matches nv12.cu entry point.
    let cu_function = unsafe { cuda_result::module::get_function(cu_module, name) }
        .map_err(|err| format!("NV12 function loading failed: {err}"))?;
    Ok(Arc::new(Nv12Module {
        context: Arc::clone(context),
        cu_module,
        cu_function,
    }))
}

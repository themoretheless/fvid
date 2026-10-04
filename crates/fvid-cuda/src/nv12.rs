//! Device-resident NV12 crop / hflip / vflip (device-to-device; no host frame copies).
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct Nv12View {
    /// Device pointer to the Y plane (`CUdeviceptr` as `u64`).
    pub y: u64,
    /// Device pointer to the interleaved UV plane.
    pub uv: u64,
    pub pitch_y: u32,
    pub pitch_uv: u32,
    pub width: u32,
    pub height: u32,
}

/// Pitched P010 surface: byte pitches, 10-bit codes in bits 15..6 of each word.
#[derive(Clone, Copy, Debug)]
pub struct P010View {
    pub y: u64,
    pub uv: u64,
    pub pitch_y: u32,
    pub pitch_uv: u32,
    pub width: u32,
    pub height: u32,
}

/// Device-resident P010 crop/reflections and optional trusted component shader.
/// Shader inputs and sampler results are 10-bit codes; output keeps the low ten
/// bits and stores them MSB-aligned. Geometry uses pixels and pitches use bytes.
pub struct P010Processor {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    inner: native_nv12::Processor,
}

impl P010Processor {
    pub fn device_name(&self) -> &str {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.device_name()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            "CUDA unavailable"
        }
    }
    pub fn new(ordinal: usize) -> Result<Self, String> {
        Self::create(ordinal, None)
    }
    pub fn with_shader(ordinal: usize, shader: &crate::ByteShader) -> Result<Self, String> {
        Self::create(ordinal, Some(shader))
    }
    fn create(ordinal: usize, shader: Option<&crate::ByteShader>) -> Result<Self, String> {
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let source =
                shader.map_or_else(|| include_str!("p010.cu").to_owned(), |s| s.p010_source());
            native_nv12::Processor::with_source(ordinal, &source).map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = shader;
            Err(crate::unsupported())
        }
    }
    /// Borrow a stream in this device's primary context. It must remain alive
    /// until this processor is dropped; switching streams drains previous work.
    pub fn follow_stream(&mut self, stream: u64) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.inner.follow_stream(stream);
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let _ = stream;
    }
    /// Valid, disjoint surfaces on the selected device; even crop/output geometry.
    pub fn apply(&mut self, src: P010View, dst: P010View, t: Nv12Transform) -> Result<(), String> {
        validate_p010_views(src, dst, t)?;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let pixels = |v: P010View| Nv12View {
                y: v.y,
                uv: v.uv,
                pitch_y: v.pitch_y,
                pitch_uv: v.pitch_uv,
                width: v.width,
                height: v.height,
            };
            self.inner.apply(pixels(src), pixels(dst), t)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
}

fn validate_p010_views(src: P010View, dst: P010View, t: Nv12Transform) -> Result<(), String> {
    let as_bytes = |v: P010View| -> Result<Nv12View, String> {
        if (v.y | v.uv | u64::from(v.pitch_y) | u64::from(v.pitch_uv)) & 1 != 0 {
            return Err("P010 pointers and byte pitches must be word-aligned".into());
        }
        Ok(Nv12View {
            y: v.y,
            uv: v.uv,
            pitch_y: v.pitch_y,
            pitch_uv: v.pitch_uv,
            width: v.width.checked_mul(2).ok_or("P010 row size overflow")?,
            height: v.height,
        })
    };
    if t.crop_x % 2 != 0 || t.out_width % 2 != 0 {
        return Err("P010 crop and size must be even for 4:2:0".into());
    }
    let byte_t = Nv12Transform {
        crop_x: t.crop_x.checked_mul(2).ok_or("P010 crop overflow")?,
        out_width: t
            .out_width
            .checked_mul(2)
            .ok_or("P010 output row overflow")?,
        ..t
    };
    validate_views(as_bytes(src)?, as_bytes(dst)?, byte_t)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Nv12Transform {
    pub crop_x: u32,
    pub crop_y: u32,
    pub out_width: u32,
    pub out_height: u32,
    pub hflip: bool,
    pub vflip: bool,
}

/// Compiles the NV12 kernel once; each `apply` runs pitched DtoD memcpy (crop)
/// or a pitched in→out kernel (flips) on FFmpeg CUDA surfaces — no pack/unpack.
pub struct Nv12Processor {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    inner: native_nv12::Processor,
}

impl Nv12Processor {
    /// Compile a trusted CUDA C byte shader over native pitched NV12 surfaces.
    /// Luma/U/V use plane indices 0/1/2 and coordinates in each output plane.
    /// Sampling shaders read the current component through FvidSampler.
    pub fn with_shader(ordinal: usize, shader: &crate::ByteShader) -> Result<Self, String> {
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            native_nv12::Processor::with_shader(ordinal, shader).map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = shader;
            Err(crate::unsupported())
        }
    }
    pub fn new(ordinal: usize) -> Result<Self, String> {
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            native_nv12::Processor::new(ordinal).map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }

    pub fn device_name(&self) -> &str {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.device_name()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            "CUDA unavailable"
        }
    }

    /// Order filter work before consumers on a borrowed CUDA stream
    /// without a full device sync (same primary context).
    /// The borrowed stream must remain alive until this processor is dropped.
    /// Switching streams drains the previous stream before subsequent work.
    pub fn follow_stream(&mut self, stream: u64) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.follow_stream(stream);
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = stream;
        }
    }

    /// Wait for filter work, including a launch whose later ordering step failed.
    pub fn synchronize(&self) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.synchronize_launch()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }

    /// `src`/`dst` must be valid, nonoverlapping CUDA surfaces on this device.
    /// Crop and size must be even. No host full-frame copies are performed.
    pub fn apply(
        &mut self,
        src: Nv12View,
        dst: Nv12View,
        transform: Nv12Transform,
    ) -> Result<(), String> {
        validate_views(src, dst, transform)?;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.apply(src, dst, transform)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (src, dst, transform);
            Err(crate::unsupported())
        }
    }
}

/// Submit a crop-only NV12 device copy directly on an externally owned CUDA
/// stream. This avoids creating a second CUDA wrapper/context when FFmpeg
/// already owns the current primary context.
pub fn copy_crop_on_stream(
    src: Nv12View,
    dst: Nv12View,
    transform: Nv12Transform,
    context: u64,
    stream: u64,
) -> Result<(), String> {
    validate_views(src, dst, transform)?;
    if transform.hflip || transform.vflip {
        return Err("direct NV12 copy accepts crop-only transforms".into());
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        native_nv12::copy_crop_on_stream(src, dst, transform, context, stream)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (src, dst, transform, context, stream);
        Err(crate::unsupported())
    }
}

fn validate_views(src: Nv12View, dst: Nv12View, t: Nv12Transform) -> Result<(), String> {
    if t.out_width == 0 || t.out_height == 0 {
        return Err("NV12 output size must be non-zero".into());
    }
    if t.out_width != dst.width || t.out_height != dst.height {
        return Err("NV12 destination size must match transform output".into());
    }
    if t.crop_x % 2 != 0 || t.crop_y % 2 != 0 || t.out_width % 2 != 0 || t.out_height % 2 != 0 {
        return Err("NV12 crop and size must be even for 4:2:0".into());
    }
    if t.crop_x
        .checked_add(t.out_width)
        .is_none_or(|x| x > src.width)
        || t.crop_y
            .checked_add(t.out_height)
            .is_none_or(|y| y > src.height)
    {
        return Err("NV12 crop exceeds source frame".into());
    }
    if src.pitch_y < src.width
        || src.pitch_uv < src.width
        || dst.pitch_y < dst.width
        || dst.pitch_uv < dst.width
    {
        return Err("NV12 pitch is smaller than width".into());
    }
    if src.y == 0 || src.uv == 0 || dst.y == 0 || dst.uv == 0 {
        return Err("NV12 device pointers must be non-null".into());
    }
    // The shipped kernel and embedded PTX use 32-bit byte offsets. Validate
    // the full view, not only the crop, before either kernel or driver copy.
    let mut ranges = Vec::with_capacity(4);
    for view in [src, dst] {
        for (pointer, pitch, rows) in [
            (view.y, view.pitch_y, view.height),
            (view.uv, view.pitch_uv, view.height.div_ceil(2)),
        ] {
            let last = rows
                .checked_sub(1)
                .and_then(|row| row.checked_mul(pitch))
                .and_then(|offset| offset.checked_add(view.width.saturating_sub(1)))
                .ok_or("NV12 plane exceeds 32-bit kernel address range")?;
            let end = pointer
                .checked_add(u64::from(last))
                .ok_or("NV12 device pointer range overflow")?;
            ranges.push((pointer, end));
        }
    }
    // Inclusive footprints include row padding between the first/last pixels.
    // Device copies and shader neighborhood reads require disjoint planes.
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            if ranges[i].0 <= ranges[j].1 && ranges[j].0 <= ranges[i].1 {
                return Err("NV12 source and destination planes must not overlap".into());
            }
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native_nv12 {
    use super::*;
    use crate::device_pool::{self, SharedDevice};
    use cudarc::driver::sys::{self as cuda, CUDA_MEMCPY2D_v2, CUdeviceptr, CUmemorytype};
    use cudarc::driver::{CudaSlice, CudaStream, DevicePtr, PinnedHostSlice};

    pub(super) struct Processor {
        device: Arc<SharedDevice>,
        /// Loaded on first flip; crop-only never pays PTX. Keeps module alive.
        nv12: Option<Arc<crate::ptx_embed::Nv12Module>>,
        shader: bool,
        cu_function: Option<cuda::CUfunction>,
        /// Allocation stream (cudarc-managed).
        stream: Arc<CudaStream>,
        /// Last device stream used for work (for order_for_ffmpeg).
        launch: cuda::CUstream,
        /// Orders our launch stream ahead of `follow` (FFmpeg / NVENC stream).
        order_event: Option<cuda::CUevent>,
        follow: Option<cuda::CUstream>,
        name: String,
        poisoned: bool,
        params: Option<CudaSlice<u32>>,
        /// Device pointer for `params` (valid while `params` is live).
        params_dev: CUdeviceptr,
        /// Last uploaded transform params; skip HtoD when unchanged.
        cached_params: Option<[u32; 10]>,
        params_host: Option<PinnedHostSlice<u32>>,
    }

    fn memcpy2d(
        kind_src: CUmemorytype,
        kind_dst: CUmemorytype,
        src: CUdeviceptr,
        src_pitch: usize,
        dst: CUdeviceptr,
        dst_pitch: usize,
        width: usize,
        height: usize,
        stream: cuda::CUstream,
    ) -> Result<(), String> {
        let copy = CUDA_MEMCPY2D_v2 {
            srcXInBytes: 0,
            srcY: 0,
            srcMemoryType: kind_src,
            srcHost: std::ptr::null(),
            srcDevice: src,
            srcArray: std::ptr::null_mut(),
            srcPitch: src_pitch,
            dstXInBytes: 0,
            dstY: 0,
            dstMemoryType: kind_dst,
            dstHost: std::ptr::null_mut(),
            dstDevice: dst,
            dstArray: std::ptr::null_mut(),
            dstPitch: dst_pitch,
            WidthInBytes: width,
            Height: height,
        };
        let code = unsafe { cuda::cuMemcpy2DAsync_v2(&copy, stream) };
        if code != cuda::CUresult::CUDA_SUCCESS {
            return Err(format!("CUDA 2D copy failed: {code:?}"));
        }
        Ok(())
    }

    pub(super) fn copy_crop_on_stream(
        src: Nv12View,
        dst: Nv12View,
        t: Nv12Transform,
        context: u64,
        stream: u64,
    ) -> Result<(), String> {
        let push = unsafe { cuda::cuCtxPushCurrent_v2(context as cuda::CUcontext) };
        if push != cuda::CUresult::CUDA_SUCCESS {
            return Err(format!("CUDA context push failed: {push:?}"));
        }
        let stream = stream as cuda::CUstream;
        let src_y = (src.y as CUdeviceptr)
            .wrapping_add((t.crop_y as usize * src.pitch_y as usize + t.crop_x as usize) as u64);
        let src_uv = (src.uv as CUdeviceptr).wrapping_add(
            ((t.crop_y as usize / 2) * src.pitch_uv as usize + t.crop_x as usize) as u64,
        );
        let result = (|| {
            memcpy2d(
                CUmemorytype::CU_MEMORYTYPE_DEVICE,
                CUmemorytype::CU_MEMORYTYPE_DEVICE,
                src_y,
                src.pitch_y as usize,
                dst.y as CUdeviceptr,
                dst.pitch_y as usize,
                t.out_width as usize,
                t.out_height as usize,
                stream,
            )?;
            memcpy2d(
                CUmemorytype::CU_MEMORYTYPE_DEVICE,
                CUmemorytype::CU_MEMORYTYPE_DEVICE,
                src_uv,
                src.pitch_uv as usize,
                dst.uv as CUdeviceptr,
                dst.pitch_uv as usize,
                t.out_width as usize,
                (t.out_height / 2) as usize,
                stream,
            )
        })();
        let mut popped = std::ptr::null_mut();
        let pop = unsafe { cuda::cuCtxPopCurrent_v2(&mut popped) };
        if pop != cuda::CUresult::CUDA_SUCCESS {
            return Err(format!("CUDA context pop failed: {pop:?}"));
        }
        result
    }

    impl Processor {
        pub(super) fn with_source(ordinal: usize, source: &str) -> Result<Self, String> {
            let mut processor = Self::new(ordinal)?;
            let device = &processor.device;
            processor.nv12 = Some(crate::ptx_embed::load_surface_source(
                &device.context,
                device.major,
                device.minor,
                source,
            )?);
            processor.shader = true;
            Ok(processor)
        }
        pub(super) fn new(ordinal: usize) -> Result<Self, String> {
            let device = device_pool::shared(ordinal)?;
            let stream = device.context.default_stream();
            let launch = stream.cu_stream();
            let name = device.name.clone();
            Ok(Self {
                device,
                nv12: None,
                shader: false,
                cu_function: None,
                stream,
                launch,
                order_event: None,
                follow: None,
                name,
                poisoned: false,
                params: None,
                params_dev: 0,
                cached_params: None,
                params_host: None,
            })
        }

        pub(super) fn follow_stream(&mut self, stream: u64) {
            if self.follow != Some(stream as cuda::CUstream) && self.synchronize_launch().is_err() {
                self.poisoned = true;
                return;
            }
            // Record an event so the consumer stream waits for our work.
            self.follow = Some(stream as cuda::CUstream);
        }

        pub(super) fn synchronize_launch(&self) -> Result<(), String> {
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| format!("CUDA bind failed: {e}"))?;
            // SAFETY: launch is our owned stream or the caller's live followed stream.
            let code = unsafe { cuda::cuStreamSynchronize(self.launch) };
            if code != cuda::CUresult::CUDA_SUCCESS {
                return Err(format!(
                    "CUDA filter stream synchronization failed: {code:?}"
                ));
            }
            Ok(())
        }

        pub(super) fn with_shader(
            ordinal: usize,
            shader: &crate::ByteShader,
        ) -> Result<Self, String> {
            shader.nv12_source()?;
            let mut processor = Self::new(ordinal)?;
            let device = &processor.device;
            processor.nv12 = Some(crate::ptx_embed::load_nv12_shader(
                &device.context,
                device.major,
                device.minor,
                shader,
            )?);
            processor.shader = true;
            Ok(processor)
        }

        pub(super) fn device_name(&self) -> &str {
            &self.name
        }

        fn ensure_order_event(&mut self) -> Result<(), String> {
            if self.order_event.is_some() {
                return Ok(());
            }
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| format!("CUDA bind failed: {e}"))?;
            let mut event: cuda::CUevent = std::ptr::null_mut();
            let code = unsafe {
                cuda::cuEventCreate(
                    &mut event,
                    cuda::CUevent_flags::CU_EVENT_DISABLE_TIMING as u32,
                )
            };
            if code != cuda::CUresult::CUDA_SUCCESS {
                return Err(format!("CUDA event create failed: {code:?}"));
            }
            self.order_event = Some(event);
            Ok(())
        }

        fn ensure_flip_kernel(&mut self) -> Result<(), String> {
            if self.cu_function.is_some() {
                return Ok(());
            }
            let nv12 = match &self.nv12 {
                Some(module) => Arc::clone(module),
                None => self.device.nv12_module_arc()?,
            };
            let cu_function = nv12.cu_function;
            let params = self
                .stream
                .alloc_zeros::<u32>(10)
                .map_err(|err| format!("NV12 params allocation failed: {err}"))?;
            let params_dev = {
                let (ptr, sync) = params.device_ptr(&self.stream);
                drop(sync);
                ptr
            };
            // A followed stream must not race the allocation stream's zero-fill.
            self.stream
                .synchronize()
                .map_err(|e| format!("CUDA params initialization failed: {e}"))?;
            // SAFETY: all ten u32 values are initialized before their first upload.
            let mut host = unsafe { self.device.context.alloc_pinned::<u32>(10) }
                .map_err(|e| format!("CUDA pinned params allocation failed: {e}"))?;
            let pointer = host.as_mut_ptr().map_err(|e| e.to_string())?;
            // SAFETY: allocation contains ten writable u32 slots; initialize
            // them through a raw pointer before constructing a typed slice.
            unsafe {
                pointer.write_bytes(0, 10);
            }
            // Params live on the launch stream; no device-wide sync needed.
            self.cu_function = Some(cu_function);
            self.nv12 = Some(nv12);
            self.params_dev = params_dev;
            self.params = Some(params);
            self.params_host = Some(host);
            Ok(())
        }

        fn order_for_ffmpeg(&mut self) -> Result<(), String> {
            let Some(follow) = self.follow else {
                return Ok(());
            };
            if follow == self.launch {
                // Same stream: NVENC already sees prior work in-order.
                return Ok(());
            }
            self.ensure_order_event()?;
            let event = self.order_event.expect("order event");
            let code = unsafe { cuda::cuEventRecord(event, self.launch) };
            if code != cuda::CUresult::CUDA_SUCCESS {
                return Err(format!("CUDA event record failed: {code:?}"));
            }
            let code = unsafe { cuda::cuStreamWaitEvent(follow, event, 0) };
            if code != cuda::CUresult::CUDA_SUCCESS {
                return Err(format!("CUDA stream wait failed: {code:?}"));
            }
            Ok(())
        }

        pub(super) fn apply(
            &mut self,
            src: Nv12View,
            dst: Nv12View,
            t: Nv12Transform,
        ) -> Result<(), String> {
            if self.poisoned {
                return Err(
                    "NV12 processor failed previously; create a new processor before retrying"
                        .into(),
                );
            }
            let result = self
                .apply_inner(src, dst, t)
                .and_then(|_| self.order_for_ffmpeg());
            if result.is_err() {
                self.poisoned = true;
                let _ = self.synchronize_launch();
            }
            result
        }

        fn apply_inner(
            &mut self,
            src: Nv12View,
            dst: Nv12View,
            t: Nv12Transform,
        ) -> Result<(), String> {
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| format!("CUDA bind failed: {e}"))?;
            // Prefer FFmpeg's stream for all device work so NVENC sees it in-order.
            let stream = self.follow.unwrap_or_else(|| self.stream.cu_stream());
            self.launch = stream;

            // Crop-only (no flips): single DtoD per plane — no kernel / no PTX.
            if !t.hflip && !t.vflip && !self.shader {
                let src_y = (src.y as CUdeviceptr).wrapping_add(
                    (t.crop_y as usize * src.pitch_y as usize + t.crop_x as usize) as u64,
                );
                let src_uv = (src.uv as CUdeviceptr).wrapping_add(
                    ((t.crop_y as usize / 2) * src.pitch_uv as usize + t.crop_x as usize) as u64,
                );
                memcpy2d(
                    CUmemorytype::CU_MEMORYTYPE_DEVICE,
                    CUmemorytype::CU_MEMORYTYPE_DEVICE,
                    src_y,
                    src.pitch_y as usize,
                    dst.y as CUdeviceptr,
                    dst.pitch_y as usize,
                    t.out_width as usize,
                    t.out_height as usize,
                    stream,
                )?;
                memcpy2d(
                    CUmemorytype::CU_MEMORYTYPE_DEVICE,
                    CUmemorytype::CU_MEMORYTYPE_DEVICE,
                    src_uv,
                    src.pitch_uv as usize,
                    dst.uv as CUdeviceptr,
                    dst.pitch_uv as usize,
                    t.out_width as usize,
                    (t.out_height / 2) as usize,
                    stream,
                )?;
                return Ok(());
            }

            self.ensure_flip_kernel()?;
            let cu_f = self.cu_function.expect("cu_function after ensure");
            let params_dev = self.params_dev;

            let params = [
                src.pitch_y,
                src.pitch_uv,
                dst.pitch_y,
                dst.pitch_uv,
                t.crop_x,
                t.crop_y,
                t.out_width,
                t.out_height,
                u32::from(t.hflip),
                u32::from(t.vflip),
            ];
            use cudarc::driver::result as cuda_result;
            use std::ffi::c_void;
            if self.cached_params != Some(params) {
                let host = self
                    .params_host
                    .as_mut()
                    .expect("pinned params after ensure");
                host.as_mut_slice()
                    .map_err(|e| e.to_string())?
                    .copy_from_slice(&params);
                let pointer = host.as_ptr().map_err(|e| e.to_string())?;
                let bytes = params.len() * std::mem::size_of::<u32>();
                // SAFETY: pinned storage stays owned by self through completion;
                // params_dev is a live allocation and the byte length is exact.
                let code = unsafe {
                    cuda::cuMemcpyHtoDAsync_v2(params_dev, pointer as *const c_void, bytes, stream)
                };
                if code != cuda::CUresult::CUDA_SUCCESS {
                    return Err(format!("NV12 params upload failed: {code:?}"));
                }
                // Only changed geometry/pitches upload these 40 bytes. Finish
                // this stream's transfer before reusing the pinned host buffer;
                // unchanged-frame processing keeps its asynchronous kernel path.
                self.synchronize_launch()?;
                self.cached_params = Some(params);
            }

            self.launch = stream;
            let mut src_y = src.y;
            let mut src_uv = src.uv;
            let mut dst_y = dst.y;
            let mut dst_uv = dst.uv;
            let mut params_arg = params_dev;
            let mut args: [*mut c_void; 5] = [
                &mut src_y as *mut _ as *mut c_void,
                &mut src_uv as *mut _ as *mut c_void,
                &mut dst_y as *mut _ as *mut c_void,
                &mut dst_uv as *mut _ as *mut c_void,
                &mut params_arg as *mut _ as *mut c_void,
            ];
            // Reverse lane order still reads adjacent addresses within each warp,
            // so direct 2D loads remain coalesced without row-sized shared memory.
            let grid = (t.out_width.div_ceil(32), t.out_height.div_ceil(16), 1);
            let block = (32u32, 16u32, 1u32);
            // SAFETY: views validated; args match fvid_nv12_transform; stream is live.
            unsafe {
                cuda_result::launch_kernel(cu_f, grid, block, 0, stream, &mut args)
                    .map_err(|err| format!("NV12 transform launch failed: {err}"))?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod shader_tests {
        use super::*;
        #[test]
        #[ignore = "requires NVIDIA GPU, driver and CUDA 13 NVRTC"]
        fn followed_stream_survives_parameter_changes_and_processor_drop() {
            let shader =
                crate::ByteShader::new(include_str!("../../../shaders/negate.cu")).unwrap();
            let mut processor = Processor::with_shader(0, &shader).unwrap();
            let stream = processor.stream.clone();
            let external = processor.device.context.new_stream().unwrap();
            let y: Vec<u8> = (0..96).map(|i| (i * 17 % 251) as u8).collect();
            let uv: Vec<u8> = (0..48).map(|i| (i * 11 % 251) as u8).collect();
            let input_y = stream.clone_htod(&y).unwrap();
            let input_uv = stream.clone_htod(&uv).unwrap();
            let pointer = |buffer: &CudaSlice<u8>| {
                let (ptr, guard) = buffer.device_ptr(&stream);
                drop(guard);
                ptr
            };
            let source = Nv12View {
                y: pointer(&input_y),
                uv: pointer(&input_uv),
                pitch_y: 12,
                pitch_uv: 12,
                width: 8,
                height: 8,
            };
            let mut outputs = Vec::new();
            for index in 0..16 {
                outputs.push((
                    index,
                    stream.alloc_zeros::<u8>(24).unwrap(),
                    stream.alloc_zeros::<u8>(12).unwrap(),
                ));
            }
            stream.synchronize().unwrap();
            processor.follow_stream(external.cu_stream() as u64);
            for (index, output_y, output_uv) in &outputs {
                let destination = Nv12View {
                    y: pointer(output_y),
                    uv: pointer(output_uv),
                    pitch_y: 6,
                    pitch_uv: 6,
                    width: 4,
                    height: 4,
                };
                // First eight uploads change crop; last eight reuse cached params
                // and keep kernel work asynchronous until Drop drains the stream.
                let crop = if *index < 8 { (*index % 2) * 2 } else { 2 };
                processor
                    .apply(
                        source,
                        destination,
                        Nv12Transform {
                            crop_x: crop,
                            crop_y: 2,
                            out_width: 4,
                            out_height: 4,
                            hflip: true,
                            vflip: true,
                        },
                    )
                    .unwrap();
            }
            drop(processor);
            for (index, output_y, output_uv) in outputs {
                let crop = if index < 8 { (index % 2) * 2 } else { 2 } as usize;
                let mut actual_y = vec![0; 24];
                let mut actual_uv = vec![0; 12];
                stream.memcpy_dtoh(&output_y, &mut actual_y).unwrap();
                stream.memcpy_dtoh(&output_uv, &mut actual_uv).unwrap();
                stream.synchronize().unwrap();
                let mut expected_y = vec![0; 24];
                let mut expected_uv = vec![0; 12];
                for row in 0..4 {
                    for col in 0..4 {
                        expected_y[row * 6 + col] = 255 - y[(5 - row) * 12 + crop + 3 - col];
                    }
                }
                for row in 0..2 {
                    for col in 0..2 {
                        for component in 0..2 {
                            expected_uv[row * 6 + col * 2 + component] =
                                255 - uv[(2 - row) * 12 + crop + (1 - col) * 2 + component];
                        }
                    }
                }
                assert_eq!(actual_y, expected_y);
                assert_eq!(actual_uv, expected_uv);
            }
        }
        #[test]
        #[ignore = "requires NVIDIA GPU, driver and CUDA 13 NVRTC"]
        fn native_p010_sampling_shader_preserves_pitches_and_matches_cpu() {
            let shader =
                crate::ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu"))
                    .unwrap();
            let mut processor = Processor::with_source(0, &shader.p010_source()).unwrap();
            let stream = processor.stream.clone();
            let y: Vec<u16> = (0..96)
                .map(|i| ((((i * 17) % 1024) << 6) | 37) as u16)
                .collect();
            let uv: Vec<u16> = (0..48)
                .map(|i| ((((i * 11) % 1024) << 6) | 19) as u16)
                .collect();
            let input_y = stream.clone_htod(&y).unwrap();
            let input_uv = stream.clone_htod(&uv).unwrap();
            for hflip in [false, true] {
                for vflip in [false, true] {
                    let output_y = stream.alloc_zeros::<u16>(24).unwrap();
                    let output_uv = stream.alloc_zeros::<u16>(12).unwrap();
                    let pointer = |buffer: &CudaSlice<u16>| {
                        let (pointer, guard) = buffer.device_ptr(&stream);
                        drop(guard);
                        pointer
                    };
                    let src = Nv12View {
                        y: pointer(&input_y),
                        uv: pointer(&input_uv),
                        pitch_y: 24,
                        pitch_uv: 24,
                        width: 8,
                        height: 8,
                    };
                    let dst = Nv12View {
                        y: pointer(&output_y),
                        uv: pointer(&output_uv),
                        pitch_y: 12,
                        pitch_uv: 12,
                        width: 4,
                        height: 4,
                    };
                    let transform = Nv12Transform {
                        crop_x: 2,
                        crop_y: 2,
                        out_width: 4,
                        out_height: 4,
                        hflip,
                        vflip,
                    };
                    validate_views(src, dst, transform).unwrap();
                    processor.apply(src, dst, transform).unwrap();
                    let mut actual_y = vec![0; 24];
                    let mut actual_uv = vec![0; 12];
                    stream.memcpy_dtoh(&output_y, &mut actual_y).unwrap();
                    stream.memcpy_dtoh(&output_uv, &mut actual_uv).unwrap();
                    stream.synchronize().unwrap();
                    let mut cropped = [0u16; 16];
                    for row in 0..4 {
                        for col in 0..4 {
                            let sx = if hflip { 3 - col } else { col };
                            let sy = if vflip { 3 - row } else { row };
                            cropped[row * 4 + col] = y[(sy + 2) * 12 + sx + 2] >> 6;
                        }
                    }
                    let mut expected_y = vec![0; 24];
                    for row in 0..4i32 {
                        for col in 0..4i32 {
                            let mut total = 0u32;
                            for dy in -1..=1 {
                                for dx in -1..=1 {
                                    total += u32::from(
                                        cropped[(row + dy).clamp(0, 3) as usize * 4
                                            + (col + dx).clamp(0, 3) as usize],
                                    );
                                }
                            }
                            expected_y[row as usize * 6 + col as usize] = ((total / 9) << 6) as u16;
                        }
                    }
                    let mut expected_uv = vec![0; 12];
                    for row in 0..2 {
                        for col in 0..2 {
                            for component in 0..2 {
                                let sx = if hflip { 1 - col } else { col };
                                let sy = if vflip { 1 - row } else { row };
                                expected_uv[row * 6 + col * 2 + component] =
                                    uv[(sy + 1) * 12 + (sx + 1) * 2 + component] & !63;
                            }
                        }
                    }
                    assert_eq!(actual_y, expected_y, "hflip={hflip}, vflip={vflip}");
                    assert_eq!(actual_uv, expected_uv, "hflip={hflip}, vflip={vflip}");
                }
            }
        }

        #[test]
        #[ignore = "requires NVIDIA GPU, driver and CUDA 13 NVRTC"]
        fn native_nv12_sampling_shader_preserves_pitches_and_matches_cpu() {
            let shader =
                crate::ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu"))
                    .unwrap();
            let mut processor = Processor::with_shader(0, &shader).unwrap();
            let stream = processor.stream.clone();
            let y: Vec<u8> = (0..96).map(|i| ((i * 17) % 251) as u8).collect();
            let uv: Vec<u8> = (0..48).map(|i| ((i * 11) % 251) as u8).collect();
            let input_y = stream.clone_htod(&y).unwrap();
            let input_uv = stream.clone_htod(&uv).unwrap();
            for hflip in [false, true] {
                for vflip in [false, true] {
                    let output_y = stream.alloc_zeros::<u8>(24).unwrap();
                    let output_uv = stream.alloc_zeros::<u8>(12).unwrap();
                    let pointer = |buffer: &CudaSlice<u8>| {
                        let (pointer, guard) = buffer.device_ptr(&stream);
                        drop(guard);
                        pointer
                    };
                    let src = Nv12View {
                        y: pointer(&input_y),
                        uv: pointer(&input_uv),
                        pitch_y: 12,
                        pitch_uv: 12,
                        width: 8,
                        height: 8,
                    };
                    let dst = Nv12View {
                        y: pointer(&output_y),
                        uv: pointer(&output_uv),
                        pitch_y: 6,
                        pitch_uv: 6,
                        width: 4,
                        height: 4,
                    };
                    let transform = Nv12Transform {
                        crop_x: 2,
                        crop_y: 2,
                        out_width: 4,
                        out_height: 4,
                        hflip,
                        vflip,
                    };
                    validate_views(src, dst, transform).unwrap();
                    processor.apply(src, dst, transform).unwrap();
                    let mut actual_y = vec![0; 24];
                    let mut actual_uv = vec![0; 12];
                    stream.memcpy_dtoh(&output_y, &mut actual_y).unwrap();
                    stream.memcpy_dtoh(&output_uv, &mut actual_uv).unwrap();
                    stream.synchronize().unwrap();
                    let mut cropped = [0u8; 16];
                    for row in 0..4 {
                        for col in 0..4 {
                            let sx = if hflip { 3 - col } else { col };
                            let sy = if vflip { 3 - row } else { row };
                            cropped[row * 4 + col] = y[(sy + 2) * 12 + sx + 2];
                        }
                    }
                    let mut expected_y = vec![0; 24];
                    for row in 0..4i32 {
                        for col in 0..4i32 {
                            let mut total = 0u32;
                            for dy in -1..=1 {
                                for dx in -1..=1 {
                                    total += u32::from(
                                        cropped[(row + dy).clamp(0, 3) as usize * 4
                                            + (col + dx).clamp(0, 3) as usize],
                                    );
                                }
                            }
                            expected_y[row as usize * 6 + col as usize] = (total / 9) as u8;
                        }
                    }
                    let mut expected_uv = vec![0; 12];
                    for row in 0..2 {
                        for col in 0..2 {
                            for component in 0..2 {
                                let sx = if hflip { 1 - col } else { col };
                                let sy = if vflip { 1 - row } else { row };
                                expected_uv[row * 6 + col * 2 + component] =
                                    uv[(sy + 1) * 12 + (sx + 1) * 2 + component];
                            }
                        }
                    }
                    assert_eq!(actual_y, expected_y, "hflip={hflip}, vflip={vflip}");
                    assert_eq!(actual_uv, expected_uv, "hflip={hflip}, vflip={vflip}");
                }
            }
        }
    }

    impl Drop for Processor {
        fn drop(&mut self) {
            // Raw launches on an externally owned stream are not tracked by
            // CudaSlice's allocation stream. Drain before fields free params,
            // pinned host storage, and the compiled module.
            let _ = self.synchronize_launch();
            if let Some(event) = self.order_event.take() {
                let _ = self.device.context.bind_to_thread();
                unsafe {
                    let _ = cuda::cuEventDestroy_v2(event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn p010_admission_checks_byte_footprints_and_alignment() {
        let src = P010View {
            y: 4096,
            uv: 8192,
            pitch_y: 128,
            pitch_uv: 128,
            width: 64,
            height: 24,
        };
        let dst = P010View {
            y: 16384,
            uv: 24576,
            ..src
        };
        let t = Nv12Transform {
            out_width: 64,
            out_height: 24,
            ..Default::default()
        };
        assert!(validate_p010_views(src, dst, t).is_ok());
        for invalid in [
            P010View { y: 4097, ..src },
            P010View {
                pitch_uv: 127,
                ..src
            },
            P010View { pitch_y: 64, ..src },
            P010View {
                width: u32::MAX,
                ..src
            },
            P010View {
                y: u64::MAX - 1,
                ..src
            },
        ] {
            assert!(validate_p010_views(invalid, dst, t).is_err());
        }
        assert!(validate_p010_views(src, src, t).is_err());
        assert!(
            validate_p010_views(
                src,
                dst,
                Nv12Transform {
                    crop_x: 1,
                    out_width: 32,
                    ..t
                }
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_kernel_offset_and_device_address_overflow_before_launch() {
        let view = Nv12View {
            y: 4096,
            uv: 8192,
            pitch_y: 64,
            pitch_uv: 64,
            width: 64,
            height: 48,
        };
        let t = Nv12Transform {
            out_width: 64,
            out_height: 48,
            ..Default::default()
        };
        for invalid in [
            Nv12View {
                pitch_y: u32::MAX,
                ..view
            },
            Nv12View {
                pitch_uv: u32::MAX,
                ..view
            },
            Nv12View {
                y: u64::MAX,
                ..view
            },
            Nv12View {
                uv: u64::MAX,
                ..view
            },
        ] {
            assert!(validate_views(invalid, view, t).is_err());
            assert!(validate_views(view, invalid, t).is_err());
        }
        let destination = Nv12View {
            y: 16384,
            uv: 24576,
            ..view
        };
        assert!(validate_views(view, destination, t).is_ok());
        assert!(
            validate_views(view, view, t)
                .unwrap_err()
                .contains("overlap")
        );
        for (y, uv) in [
            (view.y + 1, 24576),
            (view.uv + 1, 24576),
            (16384, view.y),
            (16384, 16385),
        ] {
            assert!(
                validate_views(view, Nv12View { y, uv, ..view }, t)
                    .unwrap_err()
                    .contains("overlap")
            );
        }
    }

    #[test]
    fn rejects_odd_crop() {
        let src = Nv12View {
            y: 4096,
            uv: 8192,
            pitch_y: 64,
            pitch_uv: 64,
            width: 64,
            height: 48,
        };
        let dst = Nv12View {
            y: 16384,
            uv: 24576,
            pitch_y: 32,
            pitch_uv: 32,
            width: 32,
            height: 24,
        };
        let mut t = Nv12Transform {
            crop_x: 1,
            crop_y: 0,
            out_width: 32,
            out_height: 24,
            ..Default::default()
        };
        assert!(validate_views(src, dst, t).is_err());
        t.crop_x = 2;
        assert!(validate_views(src, dst, t).is_ok());
    }
}

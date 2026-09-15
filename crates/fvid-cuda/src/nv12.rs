//! Device-resident NV12 crop / hflip / vflip (device-to-device; no host frame copies).
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

    /// Bind filter work to FFmpeg's CUDA stream so NVENC sees ordered dependencies
    /// without a full device sync (same primary context).
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

    /// `src`/`dst` must be valid CUDA device surfaces on this device. Crop and size
    /// must be even. Host full-frame copies are not performed.
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

fn validate_views(src: Nv12View, dst: Nv12View, t: Nv12Transform) -> Result<(), String> {
    if t.out_width == 0 || t.out_height == 0 {
        return Err("NV12 output size must be non-zero".into());
    }
    if t.out_width != dst.width || t.out_height != dst.height {
        return Err("NV12 destination size must match transform output".into());
    }
    if t.crop_x % 2 != 0
        || t.crop_y % 2 != 0
        || t.out_width % 2 != 0
        || t.out_height % 2 != 0
    {
        return Err("NV12 crop and size must be even for 4:2:0".into());
    }
    if t.crop_x.checked_add(t.out_width).is_none_or(|x| x > src.width)
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
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native_nv12 {
    use super::*;
    use crate::device_pool::{self, SharedDevice};
    use cudarc::driver::sys::{self as cuda, CUdeviceptr, CUDA_MEMCPY2D_v2, CUmemorytype};
    use cudarc::driver::{CudaSlice, CudaStream, DevicePtr};

    pub(super) struct Processor {
        device: Arc<SharedDevice>,
        /// Loaded on first flip; crop-only never pays PTX. Keeps module alive.
        nv12: Option<Arc<crate::ptx_embed::Nv12Module>>,
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

    impl Processor {
        pub(super) fn new(ordinal: usize) -> Result<Self, String> {
            let device = device_pool::shared(ordinal)?;
            let stream = device.context.default_stream();
            let launch = stream.cu_stream();
            let name = device.name.clone();
            Ok(Self {
                device,
                nv12: None,
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
            })
        }

        pub(super) fn follow_stream(&mut self, stream: u64) {
            // NVENC submits on FFmpeg's stream; we record an event so it waits for our work.
            self.follow = Some(stream as cuda::CUstream);
        }

        pub(super) fn device_name(&self) -> &str {
            &self.name
        }

        fn ensure_order_event(&mut self) -> Result<(), String> {
            if self.order_event.is_some() {
                return Ok(());
            }
            let _ = self.device.context.bind_to_thread();
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
            let nv12 = self.device.nv12_module_arc()?;
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
            self.stream
                .synchronize()
                .map_err(|err| format!("NV12 params sync failed: {err}"))?;
            self.cu_function = Some(cu_function);
            self.nv12 = Some(nv12);
            self.params_dev = params_dev;
            self.params = Some(params);
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
            let result = self.apply_inner(src, dst, t).and_then(|_| self.order_for_ffmpeg());
            if result.is_err() {
                self.poisoned = true;
                let _ = self.stream.synchronize();
            }
            result
        }

        fn apply_inner(
            &mut self,
            src: Nv12View,
            dst: Nv12View,
            t: Nv12Transform,
        ) -> Result<(), String> {
            let _ = self.device.context.bind_to_thread();
            // Prefer FFmpeg's stream for all device work so NVENC sees it in-order.
            let stream = self.follow.unwrap_or_else(|| self.stream.cu_stream());
            self.launch = stream;

            // Crop-only (no flips): single DtoD per plane — no kernel / no PTX.
            if !t.hflip && !t.vflip {
                let src_y = (src.y as CUdeviceptr)
                    .wrapping_add((t.crop_y as usize * src.pitch_y as usize + t.crop_x as usize) as u64);
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
                let bytes = params.len() * std::mem::size_of::<u32>();
                let code = unsafe {
                    cuda::cuMemcpyHtoDAsync_v2(
                        params_dev,
                        params.as_ptr() as *const c_void,
                        bytes,
                        stream,
                    )
                };
                if code != cuda::CUresult::CUDA_SUCCESS {
                    return Err(format!("NV12 params upload failed: {code:?}"));
                }
                self.cached_params = Some(params);
            }

            // Same stream as crop memcpy / NVENC — no cross-stream event.
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

    impl Drop for Processor {
        fn drop(&mut self) {
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
    fn rejects_odd_crop() {
        let src = Nv12View {
            y: 1,
            uv: 2,
            pitch_y: 64,
            pitch_uv: 64,
            width: 64,
            height: 48,
        };
        let dst = Nv12View {
            y: 3,
            uv: 4,
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


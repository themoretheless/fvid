//! Owned CUDA NV12 output allocation for direct codec/filter pipelines.
use crate::Nv12View;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use cudarc::driver::DevicePtr;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;

pub struct Nv12Buffer {
    width: u32,
    height: u32,
    pitch: u32,
    bytes: usize,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    buffer: cudarc::driver::CudaSlice<u8>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    stream: Arc<cudarc::driver::CudaStream>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    device: Arc<crate::device_pool::SharedDevice>,
}
impl Nv12Buffer {
    pub fn new(ordinal: usize, width: u32, height: u32) -> Result<Self, String> {
        let (pitch, bytes) = layout(width, height)?;
        if ordinal > i32::MAX as usize {
            return Err("CUDA ordinal exceeds driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let device = crate::device_pool::shared(ordinal)?;
            let stream = device.new_stream()?;
            let buffer = stream.alloc_zeros::<u8>(bytes).map_err(|e| e.to_string())?;
            stream.synchronize().map_err(|e| e.to_string())?;
            Ok(Self {
                width,
                height,
                pitch,
                bytes,
                buffer,
                stream,
                device,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (pitch, bytes);
            Err(crate::unsupported())
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn pitch(&self) -> u32 {
        self.pitch
    }
    pub fn byte_len(&self) -> usize {
        self.bytes
    }
    /// Borrowed raw device view. It is not an allocation owner. Pointer use must
    /// finish before this buffer is dropped, including external codec references.
    pub fn view(&self) -> Result<Nv12View, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| e.to_string())?;
            let (pointer, _guard) = self.buffer.device_ptr(&self.stream);
            make_view(pointer, self.width, self.height, self.pitch)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
    /// Stream for ordering filters before the explicit synchronization boundary.
    pub fn stream_handle(&self) -> Result<u64, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            Ok(self.stream.cu_stream() as u64)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
    pub fn synchronize(&self) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.stream.synchronize().map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
}
fn layout(width: u32, height: u32) -> Result<(u32, usize), String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err("CUDA NV12 dimensions must be nonzero and even".into());
    }
    let pitch = width.checked_add(255).ok_or("CUDA NV12 pitch overflow")? & !255;
    let bytes = u64::from(pitch) * (u64::from(height) + u64::from(height) / 2);
    let bytes = usize::try_from(bytes).map_err(|_| "CUDA NV12 allocation extent overflow")?;
    if bytes > isize::MAX as usize {
        return Err("CUDA NV12 allocation exceeds pointer extent".into());
    }
    Ok((pitch, bytes))
}
fn make_view(pointer: u64, width: u32, height: u32, pitch: u32) -> Result<Nv12View, String> {
    let (_, bytes) = layout(width, height)?;
    let uv = pointer
        .checked_add(u64::from(pitch) * u64::from(height))
        .ok_or("CUDA NV12 UV pointer overflow")?;
    if pointer == 0 || pointer.checked_add(bytes as u64).is_none() {
        return Err("CUDA NV12 allocation has invalid device extent".into());
    }
    Ok(Nv12View {
        y: pointer,
        uv,
        pitch_y: pitch,
        pitch_uv: pitch,
        width,
        height,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_layout_covers_both_pitched_planes_and_rejects_overflow() {
        assert_eq!(layout(128, 72).unwrap(), (256, 27648));
        assert_eq!(layout(1920, 1080).unwrap(), (2048, 3317760));
        assert!(layout(0, 72).is_err());
        assert!(layout(127, 72).is_err());
        assert!(layout(u32::MAX - 1, 72).is_err());
        let view = make_view(4096, 128, 72, 256).unwrap();
        assert_eq!(view.uv, 4096 + 256 * 72);
        assert!(make_view(u64::MAX - 100, 128, 72, 256).is_err());
        assert!(make_view(0, 128, 72, 256).is_err());
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    #[test]
    fn unsupported_host_does_not_allocate_cuda_buffer() {
        assert!(Nv12Buffer::new(0, 128, 72).is_err());
    }
}

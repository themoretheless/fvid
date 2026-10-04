//! Owned CUDA NV12 output allocation for direct codec/filter pipelines.
use crate::{Nv12View, P010View};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use cudarc::driver::{DevicePtr, DevicePtrMut};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;

pub struct Nv12Buffer {
    width: u32,
    height: u32,
    pitch: u32,
    bytes: usize,
    sample_bytes: u32,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    buffer: cudarc::driver::CudaSlice<u8>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    stream: Arc<cudarc::driver::CudaStream>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    device: Arc<crate::device_pool::SharedDevice>,
}
impl Nv12Buffer {
    pub fn new(ordinal: usize, width: u32, height: u32) -> Result<Self, String> {
        Self::new_format(ordinal, width, height, 1)
    }
    fn new_format(
        ordinal: usize,
        width: u32,
        height: u32,
        sample_bytes: u32,
    ) -> Result<Self, String> {
        let (pitch, bytes) = layout_format(width, height, sample_bytes)?;
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
                sample_bytes,
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

    /// Fill both pitched planes on the device. Limited black uses Y=16;
    /// full-range black uses Y=0. Chroma is neutral 128 in both cases.
    /// Synchronizes before returning so direct codec registration sees completed
    /// writes. No host pixel allocation/upload is performed.
    pub fn fill_black(&mut self, full_range: bool) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| e.to_string())?;
            let (y_bytes, uv_bytes) = plane_bytes(self.pitch, self.height)?;
            let (pointer, _guard) = self.buffer.device_ptr_mut(&self.stream);
            let uv = pointer
                .checked_add(y_bytes as u64)
                .ok_or("CUDA black UV pointer overflow")?;
            // SAFETY: Owned u8 allocation has both checked plane extents;
            // writes use the allocation's own live stream. The mutable pointer
            // guard records completion before later cudarc users observe it.
            unsafe {
                if self.sample_bytes == 1 {
                    cudarc::driver::result::memset_d8_async(
                        pointer,
                        if full_range { 0 } else { 16 },
                        y_bytes,
                        self.stream.cu_stream(),
                    )
                    .map_err(|e| e.to_string())?;
                    cudarc::driver::result::memset_d8_async(
                        uv,
                        128,
                        uv_bytes,
                        self.stream.cu_stream(),
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    // cuMemsetD16Async counts words, not bytes. P010 stores each
                    // 10-bit code in bits 15..6: limited Y=64, neutral UV=512.
                    cudarc::driver::sys::cuMemsetD16Async(
                        pointer,
                        if full_range { 0 } else { 64 << 6 },
                        y_bytes / 2,
                        self.stream.cu_stream(),
                    )
                    .result()
                    .map_err(|e| e.to_string())?;
                    cudarc::driver::sys::cuMemsetD16Async(
                        uv,
                        512 << 6,
                        uv_bytes / 2,
                        self.stream.cu_stream(),
                    )
                    .result()
                    .map_err(|e| e.to_string())?;
                }
            }
            self.stream.synchronize().map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = full_range;
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
            make_view_format(
                pointer,
                self.width,
                self.height,
                self.pitch,
                self.sample_bytes,
            )
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
/// Owned pitched P010 allocation. Every code occupies a 16-bit word, with
/// its ten significant bits in bits 15..6; pitches and allocation lengths are bytes.
pub struct P010Buffer {
    inner: Nv12Buffer,
}
impl P010Buffer {
    pub fn new(ordinal: usize, width: u32, height: u32) -> Result<Self, String> {
        Nv12Buffer::new_format(ordinal, width, height, 2).map(|inner| Self { inner })
    }
    pub fn dimensions(&self) -> (u32, u32) {
        self.inner.dimensions()
    }
    pub fn pitch(&self) -> u32 {
        self.inner.pitch()
    }
    pub fn byte_len(&self) -> usize {
        self.inner.byte_len()
    }
    pub fn fill_black(&mut self, full_range: bool) -> Result<(), String> {
        self.inner.fill_black(full_range)
    }
    pub fn stream_handle(&self) -> Result<u64, String> {
        self.inner.stream_handle()
    }
    pub fn synchronize(&self) -> Result<(), String> {
        self.inner.synchronize()
    }
    /// Borrowed pointer view; complete external work before dropping the owner.
    pub fn view(&self) -> Result<P010View, String> {
        let v = self.inner.view()?;
        Ok(P010View {
            y: v.y,
            uv: v.uv,
            pitch_y: v.pitch_y,
            pitch_uv: v.pitch_uv,
            width: v.width,
            height: v.height,
        })
    }
}
fn plane_bytes(pitch: u32, height: u32) -> Result<(usize, usize), String> {
    if pitch == 0 || height == 0 || height % 2 != 0 {
        return Err("CUDA NV12 planes require nonzero pitch and even height".into());
    }
    let y = usize::try_from(u64::from(pitch) * u64::from(height))
        .map_err(|_| "CUDA Y extent overflow")?;
    let uv = y / 2;
    if y.checked_add(uv)
        .is_none_or(|bytes| bytes > isize::MAX as usize)
    {
        return Err("CUDA NV12 plane extent exceeds pointer range".into());
    }
    Ok((y, uv))
}
fn layout(width: u32, height: u32) -> Result<(u32, usize), String> {
    layout_format(width, height, 1)
}
fn layout_format(width: u32, height: u32, sample_bytes: u32) -> Result<(u32, usize), String> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err("CUDA NV12 dimensions must be nonzero and even".into());
    }
    if !matches!(sample_bytes, 1 | 2) {
        return Err("CUDA 4:2:0 sample size must be one or two bytes".into());
    }
    let pitch = width
        .checked_mul(sample_bytes)
        .and_then(|row| row.checked_add(255))
        .ok_or("CUDA 4:2:0 pitch overflow")?
        & !255;
    let (y, uv) = plane_bytes(pitch, height)?;
    let bytes = y
        .checked_add(uv)
        .ok_or("CUDA NV12 allocation extent overflow")?;
    if bytes > isize::MAX as usize {
        return Err("CUDA NV12 allocation exceeds pointer extent".into());
    }
    Ok((pitch, bytes))
}
fn make_view(pointer: u64, width: u32, height: u32, pitch: u32) -> Result<Nv12View, String> {
    make_view_format(pointer, width, height, pitch, 1)
}
fn make_view_format(
    pointer: u64,
    width: u32,
    height: u32,
    pitch: u32,
    sample_bytes: u32,
) -> Result<Nv12View, String> {
    layout_format(width, height, sample_bytes)?;
    let row = width
        .checked_mul(sample_bytes)
        .ok_or("CUDA 4:2:0 row overflow")?;
    if pitch < row || pitch % sample_bytes != 0 || pointer % u64::from(sample_bytes) != 0 {
        return Err("CUDA 4:2:0 pointer/pitch is too small or unaligned".into());
    }
    let (y_bytes, uv_bytes) = plane_bytes(pitch, height)?;
    let bytes = y_bytes
        .checked_add(uv_bytes)
        .ok_or("CUDA 4:2:0 plane overflow")?;
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
    fn p010_layout_uses_word_rows_and_checks_actual_pitched_extent() {
        assert_eq!(layout_format(130, 72, 2).unwrap(), (512, 55296));
        assert_eq!(layout_format(1920, 1080, 2).unwrap(), (3840, 6220800));
        assert!(layout_format(130, 72, 3).is_err());
        assert!(layout_format(u32::MAX - 1, 72, 2).is_err());
        let view = make_view_format(4096, 130, 72, 512, 2).unwrap();
        assert_eq!(view.uv, 4096 + 512 * 72);
        assert!(make_view_format(4097, 130, 72, 512, 2).is_err());
        assert!(make_view_format(4096, 130, 72, 256, 2).is_err());
        assert!(make_view_format(4096, 130, 72, 513, 2).is_err());
        assert!(make_view_format(u64::MAX - 50000, 130, 72, 512, 2).is_err());
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires NVIDIA CUDA"]
    fn device_p010_black_fill_has_exact_codes_and_word_padding() {
        let mut output = P010Buffer::new(0, 130, 72).unwrap();
        for full_range in [false, true] {
            output.fill_black(full_range).unwrap();
            let bytes = output
                .inner
                .stream
                .clone_dtoh(&output.inner.buffer)
                .unwrap();
            let (y, uv) = plane_bytes(output.pitch(), 72).unwrap();
            assert_eq!(bytes.len(), y + uv);
            let code = |word: &[u8]| u16::from_le_bytes([word[0], word[1]]);
            assert!(
                bytes[..y]
                    .chunks_exact(2)
                    .all(|word| code(word) == if full_range { 0 } else { 64 << 6 })
            );
            assert!(
                bytes[y..]
                    .chunks_exact(2)
                    .all(|word| code(word) == 512 << 6)
            );
            let view = output.view().unwrap();
            assert_eq!(view.pitch_y, 512);
            assert_eq!(view.uv - view.y, y as u64);
        }
    }

    #[test]
    fn black_plane_extents_cover_padding_without_overlapping_chroma() {
        assert_eq!(plane_bytes(256, 72).unwrap(), (18432, 9216));
        assert!(plane_bytes(0, 72).is_err());
        assert!(plane_bytes(256, 71).is_err());
        assert!(plane_bytes(u32::MAX, u32::MAX - 1).is_err());
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device"]
    fn device_black_fill_has_correct_luma_chroma_and_padding() {
        let mut output = Nv12Buffer::new(0, 128, 72).unwrap();
        for full_range in [false, true] {
            output.fill_black(full_range).unwrap();
            let bytes = output.stream.clone_dtoh(&output.buffer).unwrap();
            let (y, uv) = plane_bytes(output.pitch, output.height).unwrap();
            assert_eq!(bytes.len(), y + uv);
            assert!(
                bytes[..y]
                    .iter()
                    .all(|value| *value == if full_range { 0 } else { 16 })
            );
            assert!(bytes[y..].iter().all(|value| *value == 128));
        }
    }
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
        assert!(P010Buffer::new(0, 130, 72).is_err());
    }
}

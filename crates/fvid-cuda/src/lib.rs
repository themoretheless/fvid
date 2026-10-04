//! Optional CUDA processing of three byte planes (for example, planar YUV420p).
//!
//! This adapter owns device buffers, pinned host staging, and a stream. Each `apply`
//! copies into pinned memory, uploads, runs one fused crop/reflection kernel,
//! downloads into pinned memory, and synchronizes. A process-wide device pool
//! reuses context and modules. It does not provide codecs, texture interop, or
//! asynchronous host-buffer lifetimes.

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod device_pool;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod host_pinned;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native;
mod nvenc_sdk;
mod nvenc_session;
pub use nvenc_session::{CodecGuid, NvencCodec, NvencPacket, NvencSession, NvencSubmit};
mod nvdec;
/// Pinned NVIDIA picture-parameter data ABI; no linked SDK symbols.
pub mod nvdec_sdk;
pub use nvdec::{NvdecApi, NvdecCaps, NvdecChroma, NvdecCodec, NvdecSession, NvdecSurface};
mod nvenc;
pub use nvenc::{NvencApi, NvencVersion};
mod codec_device;
pub use codec_device::CodecDevice;
mod nv12;
mod nv12_buffer;
pub use nv12_buffer::Nv12Buffer;
mod pipeline;
mod shader;
pub use shader::ByteShader;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod ptx_embed;
pub use nv12::{
    Nv12Processor, Nv12Transform, Nv12View, P010Processor, P010View, copy_crop_on_stream,
};
pub use pipeline::{CudaPipeline, TransferStats};

/// A validated transform with buffers reused across frames of the same size.
pub struct CudaProcessor {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    inner: native::Processor,
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    _private: (),
}

impl CudaProcessor {
    /// Creates a processor on the CUDA device with the given enumeration ordinal.
    ///
    /// Each plane uses eight parameters: input offset, output offset, input row
    /// stride, crop x, crop y, output width, output height, reserved zero.
    /// Parameters 24/25 are horizontal/vertical reflection flags (0 or 1), 26/27
    /// are input/output byte lengths, and 28..32 must be zero. Output planes must
    /// be tightly packed and input planes must be ordered and nonoverlapping.
    pub fn new(
        input_len: usize,
        output_len: usize,
        params: [u32; 32],
        ordinal: usize,
    ) -> Result<Self, String> {
        validate(input_len, output_len, &params)?;
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            native::Processor::new(input_len, output_len, params, ordinal)
                .map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(unsupported())
        }
    }

    /// Returns only after output is readable on the CPU; transfer costs are included.
    pub fn apply(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.apply(input, output)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (input, output);
            Err(unsupported())
        }
    }

    /// Depth-2 pipelined submit. When `true`, `output` holds a completed earlier frame.
    pub fn submit(&mut self, input: &[u8], output: &mut [u8]) -> Result<bool, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.submit(input, output)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (input, output);
            Err(unsupported())
        }
    }

    /// Drain one in-flight frame into `output`. Returns `false` when the queue is empty.
    pub fn flush(&mut self, output: &mut [u8]) -> Result<bool, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.flush(output)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = output;
            Err(unsupported())
        }
    }

    /// The real CUDA device name returned by the driver.
    pub fn device_name(&self) -> &str {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.device_name()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            // Public construction always returns an error on this platform.
            "CUDA unavailable"
        }
    }
}

/// Enumerates CUDA driver ordinals and device names without requiring NVRTC.
/// An error distinguishes an unavailable driver from a driver with no devices.
pub fn devices() -> Result<Vec<(usize, String)>, String> {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        native::devices()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Err(unsupported())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn unsupported() -> String {
    "CUDA execution is supported only on Linux and Windows with an NVIDIA GPU; use Metal on macOS or choose another available backend".into()
}

fn validate(input_len: usize, output_len: usize, params: &[u32; 32]) -> Result<(), String> {
    if input_len == 0 || output_len == 0 {
        return Err("CUDA frames cannot be empty".into());
    }
    let input_len = u32::try_from(input_len)
        .map_err(|_| "CUDA input exceeds the 32-bit kernel address range")?;
    let output_len = u32::try_from(output_len)
        .map_err(|_| "CUDA output exceeds the 32-bit kernel address range")?;
    if params[26] != input_len || params[27] != output_len {
        return Err("CUDA parameter byte lengths differ from buffer lengths".into());
    }
    if params[24] > 1 || params[25] > 1 {
        return Err("CUDA reflection flags must be 0 or 1".into());
    }
    if [7, 15, 23, 28, 29, 30, 31]
        .iter()
        .any(|&index| params[index] != 0)
    {
        return Err("CUDA reserved parameters must be zero".into());
    }
    let mut expected_output = 0u64;
    for plane in 0..3 {
        let p = &params[plane * 8..plane * 8 + 8];
        let (input_offset, output_offset, stride, x, y, width, height) = (
            u64::from(p[0]),
            u64::from(p[1]),
            u64::from(p[2]),
            u64::from(p[3]),
            u64::from(p[4]),
            u64::from(p[5]),
            u64::from(p[6]),
        );
        if stride == 0 || width == 0 || height == 0 || x + width > stride {
            return Err(format!(
                "CUDA plane {plane} has invalid dimensions or crop width"
            ));
        }
        if output_offset != expected_output {
            return Err(format!("CUDA output plane {plane} is not contiguous"));
        }
        expected_output = expected_output
            .checked_add(width * height)
            .ok_or_else(|| format!("CUDA output plane {plane} overflows"))?;
        if expected_output > u64::from(output_len) {
            return Err(format!("CUDA output plane {plane} exceeds its buffer"));
        }
        let plane_end = if plane == 2 {
            u64::from(input_len)
        } else {
            u64::from(params[(plane + 1) * 8])
        };
        // checked arithmetic also rejects intentionally hostile u32 dimensions.
        let crop_end = (y + height - 1)
            .checked_mul(stride)
            .and_then(|offset| offset.checked_add(input_offset))
            .and_then(|offset| offset.checked_add(x + width))
            .ok_or_else(|| format!("CUDA input plane {plane} address overflows"))?;
        if input_offset >= plane_end || plane_end > u64::from(input_len) || crop_end > plane_end {
            return Err(format!(
                "CUDA crop for plane {plane} exceeds its input plane"
            ));
        }
    }
    if expected_output != u64::from(output_len) {
        return Err("CUDA output planes do not cover the full output buffer".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn params() -> [u32; 32] {
        let mut params = [0; 32];
        params[..8].copy_from_slice(&[0, 0, 4, 1, 1, 2, 2, 0]);
        params[8..16].copy_from_slice(&[16, 4, 2, 0, 0, 1, 1, 0]);
        params[16..24].copy_from_slice(&[20, 5, 2, 0, 0, 1, 1, 0]);
        params[26] = 24;
        params[27] = 6;
        params
    }

    #[test]
    fn accepts_bounded_crop_and_reflections() {
        for horizontal in 0..=1 {
            for vertical in 0..=1 {
                let mut p = params();
                p[24] = horizontal;
                p[25] = vertical;
                assert_eq!(validate(24, 6, &p), Ok(()));
            }
        }
    }

    #[test]
    fn rejects_invalid_flags_and_reserved_fields() {
        for index in [7, 15, 23, 24, 25, 28, 29, 30, 31] {
            let mut p = params();
            p[index] = 2;
            assert!(validate(24, 6, &p).is_err(), "parameter {index}");
        }
    }

    #[test]
    fn rejects_escaped_input_plane_and_address_overflow() {
        for (index, value) in [(2, 0), (3, 3), (4, 3), (4, u32::MAX), (8, 8), (16, 25)] {
            let mut p = params();
            p[index] = value;
            assert!(validate(24, 6, &p).is_err(), "parameter {index}={value}");
        }
        let mut p = params();
        p[2] = u32::MAX;
        p[4] = u32::MAX;
        p[6] = u32::MAX;
        assert!(validate(24, 6, &p).is_err());
    }

    #[test]
    fn rejects_overlapping_and_unwritten_output_bytes() {
        for (index, value) in [(1, 1), (5, 0), (6, 0), (9, 3), (9, 5), (17, 4), (21, 2)] {
            let mut p = params();
            p[index] = value;
            assert!(validate(24, 6, &p).is_err(), "parameter {index}={value}");
        }
        let mut p = params();
        p[27] = 7;
        assert!(validate(24, 7, &p).is_err());
    }

    #[test]
    fn rejects_mismatched_and_empty_buffers() {
        for (input, output) in [(0, 6), (24, 0), (23, 6), (24, 5)] {
            assert!(validate(input, output, &params()).is_err());
        }
        #[cfg(target_pointer_width = "64")]
        assert!(validate(u32::MAX as usize + 1, 6, &params()).is_err());
        assert!(CudaProcessor::new(24, 6, params(), usize::MAX).is_err());
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    #[test]
    fn unsupported_platform_returns_actionable_errors() {
        assert!(devices().unwrap_err().contains("Metal"));
        assert!(
            CudaProcessor::new(24, 6, params(), 0)
                .err()
                .unwrap()
                .contains("Metal")
        );
    }
}

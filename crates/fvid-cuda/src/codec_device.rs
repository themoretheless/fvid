//! CUDA context/stream ownership for direct codec driver integrations.
//! This does not yet create an NVDEC or NVENC session.
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;

pub struct CodecDevice {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    stream: Arc<cudarc::driver::CudaStream>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    device: Arc<crate::device_pool::SharedDevice>,
}
impl CodecDevice {
    pub fn new(ordinal: usize) -> Result<Self, String> {
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let device = crate::device_pool::shared(ordinal)?;
            let stream = device.new_stream()?;
            Ok(Self { stream, device })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
    /// Borrow native handles after binding the context to the calling thread.
    /// Handles must not be destroyed, transferred to another context, or used
    /// after this owner is dropped. Codec sessions must be closed first.
    pub fn handles(&self) -> Result<(u64, u64), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.device
                .context
                .bind_to_thread()
                .map_err(|e| e.to_string())?;
            Ok((
                self.device.context.cu_ctx() as u64,
                self.stream.cu_stream() as u64,
            ))
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
    /// Drain stream work before releasing codec-owned surfaces or a session.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_ordinal_is_rejected_before_driver_loading() {
        assert!(
            CodecDevice::new(usize::MAX)
                .err()
                .unwrap()
                .contains("ordinal")
        );
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device"]
    fn native_context_and_stream_are_owned_without_libav() {
        let device = CodecDevice::new(0).unwrap();
        let (context, stream) = device.handles().unwrap();
        assert_ne!(context, 0);
        assert_ne!(stream, 0);
        device.synchronize().unwrap();
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    #[test]
    fn unsupported_host_never_creates_a_codec_device() {
        assert!(CodecDevice::new(0).is_err());
    }
}

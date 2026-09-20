//! Page-locked host buffers for DMA without WRITECOMBINED (safe for CPU reads after DtoH).

use std::sync::Arc;

use cudarc::driver::result::{self, DriverError};
use cudarc::driver::{CudaContext, CudaStream, HostSlice, SyncOnDrop};

/// Pinned host memory (`cuMemHostAlloc` with flags 0).
pub(crate) struct HostPinned {
    ptr: *mut u8,
    len: usize,
    _ctx: Arc<CudaContext>,
}

// SAFETY: CUDA pinned host memory is accessible from any host thread; the owning
// SharedDevice/Processor serializes GPU use on one stream.
unsafe impl Send for HostPinned {}
unsafe impl Sync for HostPinned {}

impl HostPinned {
    pub(crate) fn alloc(context: &Arc<CudaContext>, len: usize) -> Result<Self, String> {
        context
            .bind_to_thread()
            .map_err(|err| format!("CUDA bind_to_thread failed: {err}"))?;
        // SAFETY: flags 0 = portable page-locked host memory; unset contents.
        let ptr = unsafe { result::malloc_host(len, 0) }.map_err(|err: DriverError| {
            format!("CUDA pinned host allocation ({len} bytes) failed: {err}")
        })? as *mut u8;
        if ptr.is_null() {
            return Err(format!(
                "CUDA pinned host allocation ({len} bytes) returned null"
            ));
        }
        Ok(Self {
            ptr,
            len,
            _ctx: Arc::clone(context),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr was allocated for `len` bytes and remains valid until Drop.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: exclusive &mut self; allocation sized to `len`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for HostPinned {
    fn drop(&mut self) {
        // SAFETY: ptr came from malloc_host and is freed exactly once.
        let _ = unsafe { result::free_host(self.ptr as *mut _) };
    }
}

impl HostSlice<u8> for HostPinned {
    fn len(&self) -> usize {
        self.len
    }

    unsafe fn stream_synced_slice<'a>(
        &'a self,
        _stream: &'a CudaStream,
    ) -> (&'a [u8], SyncOnDrop<'a>) {
        // SAFETY: ptr was allocated for `len` bytes and remains valid until Drop.
        (
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) },
            SyncOnDrop::Sync(None),
        )
    }

    unsafe fn stream_synced_mut_slice<'a>(
        &'a mut self,
        _stream: &'a CudaStream,
    ) -> (&'a mut [u8], SyncOnDrop<'a>) {
        // SAFETY: exclusive &mut self; allocation sized to `len`.
        (
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) },
            SyncOnDrop::Sync(None),
        )
    }
}

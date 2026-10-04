//! Direct NVENC 8.1 CUDA session ABI. No frame encoding is implemented here.
//! SDK layout: NVIDIA/video-sdk-samples@aa3544dcea2fe63122e4feb83bf805ea40e58dbe,
//! Samples/NvCodec/NvEncoder/nvEncodeAPI.h. This compatibility ABI predates AV1.
use crate::{CodecDevice, NvencApi, NvencVersion};
use std::{ffi::c_void, mem::ManuallyDrop};
const API: u32 = 8 | (1 << 24);
const fn version(revision: u32) -> u32 {
    API | (revision << 16) | (7 << 28)
}
type Destroy = unsafe extern "system" fn(*mut c_void) -> i32;
type Open = unsafe extern "system" fn(*mut OpenParams, *mut *mut c_void) -> i32;
#[repr(C)]
pub(crate) struct FunctionTable {
    version: u32,
    reserved: u32,
    before_destroy: [usize; 27],
    destroy: Option<Destroy>,
    invalidate: usize,
    open: Option<Open>,
    tail: [usize; 7],
    reserved2: [usize; 281],
}
impl FunctionTable {
    pub(crate) fn new() -> Self {
        Self {
            version: version(2),
            reserved: 0,
            before_destroy: [0; 27],
            destroy: None,
            invalidate: 0,
            open: None,
            tail: [0; 7],
            reserved2: [0; 281],
        }
    }
}
#[repr(C)]
struct OpenParams {
    version: u32,
    device_type: u32,
    device: *mut c_void,
    reserved: *mut c_void,
    api: u32,
    reserved1: [u32; 253],
    reserved2: [usize; 64],
}
/// Owns the encoder, its CUDA resources and the loaded driver library.
/// An open session is not an initialized encoder or a verified hardware path.
pub struct NvencSession {
    encoder: *mut c_void,
    destroy: Destroy,
    device: ManuallyDrop<CodecDevice>,
    _api: ManuallyDrop<NvencApi>,
}
impl NvencSession {
    pub fn open(device: CodecDevice) -> Result<Self, String> {
        let api = NvencApi::load()?;
        api.require(NvencVersion { major: 8, minor: 1 })?;
        let table = api.session_table()?;
        let open = table
            .open
            .ok_or("NVENC driver omitted session-open entrypoint")?;
        let destroy = table
            .destroy
            .ok_or("NVENC driver omitted session-destroy entrypoint")?;
        let (context, _) = device.handles()?;
        let mut params = OpenParams {
            version: version(1),
            device_type: 1,
            device: context as *mut c_void,
            reserved: std::ptr::null_mut(),
            api: API,
            reserved1: [0; 253],
            reserved2: [0; 64],
        };
        let mut encoder = std::ptr::null_mut();
        // SAFETY: SDK-verified parameter/table ABI; the context is live and
        // bound to this thread. The API and device stay owned by the session.
        let status = unsafe { open(&mut params, &mut encoder) };
        if status != 0 {
            return Err(format!("NVENC session open failed with status {status}"));
        }
        if encoder.is_null() {
            return Err("NVENC opened a null encoder handle".into());
        }
        Ok(Self {
            encoder,
            destroy,
            device: ManuallyDrop::new(device),
            _api: ManuallyDrop::new(api),
        })
    }
    /// Close explicitly to observe driver errors; on failure the handle remains
    /// owned and close can be retried. Drop makes one final best-effort attempt.
    pub fn close(&mut self) -> Result<(), String> {
        if self.encoder.is_null() {
            return Ok(());
        }
        self.device.handles()?;
        self.device.synchronize()?;
        // SAFETY: The live encoder belongs to this session; the driver library
        // remains loaded and the owning CUDA context is bound above.
        close_handle(&mut self.encoder, |encoder| unsafe {
            (self.destroy)(encoder)
        })
    }
}
impl Drop for NvencSession {
    fn drop(&mut self) {
        if self.close().is_ok() {
            // SAFETY: These uniquely owned fields were initialized once and
            // never manually dropped; the encoder is closed before its owners.
            unsafe {
                ManuallyDrop::drop(&mut self.device);
                ManuallyDrop::drop(&mut self._api);
            }
        }
        // On driver failure, retain resources rather than unload a library or
        // context while the driver still owns the live encoder. Process exit
        // reclaims them; explicit close lets callers observe and retry errors.
    }
}
fn close_handle(
    encoder: &mut *mut c_void,
    destroy: impl FnOnce(*mut c_void) -> i32,
) -> Result<(), String> {
    if encoder.is_null() {
        return Ok(());
    }
    let status = destroy(*encoder);
    if status != 0 {
        return Err(format!("NVENC session close failed with status {status}"));
    }
    *encoder = std::ptr::null_mut();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVENC"]
    fn direct_cuda_session_opens_and_closes_without_libav() {
        let device = CodecDevice::new(0).unwrap();
        let mut session = NvencSession::open(device).unwrap();
        session.close().unwrap();
        session.close().unwrap();
    }
    #[test]
    fn close_error_keeps_handle_and_success_is_idempotent() {
        let original = std::ptr::dangling_mut::<c_void>();
        let mut encoder = original;
        assert!(close_handle(&mut encoder, |_| 20).is_err());
        assert_eq!(encoder, original);
        close_handle(&mut encoder, |handle| {
            assert_eq!(handle, original);
            0
        })
        .unwrap();
        assert!(encoder.is_null());
        close_handle(&mut encoder, |_| panic!("closed encoder called twice")).unwrap();
    }
    #[test]
    fn sdk_64_bit_session_abi_has_verified_sizes_and_offsets() {
        if std::mem::size_of::<usize>() != 8 {
            return;
        }
        assert_eq!(std::mem::size_of::<FunctionTable>(), 2552);
        assert_eq!(std::mem::offset_of!(FunctionTable, destroy), 224);
        assert_eq!(std::mem::offset_of!(FunctionTable, open), 240);
        assert_eq!(std::mem::size_of::<OpenParams>(), 1552);
        assert_eq!(std::mem::offset_of!(OpenParams, api), 24);
        assert_eq!(std::mem::offset_of!(OpenParams, reserved2), 1040);
        assert_eq!(version(2), 0x71020008);
    }
}

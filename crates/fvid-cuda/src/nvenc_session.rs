//! Direct NVENC 8.1 CUDA session ABI. No frame encoding is implemented here.
//! SDK layout: NVIDIA/video-sdk-samples@aa3544dcea2fe63122e4feb83bf805ea40e58dbe,
//! Samples/NvCodec/NvEncoder/nvEncodeAPI.h. This compatibility ABI predates AV1.
use crate::{CodecDevice, NvencApi, NvencVersion};
use std::{ffi::c_void, mem::ManuallyDrop};
const API: u32 = 8 | (1 << 24);
const fn version(revision: u32) -> u32 {
    API | (revision << 16) | (7 << 28)
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodecGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}
type GuidCount = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type Guids = unsafe extern "system" fn(*mut c_void, *mut CodecGuid, u32, *mut u32) -> i32;
type Preset = unsafe extern "system" fn(
    *mut c_void,
    crate::nvenc_sdk::GUID,
    crate::nvenc_sdk::GUID,
    *mut crate::nvenc_sdk::NV_ENC_PRESET_CONFIG,
) -> u32;
type Initialize =
    unsafe extern "system" fn(*mut c_void, *mut crate::nvenc_sdk::NV_ENC_INITIALIZE_PARAMS) -> u32;
type Destroy = unsafe extern "system" fn(*mut c_void) -> i32;
type Open = unsafe extern "system" fn(*mut OpenParams, *mut *mut c_void) -> i32;
#[repr(C)]
pub(crate) struct FunctionTable {
    version: u32,
    reserved: u32,
    legacy_open: usize,
    guid_count: Option<GuidCount>,
    profiles: [usize; 2],
    guids: Option<Guids>,
    before_preset: [usize; 5],
    preset: Option<Preset>,
    initialize: Option<Initialize>,
    input_buffers: [usize; 2],
    create_output: crate::nvenc_sdk::PNVENCCREATEBITSTREAMBUFFER,
    destroy_output: crate::nvenc_sdk::PNVENCDESTROYBITSTREAMBUFFER,
    before_destroy: [usize; 11],
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
            legacy_open: 0,
            guid_count: None,
            profiles: [0; 2],
            guids: None,
            before_preset: [0; 5],
            preset: None,
            initialize: None,
            input_buffers: [0; 2],
            create_output: None,
            destroy_output: None,
            before_destroy: [0; 11],
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
    table: FunctionTable,
    outputs: Vec<*mut c_void>,
    initialized: bool,
    failed: bool,
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
            table,
            outputs: Vec::new(),
            initialized: false,
            failed: false,
            device: ManuallyDrop::new(device),
            _api: ManuallyDrop::new(api),
        })
    }
    /// Initialize synchronous H.264 encoding from the driver's default preset.
    /// Configuration and frame submission are separate; this emits no packets.
    pub fn initialize_h264(
        &mut self,
        width: u32,
        height: u32,
        fps_num: u32,
        fps_den: u32,
    ) -> Result<(), String> {
        validate_geometry(width, height, fps_num, fps_den)?;
        if self.encoder.is_null() || self.initialized || self.failed {
            return Err("NVENC initialization requires a fresh live session".into());
        }
        self.device.handles()?;
        let preset_fn = self.table.preset.ok_or("NVENC omitted preset entrypoint")?;
        let initialize = self
            .table
            .initialize
            .ok_or("NVENC omitted initialization entrypoint")?;
        let codec = crate::nvenc_sdk::GUID {
            Data1: 0x6bc82762,
            Data2: 0x4e63,
            Data3: 0x4ca4,
            Data4: [0xaa, 0x85, 0x1e, 0x50, 0xf3, 0x21, 0xf6, 0xbf],
        };
        let preset_guid = crate::nvenc_sdk::GUID {
            Data1: 0xb2dfb705,
            Data2: 0x4ebd,
            Data3: 0x4c49,
            Data4: [0x9b, 0x5f, 0x24, 0xa7, 0x77, 0xd3, 0xe5, 0x87],
        };
        let mut preset = crate::nvenc_sdk::NV_ENC_PRESET_CONFIG::default();
        preset.version = version(4) | (1 << 31);
        preset.presetCfg.version = version(7) | (1 << 31);
        // SAFETY: The SDK-generated preset storage and GUIDs have the verified
        // ABI; the session, context and driver remain live for this call.
        let status = unsafe { preset_fn(self.encoder, codec, preset_guid, &mut preset) };
        if status != 0 {
            return Err(format!("NVENC preset query failed with status {status}"));
        }
        let mut params = crate::nvenc_sdk::NV_ENC_INITIALIZE_PARAMS::default();
        params.version = version(5) | (1 << 31);
        params.encodeGUID = codec;
        params.presetGUID = preset_guid;
        params.encodeWidth = width;
        params.encodeHeight = height;
        params.darWidth = width;
        params.darHeight = height;
        params.frameRateNum = fps_num;
        params.frameRateDen = fps_den;
        params.enablePTD = 1;
        params.encodeConfig = &mut preset.presetCfg;
        params.maxEncodeWidth = width;
        params.maxEncodeHeight = height;
        // SAFETY: params and its referenced config live throughout the call;
        // zeroed reserved fields and version constants match the pinned SDK.
        let status = unsafe { initialize(self.encoder, &mut params) };
        if status != 0 {
            self.failed = true;
            return Err(format!(
                "NVENC encoder initialization failed with status {status}"
            ));
        }
        self.initialized = true;
        Ok(())
    }
    /// Allocate a driver output slot after initialization. Slots remain owned
    /// by this session and are released before the encoder is destroyed.
    pub fn create_output(&mut self) -> Result<usize, String> {
        if self.encoder.is_null() || !self.initialized || self.failed {
            return Err("NVENC output allocation requires a healthy initialized session".into());
        }
        self.device.handles()?;
        let create = self
            .table
            .create_output
            .ok_or("NVENC omitted output-create entrypoint")?;
        self.table
            .destroy_output
            .ok_or("NVENC omitted output-destroy entrypoint")?;
        self.outputs
            .try_reserve(1)
            .map_err(|e| format!("NVENC output ownership allocation failed: {e}"))?;
        let mut params = crate::nvenc_sdk::NV_ENC_CREATE_BITSTREAM_BUFFER::default();
        params.version = version(1);
        // SAFETY: Initialized session and generated SDK storage are live; size,
        // deprecated heap and reserved fields remain zero as required by SDK.
        let status = unsafe { create(self.encoder, &mut params) };
        if status != 0 {
            return Err(format!(
                "NVENC output allocation failed with status {status}"
            ));
        }
        if params.bitstreamBuffer.is_null() {
            self.failed = true;
            return Err("NVENC allocated a null output handle".into());
        }
        let index = self.outputs.len();
        self.outputs.push(params.bitstreamBuffer);
        Ok(index)
    }
    /// Query all codec identifiers advertised by this live encoder session.
    /// Unknown identifiers are retained rather than silently filtered.
    pub fn codec_guids(&self) -> Result<Vec<CodecGuid>, String> {
        if self.encoder.is_null() {
            return Err("NVENC session is closed".into());
        }
        self.device.handles()?;
        let count = self
            .table
            .guid_count
            .ok_or("NVENC omitted codec-count entrypoint")?;
        let guids = self
            .table
            .guids
            .ok_or("NVENC omitted codec-list entrypoint")?;
        let mut capacity = 0;
        // SAFETY: The session and library are live; capacity is writable SDK
        // uint32_t storage, and this entrypoint uses the verified NVENCAPI ABI.
        let status = unsafe { count(self.encoder, &mut capacity) };
        if status != 0 {
            return Err(format!(
                "NVENC codec-count query failed with status {status}"
            ));
        }
        read_guids(capacity, |output, written| {
            // SAFETY: output has the exact advertised capacity of initialized
            // GUID storage; the SDK requires writes to stay within that count.
            unsafe { guids(self.encoder, output.as_mut_ptr(), capacity, written) }
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
        if !self.outputs.is_empty() {
            let destroy = self
                .table
                .destroy_output
                .ok_or("NVENC omitted output-destroy entrypoint")?;
            for output in &mut self.outputs {
                // SAFETY: Each nonnull output belongs to this live encoder;
                // successful destruction clears it, preventing a repeated free.
                close_handle(output, |handle| unsafe {
                    destroy(self.encoder, handle) as i32
                })?;
            }
        }
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
fn validate_geometry(width: u32, height: u32, fps_num: u32, fps_den: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width % 2 != 0
        || height % 2 != 0
        || fps_num == 0
        || fps_den == 0
    {
        return Err("NVENC requires positive even geometry and a nonzero frame rate ratio".into());
    }
    Ok(())
}
fn read_guids(
    capacity: u32,
    query: impl FnOnce(&mut [CodecGuid], &mut u32) -> i32,
) -> Result<Vec<CodecGuid>, String> {
    if capacity == 0 {
        return Ok(Vec::new());
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity as usize)
        .map_err(|e| format!("NVENC codec-list allocation failed: {e}"))?;
    values.resize(capacity as usize, CodecGuid::default());
    let mut written = 0;
    let status = query(&mut values, &mut written);
    if status != 0 {
        return Err(format!(
            "NVENC codec-list query failed with status {status}"
        ));
    }
    if written > capacity {
        return Err("NVENC codec-list count exceeds supplied capacity".into());
    }
    values.truncate(written as usize);
    Ok(values)
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
    #[test]
    fn output_creation_abi_matches_sdk_and_partial_cleanup_is_retryable() {
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(
                std::mem::size_of::<crate::nvenc_sdk::NV_ENC_CREATE_BITSTREAM_BUFFER>(),
                776
            );
            assert_eq!(
                std::mem::offset_of!(
                    crate::nvenc_sdk::NV_ENC_CREATE_BITSTREAM_BUFFER,
                    bitstreamBuffer
                ),
                16
            );
            assert_eq!(std::mem::offset_of!(FunctionTable, create_output), 120);
            assert_eq!(std::mem::offset_of!(FunctionTable, destroy_output), 128);
        }
        let handle = std::ptr::dangling_mut::<c_void>();
        let mut outputs = [handle, handle];
        close_handle(&mut outputs[0], |_| 0).unwrap();
        assert!(close_handle(&mut outputs[1], |_| 20).is_err());
        assert!(outputs[0].is_null());
        assert_eq!(outputs[1], handle);
        close_handle(&mut outputs[0], |_| panic!("freed output retried")).unwrap();
        close_handle(&mut outputs[1], |_| 0).unwrap();
    }
    #[test]
    fn preset_init_abi_and_geometry_match_the_pinned_sdk() {
        use crate::nvenc_sdk as sdk;
        assert!(validate_geometry(128, 72, 60, 1).is_ok());
        for args in [
            (0, 72, 60, 1),
            (127, 72, 60, 1),
            (128, 71, 60, 1),
            (128, 72, 0, 1),
            (128, 72, 60, 0),
        ] {
            assert!(validate_geometry(args.0, args.1, args.2, args.3).is_err());
        }
        if std::mem::size_of::<usize>() != 8 {
            return;
        }
        assert_eq!(std::mem::size_of::<sdk::NV_ENC_CONFIG>(), 3584);
        assert_eq!(std::mem::size_of::<sdk::NV_ENC_PRESET_CONFIG>(), 5128);
        assert_eq!(std::mem::size_of::<sdk::NV_ENC_INITIALIZE_PARAMS>(), 1808);
        assert_eq!(
            std::mem::offset_of!(sdk::NV_ENC_PRESET_CONFIG, presetCfg),
            8
        );
        assert_eq!(
            std::mem::offset_of!(sdk::NV_ENC_INITIALIZE_PARAMS, encodeConfig),
            88
        );
        assert_eq!(std::mem::size_of::<sdk::GUID>(), 16);
        assert_eq!(version(7) | (1 << 31), 0xf1070008);
        assert_eq!(std::mem::offset_of!(FunctionTable, preset), 88);
        assert_eq!(std::mem::offset_of!(FunctionTable, initialize), 96);
    }
    #[test]
    fn codec_query_keeps_unknown_guids_and_rejects_bad_counts() {
        let guid = CodecGuid {
            data1: 42,
            ..Default::default()
        };
        assert_eq!(
            read_guids(2, |out, count| {
                out[0] = guid;
                *count = 1;
                0
            })
            .unwrap(),
            vec![guid]
        );
        assert!(
            read_guids(1, |_, count| {
                *count = 2;
                0
            })
            .is_err()
        );
        assert!(read_guids(1, |_, _| 15).is_err());
        assert!(
            read_guids(0, |_, _| panic!("empty query invoked"))
                .unwrap()
                .is_empty()
        );
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVENC"]
    fn direct_cuda_session_opens_and_closes_without_libav() {
        let device = CodecDevice::new(0).unwrap();
        let mut session = NvencSession::open(device).unwrap();
        assert!(!session.codec_guids().unwrap().is_empty());
        session.initialize_h264(128, 72, 60, 1).unwrap();
        assert!(session.initialize_h264(128, 72, 60, 1).is_err());
        assert_eq!(session.create_output().unwrap(), 0);
        assert_eq!(session.create_output().unwrap(), 1);
        session.close().unwrap();
        assert!(session.create_output().is_err());
        assert!(session.codec_guids().is_err());
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

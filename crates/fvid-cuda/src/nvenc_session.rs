//! Direct NVENC 8.1 CUDA session ABI. No frame encoding is implemented here.
//! SDK layout: NVIDIA/video-sdk-samples@aa3544dcea2fe63122e4feb83bf805ea40e58dbe,
//! Samples/NvCodec/NvEncoder/nvEncodeAPI.h. This compatibility ABI predates AV1.
use crate::{CodecDevice, NvencApi, NvencVersion};
use std::{collections::VecDeque, ffi::c_void, mem::ManuallyDrop};
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
    encode: crate::nvenc_sdk::PNVENCENCODEPICTURE,
    lock_output: crate::nvenc_sdk::PNVENCLOCKBITSTREAM,
    unlock_output: crate::nvenc_sdk::PNVENCUNLOCKBITSTREAM,
    before_map: [usize; 6],
    map_input: crate::nvenc_sdk::PNVENCMAPINPUTRESOURCE,
    unmap_input: crate::nvenc_sdk::PNVENCUNMAPINPUTRESOURCE,
    destroy: Option<Destroy>,
    invalidate: usize,
    open: Option<Open>,
    register_input: crate::nvenc_sdk::PNVENCREGISTERRESOURCE,
    unregister_input: crate::nvenc_sdk::PNVENCUNREGISTERRESOURCE,
    tail: [usize; 5],
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
            encode: None,
            lock_output: None,
            unlock_output: None,
            before_map: [0; 6],
            map_input: None,
            unmap_input: None,
            destroy: None,
            invalidate: 0,
            open: None,
            register_input: None,
            unregister_input: None,
            tail: [0; 5],
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
struct InputResource {
    pitch: u32,
    registered: *mut c_void,
    mapped: *mut c_void,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvencSubmit {
    Ready,
    Queued,
    Busy,
}
pub struct NvencPacket {
    pub bytes: Vec<u8>,
    pub timestamp: u64,
    pub duration: u64,
    pub picture_type: u32,
}
struct LockedOutput {
    hw_status: u32,
    pointer: *mut u8,
    count: u32,
    timestamp: u64,
    duration: u64,
    picture_type: u32,
}
struct OutputResource {
    handle: *mut c_void,
    input: Option<usize>,
    ready: bool,
    locked: Option<LockedOutput>,
}
/// Owns the encoder, its CUDA resources and the loaded driver library.
/// An open session is not an initialized encoder or a verified hardware path.
pub struct NvencSession {
    encoder: *mut c_void,
    destroy: Destroy,
    table: FunctionTable,
    outputs: Vec<OutputResource>,
    pending: VecDeque<usize>,
    next_frame: u32,
    eos: bool,
    inputs: Vec<InputResource>,
    geometry: Option<(u32, u32)>,
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
            pending: VecDeque::new(),
            next_frame: 0,
            eos: false,
            inputs: Vec::new(),
            geometry: None,
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
        self.geometry = Some((width, height));
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
        self.outputs.push(OutputResource {
            handle: params.bitstreamBuffer,
            input: None,
            ready: false,
            locked: None,
        });
        Ok(index)
    }
    /// Register and map a caller-owned pitched CUDA NV12 allocation.
    ///
    /// # Safety
    /// The pointer must belong to this session's CUDA context and remain live
    /// until successful session close (including retries). Its actual readable
    /// extent must be at least allocation_bytes; Y followed by interleaved UV
    /// must use the declared pitch and initialized encoder geometry. The caller
    /// must synchronize writes before submitting and not reuse pending inputs.
    pub unsafe fn register_nv12(
        &mut self,
        pointer: u64,
        pitch: u32,
        allocation_bytes: u64,
    ) -> Result<usize, String> {
        if self.encoder.is_null() || !self.initialized || self.failed {
            return Err("NVENC registration requires a healthy initialized session".into());
        }
        let (width, height) = self.geometry.ok_or("NVENC geometry is unavailable")?;
        validate_surface(pointer, width, height, pitch, allocation_bytes)?;
        self.device.handles()?;
        let register = self
            .table
            .register_input
            .ok_or("NVENC omitted register entrypoint")?;
        let map = self.table.map_input.ok_or("NVENC omitted map entrypoint")?;
        self.table
            .unmap_input
            .ok_or("NVENC omitted unmap entrypoint")?;
        self.table
            .unregister_input
            .ok_or("NVENC omitted unregister entrypoint")?;
        self.inputs
            .try_reserve(1)
            .map_err(|e| format!("NVENC input ownership allocation failed: {e}"))?;
        let mut params = crate::nvenc_sdk::NV_ENC_REGISTER_RESOURCE::default();
        params.version = version(3);
        params.resourceType = 1;
        params.width = width;
        params.height = height;
        params.pitch = pitch;
        params.resourceToRegister = pointer as *mut c_void;
        params.bufferFormat = 1;
        // SAFETY: Caller guarantees the context, lifetime and NV12 allocation;
        // generated SDK storage and all reserved fields are correctly initialized.
        let status = unsafe { register(self.encoder, &mut params) };
        if status != 0 {
            return Err(format!(
                "NVENC input registration failed with status {status}"
            ));
        }
        if params.registeredResource.is_null() {
            self.failed = true;
            return Err("NVENC returned a null registered input".into());
        }
        let index = self.inputs.len();
        self.inputs.push(InputResource {
            pitch,
            registered: params.registeredResource,
            mapped: std::ptr::null_mut(),
        });
        let mut mapping = crate::nvenc_sdk::NV_ENC_MAP_INPUT_RESOURCE::default();
        mapping.version = version(4);
        mapping.registeredResource = params.registeredResource;
        // SAFETY: Registration is live and already tracked for cleanup; the
        // mapping output is SDK-typed writable storage on the bound context.
        let status = unsafe { map(self.encoder, &mut mapping) };
        if status != 0 {
            return Err(format!("NVENC input mapping failed with status {status}"));
        }
        self.inputs[index].mapped = mapping.mappedResource;
        if mapping.mappedResource.is_null() || mapping.mappedBufferFmt != 1 {
            self.failed = true;
            return Err("NVENC returned invalid NV12 mapping geometry".into());
        }
        Ok(index)
    }
    /// Submit a mapped NV12 input and an unused output slot. NEED_MORE_INPUT
    /// still accepts the frame. Busy accepts nothing and may be retried.
    pub fn submit_nv12(
        &mut self,
        input: usize,
        output: usize,
        timestamp: u64,
        duration: u64,
    ) -> Result<NvencSubmit, String> {
        if self.encoder.is_null() || !self.initialized || self.failed || self.eos {
            return Err(
                "NVENC submission requires a healthy initialized session before EOS".into(),
            );
        }
        let source = self
            .inputs
            .get(input)
            .ok_or("NVENC input index is invalid")?;
        let target = self
            .outputs
            .get(output)
            .ok_or("NVENC output index is invalid")?;
        if source.mapped.is_null()
            || target.handle.is_null()
            || target.input.is_some()
            || self.outputs.iter().any(|slot| slot.input == Some(input))
        {
            return Err("NVENC input/output is unavailable or pending".into());
        }
        let next_frame = self
            .next_frame
            .checked_add(1)
            .ok_or("NVENC frame index overflow")?;
        self.pending
            .try_reserve(1)
            .map_err(|e| format!("NVENC submission queue allocation failed: {e}"))?;
        self.device.handles()?;
        let encode = self
            .table
            .encode
            .ok_or("NVENC omitted picture entrypoint")?;
        let (width, height) = self.geometry.ok_or("NVENC geometry is unavailable")?;
        let mut params = crate::nvenc_sdk::NV_ENC_PIC_PARAMS::default();
        params.version = version(4) | (1 << 31);
        params.inputWidth = width;
        params.inputHeight = height;
        params.inputPitch = source.pitch;
        params.frameIdx = self.next_frame;
        params.inputTimeStamp = timestamp;
        params.inputDuration = duration;
        params.inputBuffer = source.mapped;
        params.outputBitstream = target.handle;
        params.bufferFmt = 1;
        params.pictureStruct = 1;
        // SAFETY: Session-owned mapped input/output handles are live, reserved
        // fields are zero, and caller retains the registered CUDA allocation.
        let status = unsafe { encode(self.encoder, &mut params) };
        let accepted = match submission_status(status) {
            Ok(value) => value,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        if accepted == NvencSubmit::Busy {
            return Ok(accepted);
        }
        self.outputs[output].input = Some(input);
        self.outputs[output].ready = accepted == NvencSubmit::Ready;
        self.pending.push_back(output);
        self.next_frame = next_frame;
        if accepted == NvencSubmit::Ready {
            for &index in &self.pending {
                self.outputs[index].ready = true;
            }
        }
        Ok(accepted)
    }
    /// Submit EOS once. Success makes all accepted outputs eligible to read.
    pub fn finish(&mut self) -> Result<(), String> {
        if self.encoder.is_null() || !self.initialized || self.failed {
            return Err("NVENC finish requires a healthy initialized session".into());
        }
        self.flush_encoder()
    }
    fn flush_encoder(&mut self) -> Result<(), String> {
        if self.eos {
            return Ok(());
        }
        self.device.handles()?;
        let encode = self
            .table
            .encode
            .ok_or("NVENC omitted picture entrypoint")?;
        let mut params = crate::nvenc_sdk::NV_ENC_PIC_PARAMS::default();
        params.version = version(4) | (1 << 31);
        params.encodePicFlags = 8;
        // SAFETY: SDK EOS parameters carry no input/output pointer; the encoder
        // and owning device remain live until accepted work is drained.
        let status = unsafe { encode(self.encoder, &mut params) };
        if status != 0 {
            return Err(format!("NVENC EOS failed with status {status}"));
        }
        self.eos = true;
        for &index in &self.pending {
            self.outputs[index].ready = true;
        }
        Ok(())
    }
    /// Poll the oldest accepted output; returns None while the driver is busy
    /// or needs further input. Successful unlock precedes slot/input reuse.
    pub fn receive(&mut self) -> Result<Option<NvencPacket>, String> {
        if self.encoder.is_null() || self.failed {
            return Err("NVENC output requires a healthy live session".into());
        }
        let Some(&index) = self.pending.front() else {
            return Ok(None);
        };
        if !self.outputs[index].ready {
            return Ok(None);
        }
        self.device.handles()?;
        if !self.lock_slot(index, true)? {
            return Ok(None);
        }
        let unlock = self
            .table
            .unlock_output
            .ok_or("NVENC omitted output-unlock entrypoint")?;
        let locked = self.outputs[index].locked.as_ref().unwrap();
        if locked.hw_status != 0 || locked.count == 0 {
            return Err(format!(
                "NVENC returned invalid encoded output: hardware status {}, bytes {}",
                locked.hw_status, locked.count
            ));
        }
        let count = usize::try_from(locked.count).map_err(|_| "NVENC output size overflow")?;
        if (count != 0 && locked.pointer.is_null()) || count > isize::MAX as usize {
            return Err("NVENC locked output has invalid pointer/length".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(count)
            .map_err(|e| format!("NVENC packet allocation failed: {e}"))?;
        if count != 0 {
            // SAFETY: Successful SDK lock guarantees this readable extent until
            // unlock; zero length avoids creating a slice from a null pointer.
            bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(locked.pointer, count) });
        }
        let packet = NvencPacket {
            bytes,
            timestamp: locked.timestamp,
            duration: locked.duration,
            picture_type: locked.picture_type,
        };
        // SAFETY: The output remains locked and the driver library is live.
        let status = unsafe { unlock(self.encoder, self.outputs[index].handle) };
        if status != 0 {
            return Err(format!("NVENC output unlock failed with status {status}"));
        }
        self.outputs[index].locked = None;
        self.outputs[index].input = None;
        self.outputs[index].ready = false;
        self.pending.pop_front();
        Ok(Some(packet))
    }
    fn lock_slot(&mut self, index: usize, nonblocking: bool) -> Result<bool, String> {
        let lock = self
            .table
            .lock_output
            .ok_or("NVENC omitted output-lock entrypoint")?;
        if self.outputs[index].locked.is_none() {
            let mut params = crate::nvenc_sdk::NV_ENC_LOCK_BITSTREAM::default();
            params.version = version(1);
            params.outputBitstream = self.outputs[index].handle;
            params.set_doNotWait(u32::from(nonblocking));
            // SAFETY: The oldest ready output belongs to this live encoder;
            // SDK storage is writable and nonblocking mode avoids deadlock.
            let status = unsafe { lock(self.encoder, &mut params) };
            if status == crate::nvenc_sdk::_NVENCSTATUS_NV_ENC_ERR_LOCK_BUSY {
                return Ok(false);
            }
            if status != 0 {
                return Err(format!("NVENC output lock failed with status {status}"));
            }
            self.outputs[index].locked = Some(LockedOutput {
                hw_status: params.hwEncodeStatus,
                pointer: params.bitstreamBufferPtr.cast(),
                count: params.bitstreamSizeInBytes,
                timestamp: params.outputTimeStamp,
                duration: params.outputDuration,
                picture_type: params.pictureType,
            });
        }
        Ok(true)
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
        if self.initialized && (!self.pending.is_empty() || self.failed) {
            self.flush_encoder()?;
        }
        while let Some(&index) = self.pending.front() {
            // A successful EOS makes outputs eligible; blocking lock waits for
            // completion before the associated CUDA input can be released.
            if !self.lock_slot(index, false)? {
                return Err("NVENC cleanup output is still busy".into());
            }
            let unlock = self
                .table
                .unlock_output
                .ok_or("NVENC omitted output-unlock entrypoint")?;
            // SAFETY: This pending slot has a successful outstanding lock.
            let status = unsafe { unlock(self.encoder, self.outputs[index].handle) };
            if status != 0 {
                return Err(format!("NVENC cleanup unlock failed with status {status}"));
            }
            self.outputs[index].locked = None;
            self.outputs[index].input = None;
            self.outputs[index].ready = false;
            self.pending.pop_front();
        }
        if !self.inputs.is_empty() {
            let unmap = self
                .table
                .unmap_input
                .ok_or("NVENC omitted unmap entrypoint")?;
            let unregister = self
                .table
                .unregister_input
                .ok_or("NVENC omitted unregister entrypoint")?;
            for input in &mut self.inputs {
                release_input(
                    input,
                    // SAFETY: Each mapped handle belongs to this live encoder;
                    // helper ordering prevents unregister while mapping is live.
                    |handle| unsafe { unmap(self.encoder, handle) as i32 },
                    // SAFETY: Each registered handle belongs to this encoder;
                    // successful releases are cleared before a retry.
                    |handle| unsafe { unregister(self.encoder, handle) as i32 },
                )?;
            }
        }
        if !self.outputs.is_empty() {
            let destroy = self
                .table
                .destroy_output
                .ok_or("NVENC omitted output-destroy entrypoint")?;
            for output in &mut self.outputs {
                // SAFETY: Each nonnull output belongs to this live encoder;
                // successful destruction clears it, preventing a repeated free.
                close_handle(&mut output.handle, |handle| unsafe {
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
fn submission_status(status: u32) -> Result<NvencSubmit, String> {
    match status {
        0 => Ok(NvencSubmit::Ready),
        crate::nvenc_sdk::_NVENCSTATUS_NV_ENC_ERR_NEED_MORE_INPUT => Ok(NvencSubmit::Queued),
        crate::nvenc_sdk::_NVENCSTATUS_NV_ENC_ERR_ENCODER_BUSY => Ok(NvencSubmit::Busy),
        _ => Err(format!(
            "NVENC picture submission failed with status {status}"
        )),
    }
}
fn release_input(
    input: &mut InputResource,
    unmap: impl FnOnce(*mut c_void) -> i32,
    unregister: impl FnOnce(*mut c_void) -> i32,
) -> Result<(), String> {
    close_handle(&mut input.mapped, unmap).map_err(|e| format!("NVENC input unmap: {e}"))?;
    close_handle(&mut input.registered, unregister)
        .map_err(|e| format!("NVENC input unregister: {e}"))
}
fn validate_surface(
    pointer: u64,
    width: u32,
    height: u32,
    pitch: u32,
    bytes: u64,
) -> Result<(), String> {
    validate_geometry(width, height, 1, 1)?;
    let minimum = u64::from(pitch)
        .checked_mul(u64::from(height) + u64::from(height) / 2)
        .ok_or("NVENC NV12 allocation size overflow")?;
    if pointer.checked_add(bytes).is_none()
        || pointer == 0
        || usize::try_from(pointer).is_err()
        || pitch < width
        || pitch % 2 != 0
        || bytes < minimum
    {
        return Err("NVENC NV12 pointer, pitch or allocation extent is invalid".into());
    }
    Ok(())
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
    #[test]
    fn submission_distinguishes_buffered_frames_from_unaccepted_busy_frames() {
        assert_eq!(
            super::submission_status(0).unwrap(),
            super::NvencSubmit::Ready
        );
        assert_eq!(
            super::submission_status(17).unwrap(),
            super::NvencSubmit::Queued
        );
        assert_eq!(
            super::submission_status(18).unwrap(),
            super::NvencSubmit::Busy
        );
        assert!(super::submission_status(20).is_err());
    }
    #[test]
    fn submission_entrypoints_preserve_sdk_function_table_offsets() {
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(std::mem::offset_of!(super::FunctionTable, encode), 136);
            assert_eq!(std::mem::offset_of!(super::FunctionTable, lock_output), 144);
            assert_eq!(
                std::mem::offset_of!(super::FunctionTable, unlock_output),
                152
            );
        }
    }
    use super::*;
    #[test]
    fn input_cleanup_does_not_unregister_a_failed_mapping_release() {
        let pointer = std::ptr::dangling_mut::<c_void>();
        let mut input = InputResource {
            pitch: 128,
            registered: pointer,
            mapped: pointer,
        };
        assert!(
            release_input(
                &mut input,
                |_| 20,
                |_| panic!("unregistered a live mapping")
            )
            .is_err()
        );
        assert_eq!(input.mapped, pointer);
        assert_eq!(input.registered, pointer);
        assert!(release_input(&mut input, |_| 0, |_| 20).is_err());
        assert!(input.mapped.is_null());
        assert_eq!(input.registered, pointer);
        release_input(
            &mut input,
            |_| panic!("mapped handle released twice"),
            |_| 0,
        )
        .unwrap();
        assert!(input.registered.is_null());
    }
    #[test]
    fn nv12_surface_extent_includes_both_planes() {
        assert!(validate_surface(256, 128, 72, 256, 27648).is_ok());
        assert!(validate_surface(0, 128, 72, 256, 27648).is_err());
        assert!(validate_surface(256, 128, 72, 126, 27648).is_err());
        assert!(validate_surface(256, 128, 72, 257, 30000).is_err());
        assert!(validate_surface(256, 128, 72, 256, 18432).is_err());
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(
                std::mem::size_of::<crate::nvenc_sdk::NV_ENC_REGISTER_RESOURCE>(),
                1536
            );
            assert_eq!(
                std::mem::size_of::<crate::nvenc_sdk::NV_ENC_MAP_INPUT_RESOURCE>(),
                1544
            );
            assert_eq!(std::mem::offset_of!(FunctionTable, map_input), 208);
            assert_eq!(std::mem::offset_of!(FunctionTable, register_input), 248);
        }
    }
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
        use cudarc::driver::DevicePtr;
        let owner = crate::device_pool::shared(0).unwrap();
        let stream = owner.new_stream().unwrap();
        let buffers: Vec<_> = (0..4)
            .map(|_| stream.alloc_zeros::<u8>(128 * 72 * 3 / 2).unwrap())
            .collect();
        stream.synchronize().unwrap();
        let device = CodecDevice::new(0).unwrap();
        let mut session = NvencSession::open(device).unwrap();
        assert!(!session.codec_guids().unwrap().is_empty());
        session.initialize_h264(128, 72, 60, 1).unwrap();
        assert!(session.initialize_h264(128, 72, 60, 1).is_err());
        for (index, buffer) in buffers.iter().enumerate() {
            let (pointer, _guard) = buffer.device_ptr(&stream);
            // SAFETY: All allocations use the same pooled primary context,
            // are synchronized and remain live until successful close.
            assert_eq!(
                unsafe { session.register_nv12(pointer, 128, 128 * 72 * 3 / 2) }.unwrap(),
                index
            );
            assert_eq!(session.create_output().unwrap(), index);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        for index in 0..buffers.len() {
            loop {
                let result = session.submit_nv12(index, index, index as u64, 1).unwrap();
                if result != super::NvencSubmit::Busy {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "NVENC submission remained busy"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        session.finish().unwrap();
        session.finish().unwrap();
        assert!(session.submit_nv12(0, 0, 99, 1).is_err());
        let mut timestamps = Vec::new();
        while timestamps.len() < buffers.len() {
            if let Some(packet) = session.receive().unwrap() {
                assert!(!packet.bytes.is_empty());
                assert_eq!(packet.duration, 1);
                timestamps.push(packet.timestamp);
            } else {
                assert!(
                    std::time::Instant::now() < deadline,
                    "NVENC output remained busy"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        timestamps.sort_unstable();
        assert_eq!(timestamps, vec![0, 1, 2, 3]);
        assert!(session.receive().unwrap().is_none());
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

//! Hardware H.264 decoding on macOS through VideoToolbox, Apple's interface
//! to the media engine. Frames come back as packed 8-bit planar 4:2:0 in
//! decode order; the caller reorders them for display exactly as it does for
//! the software decoder. The bindings are the plain C API (CoreFoundation,
//! CoreMedia, CoreVideo, VideoToolbox), no Objective-C.
#![cfg(target_os = "macos")]
use std::{ffi::c_void, fmt, ptr};

type OSStatus = i32;
type CFRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
#[repr(C)]
struct CallbackRecord {
    callback: Option<OutputCallback>,
    refcon: *mut c_void,
}
type OutputCallback = unsafe extern "C" fn(
    refcon: *mut c_void,
    source: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    image: CFRef,
    pts: CMTime,
    duration: CMTime,
);
/// Sized stand-ins for the opaque CoreFoundation callback tables; only their
/// addresses are used.
#[repr(C)]
struct KeyCallBacks {
    _words: [usize; 7],
}
#[repr(C)]
struct ValueCallBacks {
    _words: [usize; 6],
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorNull: CFRef;
    static kCFTypeDictionaryKeyCallBacks: KeyCallBacks;
    static kCFTypeDictionaryValueCallBacks: ValueCallBacks;
    fn CFRelease(cf: CFRef);
    fn CFDictionaryCreate(
        allocator: CFRef,
        keys: *const CFRef,
        values: *const CFRef,
        count: isize,
        key_callbacks: *const KeyCallBacks,
        value_callbacks: *const ValueCallBacks,
    ) -> CFRef;
    fn CFNumberCreate(allocator: CFRef, number_type: isize, value: *const c_void) -> CFRef;
    fn CFDataCreate(allocator: CFRef, bytes: *const u8, length: usize) -> CFRef;
    fn CFStringCreateWithCString(
        allocator: CFRef,
        c_str: *const u8,
        encoding: u32,
    ) -> CFRef;
}
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
fn cfstr(s: &str) -> CFRef {
    // SAFETY: s is a valid UTF-8 string that we pass as a C string with a null terminator.
    unsafe {
        let mut buf = s.as_bytes().to_vec();
        buf.push(0);
        CFStringCreateWithCString(ptr::null(), buf.as_ptr(), K_CF_STRING_ENCODING_UTF8)
    }
}
macro_rules! CFSTR {
    ($s:expr) => {
        cfstr($s)
    };
}
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFRef,
        count: usize,
        pointers: *const *const u8,
        sizes: *const usize,
        nal_length_size: i32,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        allocator: CFRef,
        count: usize,
        pointers: *const *const u8,
        sizes: *const usize,
        nal_length_size: i32,
        parameter_set_strings: *const c_void,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionCreate(
        allocator: CFRef,
        codec_type: u32,
        width: i32,
        height: i32,
        extensions: CFRef,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFRef,
        memory: *mut c_void,
        block_length: usize,
        block_allocator: CFRef,
        custom_source: *const c_void,
        offset: usize,
        data_length: usize,
        flags: u32,
        out: *mut CFRef,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFRef,
        data: CFRef,
        format: CFRef,
        sample_count: isize,
        timing_count: isize,
        timing: *const c_void,
        size_count: isize,
        sizes: *const usize,
        out: *mut CFRef,
    ) -> OSStatus;
}
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFRef;
    fn CVPixelBufferLockBaseAddress(buffer: CFRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: CFRef, flags: u64) -> i32;
    fn CVPixelBufferGetPixelFormatType(buffer: CFRef) -> u32;
    fn CVPixelBufferGetPlaneCount(buffer: CFRef) -> usize;
    fn CVPixelBufferGetWidthOfPlane(buffer: CFRef, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(buffer: CFRef, plane: usize) -> usize;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: CFRef, plane: usize) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: CFRef, plane: usize) -> *const u8;
}
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: CFRef,
        format: CFRef,
        decoder_specification: CFRef,
        destination_attributes: CFRef,
        callback: *const CallbackRecord,
        out: *mut CFRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: CFRef,
        sample: CFRef,
        flags: u32,
        source: *mut c_void,
        info_flags: *mut u32,
    ) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: CFRef);
}

/// kCVPixelFormatType_420YpCbCr8Planar ('y420') and its full-range twin ('f420').
const PLANAR_420: u32 = 0x7934_3230;
const PLANAR_420_FULL: u32 = 0x6634_3230;
const CF_NUMBER_SINT32: isize = 3;
const READ_ONLY_LOCK: u64 = 1;
const K_CM_VIDEO_CODEC_TYPE_VP9: u32 = 0x7670_3039; // 'vp09'
const K_CM_VIDEO_CODEC_TYPE_AV1: u32 = 0x6176_3031; // 'av01'

#[derive(Debug)]
pub struct Error(String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
fn status(what: &str, code: OSStatus) -> Error {
    Error(format!("VideoToolbox: {what} failed ({code})"))
}

/// A decoded picture as tightly packed 8-bit planes; chroma planes are
/// `width.div_ceil(2)` by `height.div_ceil(2)`.
pub struct Planes {
    pub width: usize,
    pub height: usize,
    pub full_range: bool,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
}

/// One frame's output slot, addressed through the source refcon.
/// kVTVideoDecoderReferenceMissingErr: the frame predicts from a picture the
/// decoder never saw (a stream opening mid-GOP, open-GOP leading frames).
/// Nothing can be shown for it, but decoding recovers at the next keyframe.
const REFERENCE_MISSING: OSStatus = -17694;

struct Slot {
    result: Result<Option<Planes>, Error>,
}

unsafe extern "C" fn output(
    _refcon: *mut c_void,
    source: *mut c_void,
    code: OSStatus,
    _info_flags: u32,
    image: CFRef,
    _pts: CMTime,
    _duration: CMTime,
) {
    // SAFETY: `source` is the `Slot` the synchronous decode call passed and
    // still owns; VideoToolbox invokes this callback before that call returns.
    let slot = unsafe { &mut *(source as *mut Slot) };
    if code == REFERENCE_MISSING {
        slot.result = Ok(None);
        return;
    }
    if code != 0 {
        slot.result = Err(status("frame decode", code));
        return;
    }
    if image.is_null() {
        // A dropped frame: nothing to show for this access unit.
        slot.result = Ok(None);
        return;
    }
    slot.result = unsafe { copy_planes(image) };
}

unsafe fn copy_planes(image: CFRef) -> Result<Option<Planes>, Error> {
    // SAFETY: `image` is a live CVPixelBuffer for the duration of the callback.
    unsafe {
        let format = CVPixelBufferGetPixelFormatType(image);
        let full_range = match format {
            PLANAR_420 => false,
            PLANAR_420_FULL => true,
            other => return Err(Error(format!("VideoToolbox: unexpected pixel format {other:#x}"))),
        };
        if CVPixelBufferGetPlaneCount(image) != 3 {
            return Err(Error("VideoToolbox: planar output has no three planes".into()));
        }
        let lock = CVPixelBufferLockBaseAddress(image, READ_ONLY_LOCK);
        if lock != 0 {
            return Err(status("pixel buffer lock", lock));
        }
        let mut planes = [Vec::new(), Vec::new(), Vec::new()];
        let mut dims = [(0, 0); 3];
        for (index, plane) in planes.iter_mut().enumerate() {
            let width = CVPixelBufferGetWidthOfPlane(image, index);
            let height = CVPixelBufferGetHeightOfPlane(image, index);
            let stride = CVPixelBufferGetBytesPerRowOfPlane(image, index);
            let base = CVPixelBufferGetBaseAddressOfPlane(image, index);
            if base.is_null() || stride < width {
                CVPixelBufferUnlockBaseAddress(image, READ_ONLY_LOCK);
                return Err(Error("VideoToolbox: invalid plane layout".into()));
            }
            let mut out = Vec::with_capacity(width * height);
            for row in 0..height {
                let line = std::slice::from_raw_parts(base.add(row * stride), width);
                out.extend_from_slice(line);
            }
            *plane = out;
            dims[index] = (width, height);
        }
        CVPixelBufferUnlockBaseAddress(image, READ_ONLY_LOCK);
        let (width, height) = dims[0];
        if dims[1] != (width.div_ceil(2), height.div_ceil(2)) || dims[2] != dims[1] {
            return Err(Error("VideoToolbox: chroma planes are not 4:2:0".into()));
        }
        let [y, cb, cr] = planes;
        Ok(Some(Planes {
            width,
            height,
            full_range,
            y,
            cb,
            cr,
        }))
    }
}

/// A decompression session for one video stream (parameter sets from avcC or hvcC).
pub struct Session {
    format: CFRef,
    session: CFRef,
}
// SAFETY: the session is only ever driven from one thread at a time; the
// handle itself may move between threads.
unsafe impl Send for Session {}

impl Session {
    /// Create an H.264 session. `sps` and `pps` are NAL units including their
    /// header byte, as stored in avcC; `nal_length_size` is the sample's
    /// length-prefix width (1, 2, 4).
    pub fn new(sps: &[&[u8]], pps: &[&[u8]], nal_length_size: u8) -> Result<Self, Error> {
        if sps.is_empty() || pps.is_empty() || !matches!(nal_length_size, 1 | 2 | 4) {
            return Err(Error("VideoToolbox: invalid parameter sets".into()));
        }
        let sets: Vec<&[u8]> = sps.iter().chain(pps).copied().collect();
        let pointers: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        // SAFETY: plain C calls with valid pointers; every created object is
        // released on failure or in `Drop`.
        unsafe {
            let mut format = ptr::null();
            let code = CMVideoFormatDescriptionCreateFromH264ParameterSets(
                ptr::null(),
                sets.len(),
                pointers.as_ptr(),
                sizes.as_ptr(),
                i32::from(nal_length_size),
                &mut format,
            );
            if code != 0 || format.is_null() {
                return Err(status("format description", code));
            }
            Self::from_format(format)
        }
    }
    
    /// Create an HEVC session. `vps`, `sps`, and `pps` are NAL units including
    /// their header byte, as stored in hvcC; `nal_length_size` is the sample's
    /// length-prefix width (1, 2, 4).
    pub fn new_hevc(
        vps: &[&[u8]],
        sps: &[&[u8]],
        pps: &[&[u8]],
        nal_length_size: u8,
    ) -> Result<Self, Error> {
        if vps.is_empty() || sps.is_empty() || pps.is_empty() || !matches!(nal_length_size, 1 | 2 | 4) {
            return Err(Error("VideoToolbox: invalid HEVC parameter sets".into()));
        }
        let sets: Vec<&[u8]> = vps.iter().chain(sps).chain(pps).copied().collect();
        let pointers: Vec<*const u8> = sets.iter().map(|s| s.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|s| s.len()).collect();
        // SAFETY: plain C calls with valid pointers; every created object is
        // released on failure or in `Drop`.
        unsafe {
            let mut format = ptr::null();
            let code = CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                ptr::null(),
                sets.len(),
                pointers.as_ptr(),
                sizes.as_ptr(),
                i32::from(nal_length_size),
                ptr::null(),
                &mut format,
            );
            if code != 0 || format.is_null() {
                return Err(status("HEVC format description", code));
            }
            Self::from_format(format)
        }
    }
    
    /// Create a VP9 session. `config` is the vpcC configuration box contents;
    /// `width` and `height` are the coded dimensions.
    pub fn new_vp9(config: &[u8], width: u32, height: u32) -> Result<Self, Error> {
        if config.is_empty() {
            return Err(Error("VideoToolbox: empty VP9 configuration".into()));
        }
        unsafe {
            let config_data = CFDataCreate(
                ptr::null(),
                config.as_ptr(),
                config.len(),
            );
            if config_data.is_null() {
                return Err(Error("VideoToolbox: VP9 config data creation failed".into()));
            }
            let keys = [CFSTR!("vpcC")];
            let values = [config_data];
            let extensions = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            CFRelease(config_data);
            let mut format = ptr::null();
            let code = CMVideoFormatDescriptionCreate(
                ptr::null(),
                K_CM_VIDEO_CODEC_TYPE_VP9,
                width as i32,
                height as i32,
                extensions,
                &mut format,
            );
            if !extensions.is_null() {
                CFRelease(extensions);
            }
            if code != 0 || format.is_null() {
                return Err(status("VP9 format description", code));
            }
            Self::from_format(format)
        }
    }
    
    /// Create an AV1 session. `config` is the av1C configuration box contents;
    /// `width` and `height` are the coded dimensions.
    pub fn new_av1(config: &[u8], width: u32, height: u32) -> Result<Self, Error> {
        if config.is_empty() {
            return Err(Error("VideoToolbox: empty AV1 configuration".into()));
        }
        unsafe {
            let config_data = CFDataCreate(
                ptr::null(),
                config.as_ptr(),
                config.len(),
            );
            if config_data.is_null() {
                return Err(Error("VideoToolbox: AV1 config data creation failed".into()));
            }
            let keys = [CFSTR!("av1C")];
            let values = [config_data];
            let extensions = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            CFRelease(config_data);
            let mut format = ptr::null();
            let code = CMVideoFormatDescriptionCreate(
                ptr::null(),
                K_CM_VIDEO_CODEC_TYPE_AV1,
                width as i32,
                height as i32,
                extensions,
                &mut format,
            );
            if !extensions.is_null() {
                CFRelease(extensions);
            }
            if code != 0 || format.is_null() {
                return Err(status("AV1 format description", code));
            }
            Self::from_format(format)
        }
    }
    
    unsafe fn from_format(format: CFRef) -> Result<Self, Error> {
        // SAFETY: format is a valid CMVideoFormatDescription; all created objects
        // are released on failure or in `Drop`.
        unsafe {
            let pixel_format: i32 = PLANAR_420 as i32;
            let number = CFNumberCreate(
                ptr::null(),
                CF_NUMBER_SINT32,
                &pixel_format as *const i32 as *const c_void,
            );
            let keys = [kCVPixelBufferPixelFormatTypeKey];
            let values = [number];
            let attributes = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            CFRelease(number);
            let record = CallbackRecord {
                callback: Some(output),
                refcon: ptr::null_mut(),
            };
            let mut session = ptr::null();
            let code = VTDecompressionSessionCreate(
                ptr::null(),
                format,
                ptr::null(),
                attributes,
                &record,
                &mut session,
            );
            if !attributes.is_null() {
                CFRelease(attributes);
            }
            if code != 0 || session.is_null() {
                CFRelease(format);
                return Err(status("session creation", code));
            }
            Ok(Self { format, session })
        }
    }
    /// Decode one length-prefixed access unit synchronously. `None` means the
    /// decoder produced no picture for it.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<Planes>, Error> {
        if packet.is_empty() {
            return Err(Error("VideoToolbox: empty access unit".into()));
        }
        let mut slot = Slot { result: Ok(None) };
        // SAFETY: the block buffer borrows `packet` (kCFAllocatorNull, never
        // freed by CoreMedia) only for this synchronous call, and `slot`
        // outlives the decode call that writes to it.
        unsafe {
            let mut block = ptr::null();
            let code = CMBlockBufferCreateWithMemoryBlock(
                ptr::null(),
                packet.as_ptr() as *mut c_void,
                packet.len(),
                kCFAllocatorNull,
                ptr::null(),
                0,
                packet.len(),
                0,
                &mut block,
            );
            if code != 0 || block.is_null() {
                return Err(status("block buffer", code));
            }
            let sizes = [packet.len()];
            let mut sample = ptr::null();
            let code = CMSampleBufferCreateReady(
                ptr::null(),
                block,
                self.format,
                1,
                0,
                ptr::null(),
                1,
                sizes.as_ptr(),
                &mut sample,
            );
            if code != 0 || sample.is_null() {
                CFRelease(block);
                return Err(status("sample buffer", code));
            }
            let mut info = 0u32;
            let code = VTDecompressionSessionDecodeFrame(
                self.session,
                sample,
                0,
                &mut slot as *mut Slot as *mut c_void,
                &mut info,
            );
            CFRelease(sample);
            CFRelease(block);
            if code == REFERENCE_MISSING {
                return Ok(None);
            }
            if code != 0 {
                return Err(status("decode", code));
            }
        }
        slot.result
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: both handles were created by this session and are released once.
        unsafe {
            VTDecompressionSessionInvalidate(self.session);
            CFRelease(self.session);
            CFRelease(self.format);
        }
    }
}

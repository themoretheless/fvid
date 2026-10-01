//! Hardware compression of retained CoreVideo surfaces without pixel downloads.
use super::*;
use std::sync::Mutex;

type Callback = unsafe extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, CFRef);
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetFormatDescription(sample: CFRef) -> CFRef;
    fn CMSampleBufferGetPresentationTimeStamp(sample: CFRef) -> CMTime;
    fn CMSampleBufferGetDuration(sample: CFRef) -> CMTime;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        format: CFRef,
        index: usize,
        bytes: *mut *const u8,
        length: *mut usize,
        count: *mut usize,
        prefix: *mut i32,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
        format: CFRef,
        index: usize,
        bytes: *mut *const u8,
        length: *mut usize,
        count: *mut usize,
        prefix: *mut i32,
    ) -> OSStatus;
    static kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: CFRef;
    static kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder: CFRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFRef;
    static kVTCompressionPropertyKey_ColorPrimaries: CFRef;
    static kVTCompressionPropertyKey_HDRMetadataInsertionMode: CFRef;
    static kVTHDRMetadataInsertionMode_None: CFRef;
    static kVTCompressionPropertyKey_TransferFunction: CFRef;
    static kVTCompressionPropertyKey_YCbCrMatrix: CFRef;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFRef;
    static kCVImageBufferColorPrimaries_ITU_R_2020: CFRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFRef;
    static kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ: CFRef;
    static kCVImageBufferTransferFunction_ITU_R_2100_HLG: CFRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_709_2: CFRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_2020: CFRef;
    static kCVImageBufferColorPrimariesKey: CFRef;
    static kCVImageBufferTransferFunctionKey: CFRef;
    static kCVImageBufferYCbCrMatrixKey: CFRef;
    fn CVPixelBufferGetIOSurface(image: CFRef) -> CFRef;
    fn CVPixelBufferCreateWithIOSurface(
        allocator: CFRef,
        surface: CFRef,
        attributes: CFRef,
        image: *mut CFRef,
    ) -> i32;
    fn CVBufferSetAttachment(buffer: CFRef, key: CFRef, value: CFRef, mode: u32);
    static kVTProfileLevel_HEVC_Main10_AutoLevel: CFRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFRef;
    fn VTCompressionSessionCreate(
        allocator: CFRef,
        width: i32,
        height: i32,
        codec: u32,
        specification: CFRef,
        attributes: CFRef,
        compressed_allocator: CFRef,
        callback: Option<Callback>,
        refcon: *mut c_void,
        out: *mut CFRef,
    ) -> OSStatus;
    fn VTCompressionSessionPrepareToEncodeFrames(session: CFRef) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        session: CFRef,
        image: CFRef,
        pts: CMTime,
        duration: CMTime,
        properties: CFRef,
        source: *mut c_void,
        flags: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(session: CFRef, until: CMTime) -> OSStatus;
    fn VTCompressionSessionInvalidate(session: CFRef);
    fn VTSessionSetProperty(session: CFRef, key: CFRef, value: CFRef) -> OSStatus;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanFalse: CFRef;
    fn CFDictionaryGetValue(dictionary: CFRef, key: CFRef) -> CFRef;
    fn CFGetTypeID(value: CFRef) -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDataGetLength(value: CFRef) -> isize;
    fn CFDataGetBytePtr(value: CFRef) -> *const u8;
    fn CFArrayGetCount(array: CFRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
}
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms: CFRef;
    fn CMFormatDescriptionGetExtensions(format: CFRef) -> CFRef;
    static kCMSampleAttachmentKey_NotSync: CFRef;
    fn CMSampleBufferGetSampleAttachmentsArray(sample: CFRef, create: u8) -> CFRef;
    fn CMSampleBufferGetDataBuffer(sample: CFRef) -> CFRef;
    fn CMBlockBufferGetDataLength(buffer: CFRef) -> usize;
    fn CMBlockBufferCopyDataBytes(
        buffer: CFRef,
        offset: usize,
        length: usize,
        out: *mut c_void,
    ) -> OSStatus;
}

#[derive(Clone, Copy, Debug)]
pub enum EncoderCodec {
    H264,
    Hevc,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedTime {
    pub value: i64,
    pub timescale: u32,
}
impl EncodedTime {
    /// Convert for a nanosecond container timeline, truncating sub-nanosecond
    /// fractions toward zero. Reject malformed times and signed overflow.
    pub fn nanoseconds(self) -> Result<i64, Error> {
        if self.timescale == 0 {
            return Err(Error("timestamp has zero timescale".into()));
        }
        let value = i128::from(self.value) * 1_000_000_000 / i128::from(self.timescale);
        i64::try_from(value).map_err(|_| Error("timestamp exceeds signed nanosecond range".into()))
    }
    fn from_core(time: CMTime) -> Result<Self, Error> {
        if time.flags & 1 == 0 || time.flags & 28 != 0 || time.timescale <= 0 || time.epoch != 0 {
            return Err(Error("encoder returned non-numeric sample timing".into()));
        }
        Ok(Self {
            value: time.value,
            timescale: time.timescale as u32,
        })
    }
}
/// Compressed length-prefixed access unit. Pixel memory never visits the host.
pub struct EncodedFrame {
    pub data: Vec<u8>,
    /// Authored SPS/PPS for AVC; VPS/SPS/PPS for HEVC, retaining NAL headers.
    pub parameter_sets: Vec<Vec<u8>>,
    pub nal_length_size: u8,
    /// Exact avcC/hvcC payload for container codec-private data.
    pub decoder_configuration: Vec<u8>,
    /// Independent decodability reported by CoreMedia sample attachments.
    pub key_frame: bool,
    pub pts: EncodedTime,
    pub duration: EncodedTime,
}
struct Sample(CFRef);
// SAFETY: callback retains immutable CMSampleBuffer data; access is serialized.
unsafe impl Send for Sample {}
impl Drop for Sample {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
#[derive(Default)]
struct Output {
    sample: Option<Sample>,
    error: Option<OSStatus>,
}
unsafe extern "C" fn output(
    refcon: *mut c_void,
    _: *mut c_void,
    code: OSStatus,
    flags: u32,
    sample: CFRef,
) {
    // The boxed mutex stays at a stable address until session invalidation.
    let state = unsafe { &*refcon.cast::<Mutex<Output>>() };
    let Ok(mut state) = state.lock() else {
        return;
    };
    if code != 0 || sample.is_null() || flags & 2 != 0 || state.sample.is_some() {
        state.error = Some(if code != 0 { code } else { -1 });
    } else {
        state.sample = Some(Sample(unsafe { CFRetain(sample) }));
    }
}
/// Synchronous hardware encoder. Completing each frame bounds retained output
/// to one sample and keeps callback lifetime independent of the caller's stack.
pub struct Encoder {
    session: CFRef,
    output: Box<Mutex<Output>>,
    size: [usize; 2],
    failed: bool,
    codec: EncoderCodec,
    depth: u8,
    colour: Option<[u8; 3]>,
    started: bool,
}
impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            VTCompressionSessionInvalidate(self.session);
            CFRelease(self.session);
        }
    }
}
impl Encoder {
    /// Set the encoded signal before submitting frames. Supported output
    /// signals are Rec.709 and BT.2020 with BT.709, PQ or HLG transfer.
    pub fn set_colour(&mut self, primaries: u8, transfer: u8, matrix: u8) -> Result<(), Error> {
        if self.started || self.failed {
            return Err(Error("encoder colour must be set before frames".into()));
        }
        if ![1, 9].contains(&primaries)
            || ![1, 16, 18].contains(&transfer)
            || ![1, 9].contains(&matrix)
        {
            return Err(Error("unsupported hardware encoder colour signal".into()));
        }
        // SDK constants identify the strings accepted by VTSessionSetProperty.
        unsafe {
            let values = [
                (
                    kVTCompressionPropertyKey_ColorPrimaries,
                    if primaries == 1 {
                        kCVImageBufferColorPrimaries_ITU_R_709_2
                    } else {
                        kCVImageBufferColorPrimaries_ITU_R_2020
                    },
                ),
                (
                    kVTCompressionPropertyKey_TransferFunction,
                    match transfer {
                        16 => kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ,
                        18 => kCVImageBufferTransferFunction_ITU_R_2100_HLG,
                        _ => kCVImageBufferTransferFunction_ITU_R_709_2,
                    },
                ),
                (
                    kVTCompressionPropertyKey_YCbCrMatrix,
                    if matrix == 1 {
                        kCVImageBufferYCbCrMatrix_ITU_R_709_2
                    } else {
                        kCVImageBufferYCbCrMatrix_ITU_R_2020
                    },
                ),
            ];
            for (key, value) in values {
                let code = VTSessionSetProperty(self.session, key, value);
                if code != 0 {
                    self.failed = true;
                    return Err(status("encoder colour", code));
                }
            }
        }
        self.colour = Some([primaries, transfer, matrix]);
        Ok(())
    }
    pub fn new(codec: EncoderCodec, width: usize, height: usize) -> Result<Self, Error> {
        Self::new_with_depth(codec, width, height, 8)
    }
    pub fn new_with_depth(
        codec: EncoderCodec,
        width: usize,
        height: usize,
        depth: u8,
    ) -> Result<Self, Error> {
        Self::configured(codec, width, height, depth, None)
    }
    /// Target average rate in bits per second. The encoder's rate control is
    /// lossy and does not promise an exact output size for a short stream.
    pub fn new_with_bitrate(
        codec: EncoderCodec,
        width: usize,
        height: usize,
        depth: u8,
        bits_per_second: u32,
    ) -> Result<Self, Error> {
        if bits_per_second == 0 || bits_per_second > i32::MAX as u32 {
            return Err(Error(
                "hardware encoder bitrate must fit a positive signed 32-bit value".into(),
            ));
        }
        Self::configured(codec, width, height, depth, Some(bits_per_second as i32))
    }
    fn configured(
        codec: EncoderCodec,
        width: usize,
        height: usize,
        depth: u8,
        bitrate: Option<i32>,
    ) -> Result<Self, Error> {
        if depth != 8 && !(depth == 10 && matches!(codec, EncoderCodec::Hevc)) {
            return Err(Error(
                "hardware encoder supports H264 8-bit or HEVC 8/10-bit".into(),
            ));
        }
        if width == 0 || height == 0 || width > i32::MAX as usize || height > i32::MAX as usize {
            return Err(Error("invalid hardware encoder dimensions".into()));
        }
        let mut output = Box::new(Mutex::new(Output::default()));
        unsafe {
            let keys = [kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder];
            let values = [kCFBooleanTrue];
            let specification = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            if specification.is_null() {
                return Err(Error("encoder specification allocation failed".into()));
            }
            let mut session = ptr::null();
            let code = VTCompressionSessionCreate(
                ptr::null(),
                width as i32,
                height as i32,
                match codec {
                    EncoderCodec::H264 => u32::from_be_bytes(*b"avc1"),
                    EncoderCodec::Hevc => u32::from_be_bytes(*b"hvc1"),
                },
                specification,
                ptr::null(),
                ptr::null(),
                Some(self::output),
                (&mut *output as *mut Mutex<Output>).cast(),
                &mut session,
            );
            CFRelease(specification);
            if code != 0 || session.is_null() {
                if !session.is_null() {
                    VTCompressionSessionInvalidate(session);
                    CFRelease(session);
                }
                return Err(status("encoder creation", code));
            }
            let encoder = Self {
                session,
                output,
                size: [width, height],
                failed: false,
                colour: None,
                started: false,
                codec,
                depth,
            };
            if depth == 10 {
                let code = VTSessionSetProperty(
                    session,
                    kVTCompressionPropertyKey_ProfileLevel,
                    kVTProfileLevel_HEVC_Main10_AutoLevel,
                );
                if code != 0 {
                    return Err(status("Main10 encoder profile", code));
                }
            }
            let code = VTSessionSetProperty(
                session,
                kVTCompressionPropertyKey_AllowFrameReordering,
                kCFBooleanFalse,
            );
            if code != 0 {
                return Err(status("encoder frame reordering", code));
            }
            if let Some(value) = bitrate {
                let number =
                    CFNumberCreate(ptr::null(), CF_NUMBER_SINT32, (&value as *const i32).cast());
                if number.is_null() {
                    return Err(Error("encoder bitrate allocation failed".into()));
                }
                let code =
                    VTSessionSetProperty(session, kVTCompressionPropertyKey_AverageBitRate, number);
                CFRelease(number);
                if code != 0 {
                    return Err(status("encoder bitrate", code));
                }
            }
            if matches!(codec, EncoderCodec::Hevc) {
                let code = VTSessionSetProperty(
                    session,
                    kVTCompressionPropertyKey_HDRMetadataInsertionMode,
                    kVTHDRMetadataInsertionMode_None,
                );
                if code != 0 {
                    return Err(status("encoder HDR metadata mode", code));
                }
            }
            let code = VTCompressionSessionPrepareToEncodeFrames(session);
            if code != 0 {
                return Err(status("encoder preparation", code));
            }
            let mut hardware = ptr::null();
            let code = VTSessionCopyProperty(
                session,
                kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                ptr::null(),
                &mut hardware,
            );
            let confirmed = code == 0 && !hardware.is_null() && CFBooleanGetValue(hardware) != 0;
            if !hardware.is_null() {
                CFRelease(hardware);
            }
            if !confirmed {
                return Err(Error(
                    "encoder did not confirm hardware acceleration".into(),
                ));
            }
            Ok(encoder)
        }
    }
    /// Submit the native image and wait for its compressed output. Timestamps
    /// are expressed in `timescale` units. No pixel lock, copy or conversion.
    pub fn encode(
        &mut self,
        surface: &Surface,
        pts: i64,
        duration: i64,
        timescale: i32,
    ) -> Result<EncodedFrame, Error> {
        self.encode_with_keyframe(surface, pts, duration, timescale, false)
    }
    /// Request a random-access point, for example at an export segment boundary.
    pub fn encode_with_keyframe(
        &mut self,
        surface: &Surface,
        pts: i64,
        duration: i64,
        timescale: i32,
        force_keyframe: bool,
    ) -> Result<EncodedFrame, Error> {
        if self.failed {
            return Err(Error(
                "hardware encoder stopped after a previous failure".into(),
            ));
        }
        if [surface.width(), surface.height()] != self.size
            || surface.depth() != self.depth
            || timescale <= 0
            || duration <= 0
        {
            return Err(Error(
                "invalid hardware encoder frame geometry or timing".into(),
            ));
        }
        self.failed = true;
        self.started = true;
        *self
            .output
            .lock()
            .map_err(|_| Error("encoder callback poisoned".into()))? = Output::default();
        let time = |value| CMTime {
            value,
            timescale,
            flags: 1,
            epoch: 0,
        };
        unsafe {
            // A separate CVPixelBuffer shares the same IOSurface pixels but
            // owns signal attachments. Never mutate a caller's immutable Surface.
            let input = if let Some([primaries, transfer, matrix]) = self.colour {
                let backing = CVPixelBufferGetIOSurface(surface.0.image);
                if backing.is_null() {
                    return Err(Error("colour encoding requires IOSurface input".into()));
                }
                let mut image = ptr::null();
                let code =
                    CVPixelBufferCreateWithIOSurface(ptr::null(), backing, ptr::null(), &mut image);
                if code != 0 || image.is_null() {
                    if !image.is_null() {
                        CFRelease(image);
                    }
                    return Err(status("encoder signal surface", code));
                }
                let buffer = Sample(image);
                CVBufferSetAttachment(
                    image,
                    kCVImageBufferColorPrimariesKey,
                    if primaries == 1 {
                        kCVImageBufferColorPrimaries_ITU_R_709_2
                    } else {
                        kCVImageBufferColorPrimaries_ITU_R_2020
                    },
                    1,
                );
                CVBufferSetAttachment(
                    image,
                    kCVImageBufferTransferFunctionKey,
                    match transfer {
                        16 => kCVImageBufferTransferFunction_SMPTE_ST_2084_PQ,
                        18 => kCVImageBufferTransferFunction_ITU_R_2100_HLG,
                        _ => kCVImageBufferTransferFunction_ITU_R_709_2,
                    },
                    1,
                );
                CVBufferSetAttachment(
                    image,
                    kCVImageBufferYCbCrMatrixKey,
                    if matrix == 1 {
                        kCVImageBufferYCbCrMatrix_ITU_R_709_2
                    } else {
                        kCVImageBufferYCbCrMatrix_ITU_R_2020
                    },
                    1,
                );
                Some(buffer)
            } else {
                None
            };
            let properties = if force_keyframe {
                CFDictionaryCreate(
                    ptr::null(),
                    [kVTEncodeFrameOptionKey_ForceKeyFrame].as_ptr(),
                    [kCFBooleanTrue].as_ptr(),
                    1,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                )
            } else {
                ptr::null()
            };
            if force_keyframe && properties.is_null() {
                return Err(Error("encoder frame properties allocation failed".into()));
            }
            let code = VTCompressionSessionEncodeFrame(
                self.session,
                input.as_ref().map_or(surface.0.image, |image| image.0),
                time(pts),
                time(duration),
                properties,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if !properties.is_null() {
                CFRelease(properties);
            }
            if code != 0 {
                return Err(status("encode frame", code));
            }
            let code = VTCompressionSessionCompleteFrames(
                self.session,
                CMTime {
                    value: 0,
                    timescale: 0,
                    flags: 0,
                    epoch: 0,
                },
            );
            if code != 0 {
                return Err(status("complete encoded frame", code));
            }
        }
        let mut output = self
            .output
            .lock()
            .map_err(|_| Error("encoder callback poisoned".into()))?;
        if let Some(code) = output.error.take() {
            return Err(status("encoder callback", code));
        }
        let sample = output
            .sample
            .take()
            .ok_or_else(|| Error("encoder produced no frame".into()))?;
        unsafe {
            let buffer = CMSampleBufferGetDataBuffer(sample.0);
            if buffer.is_null() {
                return Err(Error("encoder produced no data buffer".into()));
            }
            let length = CMBlockBufferGetDataLength(buffer);
            if length == 0 || length > 64 * 1024 * 1024 {
                return Err(Error(
                    "encoded frame exceeds 64 MiB bound or is empty".into(),
                ));
            }
            let mut data = vec![0; length];
            let code = CMBlockBufferCopyDataBytes(buffer, 0, length, data.as_mut_ptr().cast());
            if code != 0 {
                return Err(status("copy compressed bytes", code));
            }
            let format = CMSampleBufferGetFormatDescription(sample.0);
            if format.is_null() {
                return Err(Error("encoded frame has no format description".into()));
            }
            let get = match self.codec {
                EncoderCodec::H264 => CMVideoFormatDescriptionGetH264ParameterSetAtIndex,
                EncoderCodec::Hevc => CMVideoFormatDescriptionGetHEVCParameterSetAtIndex,
            };
            let mut count = 0;
            let mut prefix = 0;
            let code = get(
                format,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut count,
                &mut prefix,
            );
            if code != 0 {
                return Err(status("encoder parameter sets", code));
            }
            if !(1..=16).contains(&count) || ![1, 2, 4].contains(&prefix) {
                return Err(Error("unsupported encoder configuration".into()));
            }
            let mut parameter_sets = Vec::with_capacity(count);
            for index in 0..count {
                let mut bytes = ptr::null();
                let mut length = 0;
                let code = get(
                    format,
                    index,
                    &mut bytes,
                    &mut length,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                if code != 0 {
                    return Err(status("encoder parameter set", code));
                }
                if bytes.is_null() || length == 0 || length > 1024 * 1024 {
                    return Err(Error("invalid encoder parameter set size".into()));
                }
                parameter_sets.push(std::slice::from_raw_parts(bytes, length).to_vec());
            }
            let extensions = CMFormatDescriptionGetExtensions(format);
            if extensions.is_null() {
                return Err(Error("encoder format has no extensions".into()));
            }
            let atoms = CFDictionaryGetValue(
                extensions,
                kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms,
            );
            if atoms.is_null() {
                return Err(Error("encoder format has no configuration atoms".into()));
            }
            let key = cfstr(match self.codec {
                EncoderCodec::H264 => "avcC",
                EncoderCodec::Hevc => "hvcC",
            });
            if key.is_null() {
                return Err(Error("encoder configuration key allocation failed".into()));
            }
            let value = CFDictionaryGetValue(atoms, key);
            CFRelease(key);
            if value.is_null() || CFGetTypeID(value) != CFDataGetTypeID() {
                return Err(Error("encoder configuration atom is not data".into()));
            }
            let length = CFDataGetLength(value);
            let bytes = CFDataGetBytePtr(value);
            if !(1..=1024 * 1024).contains(&length) || bytes.is_null() {
                return Err(Error("invalid encoder configuration size".into()));
            }
            let decoder_configuration = std::slice::from_raw_parts(bytes, length as usize).to_vec();
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample.0, 0);
            let key_frame = if attachments.is_null() || CFArrayGetCount(attachments) == 0 {
                true
            } else {
                let dictionary = CFArrayGetValueAtIndex(attachments, 0);
                let not_sync = CFDictionaryGetValue(dictionary, kCMSampleAttachmentKey_NotSync);
                not_sync.is_null() || CFBooleanGetValue(not_sync) == 0
            };
            let pts = EncodedTime::from_core(CMSampleBufferGetPresentationTimeStamp(sample.0))?;
            let duration = EncodedTime::from_core(CMSampleBufferGetDuration(sample.0))?;
            if duration.value <= 0 {
                return Err(Error("encoder returned non-positive duration".into()));
            }
            self.failed = false;
            Ok(EncodedFrame {
                data,
                parameter_sets,
                nal_length_size: prefix as u8,
                decoder_configuration,
                key_frame,
                pts,
                duration,
            })
        }
    }
}

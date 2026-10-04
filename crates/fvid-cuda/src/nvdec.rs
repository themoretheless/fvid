/*
 * This copyright notice applies to this header file only:
 *
 * Copyright (c) 2010-2018 NVIDIA Corporation
 *
 * Permission is hereby granted, free of charge, to any person
 * obtaining a copy of this software and associated documentation
 * files (the "Software"), to deal in the Software without
 * restriction, including without limitation the rights to use,
 * copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the software, and to permit persons to whom the
 * software is furnished to do so, subject to the following
 * conditions:
 *
 * The above copyright notice and this permission notice shall be
 * included in all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
 * EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
 * OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
 * NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
 * HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
 * WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
 * OTHER DEALINGS IN THE SOFTWARE.
 */

//! Direct NVDEC capability query, without libav or a linked CUDA SDK.
//! ABI: NVIDIA video-sdk-samples@aa3544dcea2fe63122e4feb83bf805ea40e58dbe,
//! Samples/NvCodec/NvDecoder/cuviddec.h (SDK 8.1 capability layout).
use crate::CodecDevice;

/// Codec identifiers available in the pinned NVDEC header.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvdecCodec {
    H264 = 4,
    Hevc = 8,
    Vp8 = 9,
    Vp9 = 10,
}
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NvdecChroma {
    Monochrome = 0,
    Yuv420 = 1,
    Yuv422 = 2,
    Yuv444 = 3,
}
#[cfg(any(test, target_os = "linux", target_os = "windows"))]
#[repr(C)]
#[derive(Default)]
struct RawCaps {
    codec: u32,
    chroma: u32,
    bit_depth_minus8: u32,
    reserved1: [u32; 3],
    supported: u8,
    reserved2: [u8; 3],
    max_width: u32,
    max_height: u32,
    max_macroblocks: u32,
    min_width: u16,
    min_height: u16,
    reserved3: [u32; 11],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NvdecCaps {
    pub supported: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub max_macroblocks: u32,
    pub min_width: u16,
    pub min_height: u16,
}
impl NvdecCaps {
    /// Check coded dimensions before allocating decoder surfaces.
    /// This does not prove that a particular bitstream can be decoded.
    pub fn accepts_geometry(self, width: u32, height: u32) -> bool {
        let macroblocks = u64::from(width).div_ceil(16) * u64::from(height).div_ceil(16);
        self.supported
            && width != 0
            && height != 0
            && width >= u32::from(self.min_width)
            && height >= u32::from(self.min_height)
            && width <= self.max_width
            && height <= self.max_height
            && macroblocks <= u64::from(self.max_macroblocks)
    }
}
/// Keeps the driver library alive. No decoder is created by loading it.
pub struct NvdecApi {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    library: libloading::Library,
}
impl NvdecApi {
    pub fn load() -> Result<Self, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            #[cfg(target_os = "linux")]
            let name = "libnvcuvid.so.1";
            #[cfg(target_os = "windows")]
            let name = "nvcuvid.dll";
            // SAFETY: Fixed NVIDIA driver name, not a caller-supplied path.
            let library = unsafe { libloading::Library::new(name) }
                .map_err(|e| format!("NVDEC driver library {name} is unavailable: {e}"))?;
            Ok(Self { library })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err("NVDEC requires an NVIDIA driver on Linux or Windows".into())
        }
    }
    /// Query the selected device with its CUDA context bound to this thread.
    pub fn capabilities(
        &self,
        device: &CodecDevice,
        codec: NvdecCodec,
        chroma: NvdecChroma,
        bit_depth: u8,
    ) -> Result<NvdecCaps, String> {
        validate_depth(bit_depth)?;
        device.handles()?;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            // SAFETY: The pinned CUDAAPI signature uses system calling convention,
            // writable CUVIDDECODECAPS storage and a four-byte CUresult.
            let query = unsafe {
                self.library
                    .get::<unsafe extern "system" fn(*mut RawCaps) -> i32>(
                        b"cuvidGetDecoderCaps\0".as_slice(),
                    )
            }
            .map_err(|e| format!("NVDEC capability entrypoint is unavailable: {e}"))?;
            query_caps(codec, chroma, bit_depth, |caps| {
                // SAFETY: caps has the verified SDK layout and zero reserves;
                // context is bound and the library remains live for this call.
                unsafe { query(caps) }
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (codec, chroma);
            Err("NVDEC requires an NVIDIA driver on Linux or Windows".into())
        }
    }
}
fn validate_depth(depth: u8) -> Result<(), String> {
    if matches!(depth, 8 | 10 | 12) {
        Ok(())
    } else {
        Err("NVDEC bit depth must be 8, 10 or 12".into())
    }
}
#[cfg(any(test, target_os = "linux", target_os = "windows"))]
fn query_caps(
    codec: NvdecCodec,
    chroma: NvdecChroma,
    depth: u8,
    query: impl FnOnce(&mut RawCaps) -> i32,
) -> Result<NvdecCaps, String> {
    validate_depth(depth)?;
    let mut raw = RawCaps {
        codec: codec as u32,
        chroma: chroma as u32,
        bit_depth_minus8: u32::from(depth - 8),
        ..RawCaps::default()
    };
    let status = query(&mut raw);
    if status != 0 {
        return Err(format!(
            "NVDEC capability query failed with CUDA status {status}"
        ));
    }
    if raw.supported > 1 {
        return Err("NVDEC returned an invalid support flag".into());
    }
    let caps = NvdecCaps {
        supported: raw.supported == 1,
        max_width: raw.max_width,
        max_height: raw.max_height,
        max_macroblocks: raw.max_macroblocks,
        min_width: raw.min_width,
        min_height: raw.min_height,
    };
    if caps.supported
        && (caps.max_width == 0
            || caps.max_height == 0
            || caps.max_macroblocks == 0
            || u32::from(caps.min_width) > caps.max_width
            || u32::from(caps.min_height) > caps.max_height)
    {
        return Err("NVDEC returned inconsistent supported geometry limits".into());
    }
    Ok(caps)
}

type DecoderHandle = *mut std::ffi::c_void;
type CreateDecoder = unsafe extern "system" fn(*mut DecoderHandle, *mut CreateInfo) -> i32;
type DestroyDecoder = unsafe extern "system" fn(DecoderHandle) -> i32;
#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}
#[repr(C)]
#[derive(Default)]
struct CreateInfo {
    width: std::ffi::c_ulong,
    height: std::ffi::c_ulong,
    decode_surfaces: std::ffi::c_ulong,
    codec: u32,
    chroma: u32,
    flags: std::ffi::c_ulong,
    depth_minus8: std::ffi::c_ulong,
    intra_only: std::ffi::c_ulong,
    max_width: std::ffi::c_ulong,
    max_height: std::ffi::c_ulong,
    reserved1: std::ffi::c_ulong,
    display: Rect,
    output_format: u32,
    deinterlace: u32,
    target_width: std::ffi::c_ulong,
    target_height: std::ffi::c_ulong,
    output_surfaces: std::ffi::c_ulong,
    context_lock: *mut std::ffi::c_void,
    target: Rect,
    reserved2: [std::ffi::c_ulong; 5],
}
/// Owns a decoder allocation. Packet parsing and picture submission are pending.
pub struct NvdecSession {
    decoder: DecoderHandle,
    destroy: DestroyDecoder,
    device: std::mem::ManuallyDrop<CodecDevice>,
    api: std::mem::ManuallyDrop<NvdecApi>,
}
impl NvdecSession {
    /// Create 4:2:0 decoding with reference count supplied by the sequence parser.
    pub fn open(
        device: CodecDevice,
        codec: NvdecCodec,
        depth: u8,
        width: u32,
        height: u32,
        decode_surfaces: u32,
        output_surfaces: u32,
    ) -> Result<Self, String> {
        let mut params = create_info(
            codec,
            depth,
            width,
            height,
            decode_surfaces,
            output_surfaces,
        )?;
        let api = NvdecApi::load()?;
        if !api
            .capabilities(&device, codec, NvdecChroma::Yuv420, depth)?
            .accepts_geometry(width, height)
        {
            return Err("NVDEC device does not support requested coded geometry/format".into());
        }
        let (create, destroy) = api.decoder_entrypoints()?;
        let mut session = Self {
            decoder: std::ptr::null_mut(),
            destroy,
            device: std::mem::ManuallyDrop::new(device),
            api: std::mem::ManuallyDrop::new(api),
        };
        // SAFETY: Verified writable SDK storage, bound context and retained owners.
        let status = unsafe { create(&mut session.decoder, &mut params) };
        if status != 0 {
            return Err(format!(
                "NVDEC decoder creation failed with CUDA status {status}"
            ));
        }
        if session.decoder.is_null() {
            return Err("NVDEC returned a null decoder".into());
        }
        Ok(session)
    }
    /// Failed destruction preserves the handle for retry.
    pub fn close(&mut self) -> Result<(), String> {
        if self.decoder.is_null() {
            return Ok(());
        }
        self.device.handles()?;
        self.device.synchronize()?;
        release_decoder(&mut self.decoder, |handle| {
            // SAFETY: Live decoder, bound context and retained driver library.
            unsafe { (self.destroy)(handle) }
        })
    }
}
impl Drop for NvdecSession {
    fn drop(&mut self) {
        if self.close().is_ok() {
            // SAFETY: Decoder closed; owners released exactly once.
            unsafe {
                std::mem::ManuallyDrop::drop(&mut self.api);
                std::mem::ManuallyDrop::drop(&mut self.device);
            }
        }
        // A failed final cleanup intentionally retains library/context.
    }
}
impl NvdecApi {
    fn decoder_entrypoints(&self) -> Result<(CreateDecoder, DestroyDecoder), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let create_name = [b"cuvidCreateDecoder".as_slice(), &[0]].concat();
            let destroy_name = [b"cuvidDestroyDecoder".as_slice(), &[0]].concat();
            // SAFETY: Fixed CUDAAPI symbol and C-verified storage layout.
            let create = unsafe { self.library.get::<CreateDecoder>(create_name.as_slice()) }
                .map_err(|e| format!("NVDEC create entrypoint unavailable: {e}"))?;
            // SAFETY: Fixed CUDAAPI destructor with retained library.
            let destroy = unsafe { self.library.get::<DestroyDecoder>(destroy_name.as_slice()) }
                .map_err(|e| format!("NVDEC destroy entrypoint unavailable: {e}"))?;
            Ok((*create, *destroy))
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err("NVDEC requires an NVIDIA driver on Linux or Windows".into())
        }
    }
}
fn create_info(
    codec: NvdecCodec,
    depth: u8,
    width: u32,
    height: u32,
    decode_surfaces: u32,
    output_surfaces: u32,
) -> Result<CreateInfo, String> {
    validate_depth(depth)?;
    if width == 0
        || height == 0
        || width % 2 != 0
        || height % 2 != 0
        || width > i16::MAX as u32
        || height > i16::MAX as u32
    {
        return Err("NVDEC 4:2:0 dimensions must be nonzero, even and fit SDK rectangles".into());
    }
    if decode_surfaces == 0 || decode_surfaces > 64 || output_surfaces == 0 || output_surfaces > 64
    {
        return Err("NVDEC surface counts must be between 1 and 64".into());
    }
    Ok(CreateInfo {
        width: width.into(),
        height: height.into(),
        decode_surfaces: decode_surfaces.into(),
        output_surfaces: output_surfaces.into(),
        codec: codec as u32,
        chroma: NvdecChroma::Yuv420 as u32,
        flags: 4,
        depth_minus8: (depth - 8).into(),
        max_width: width.into(),
        max_height: height.into(),
        display: Rect {
            right: width as i16,
            bottom: height as i16,
            ..Rect::default()
        },
        target_width: width.into(),
        target_height: height.into(),
        output_format: u32::from(depth > 8),
        ..CreateInfo::default()
    })
}
fn release_decoder(
    handle: &mut DecoderHandle,
    destroy: impl FnOnce(DecoderHandle) -> i32,
) -> Result<(), String> {
    if handle.is_null() {
        return Ok(());
    }
    let status = destroy(*handle);
    if status != 0 {
        return Err(format!(
            "NVDEC decoder destruction failed with CUDA status {status}"
        ));
    }
    *handle = std::ptr::null_mut();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_storage_matches_linux_and_windows_c_layouts() {
        if std::mem::size_of::<usize>() != 8 {
            return;
        }
        let expected = if std::mem::size_of::<std::ffi::c_ulong>() == 8 {
            (176, 80, 88, 120, 136)
        } else {
            (112, 44, 52, 72, 88)
        };
        assert_eq!(
            (
                std::mem::size_of::<CreateInfo>(),
                std::mem::offset_of!(CreateInfo, display),
                std::mem::offset_of!(CreateInfo, output_format),
                std::mem::offset_of!(CreateInfo, context_lock),
                std::mem::offset_of!(CreateInfo, reserved2)
            ),
            expected
        );
    }
    #[test]
    fn creation_request_checks_surface_counts_and_preserves_10_bit_format() {
        let r = create_info(NvdecCodec::Hevc, 10, 1920, 1080, 20, 4).unwrap();
        assert_eq!(
            (r.codec, r.chroma, r.output_format, r.depth_minus8),
            (8, 1, 1, 2)
        );
        assert_eq!((r.decode_surfaces, r.output_surfaces), (20, 4));
        assert_eq!(r.flags, 4);
        assert!(r.context_lock.is_null());
        assert_eq!(r.reserved2, [0; 5]);
        assert!(create_info(NvdecCodec::H264, 8, 1919, 1080, 20, 4).is_err());
        assert!(create_info(NvdecCodec::H264, 8, 1920, 1080, 0, 4).is_err());
        assert!(create_info(NvdecCodec::H264, 8, 1920, 1080, 20, 65).is_err());
    }
    #[test]
    fn destruction_keeps_failed_handle_for_retry() {
        let original = std::ptr::dangling_mut::<std::ffi::c_void>();
        let mut handle = original;
        assert!(release_decoder(&mut handle, |_| 999).is_err());
        assert_eq!(handle, original);
        release_decoder(&mut handle, |h| {
            assert_eq!(h, original);
            0
        })
        .unwrap();
        assert!(handle.is_null());
        release_decoder(&mut handle, |_| panic!("duplicate destruction")).unwrap();
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn direct_decoder_allocation_without_libav() {
        let mut session = NvdecSession::open(
            CodecDevice::new(0).unwrap(),
            NvdecCodec::H264,
            8,
            1920,
            1080,
            20,
            4,
        )
        .unwrap();
        session.close().unwrap();
        session.close().unwrap();
    }
    #[test]
    fn capability_storage_matches_pinned_c_header() {
        assert_eq!(std::mem::size_of::<RawCaps>(), 88);
        assert_eq!(std::mem::align_of::<RawCaps>(), 4);
        assert_eq!(std::mem::offset_of!(RawCaps, supported), 24);
        assert_eq!(std::mem::offset_of!(RawCaps, max_width), 28);
        assert_eq!(std::mem::offset_of!(RawCaps, min_width), 40);
        assert_eq!(std::mem::offset_of!(RawCaps, reserved3), 44);
    }
    #[test]
    fn query_initializes_inputs_and_refuses_errors_and_inconsistent_limits() {
        let caps = query_caps(NvdecCodec::Hevc, NvdecChroma::Yuv420, 10, |r| {
            assert_eq!((r.codec, r.chroma, r.bit_depth_minus8), (8, 1, 2));
            assert_eq!(r.reserved1, [0; 3]);
            assert_eq!(r.reserved2, [0; 3]);
            assert_eq!(r.reserved3, [0; 11]);
            r.supported = 1;
            r.max_width = 4096;
            r.max_height = 4096;
            r.max_macroblocks = 32768;
            r.min_width = 64;
            r.min_height = 64;
            0
        })
        .unwrap();
        assert!(caps.accepts_geometry(1920, 1080));
        assert!(!caps.accepts_geometry(4096, 4096));
        assert!(!caps.accepts_geometry(0, 1080));
        assert!(!caps.accepts_geometry(32, 64));
        assert!(!caps.accepts_geometry(u32::MAX, u32::MAX));
        assert!(query_caps(NvdecCodec::H264, NvdecChroma::Yuv420, 7, |_| panic!()).is_err());
        assert!(query_caps(NvdecCodec::H264, NvdecChroma::Yuv420, 8, |_| 999).is_err());
        assert!(
            query_caps(NvdecCodec::H264, NvdecChroma::Yuv420, 8, |r| {
                r.supported = 1;
                0
            })
            .is_err()
        );
        assert!(
            query_caps(NvdecCodec::H264, NvdecChroma::Yuv420, 8, |r| {
                r.supported = 2;
                0
            })
            .is_err()
        );
    }
    #[test]
    fn unsupported_capability_is_a_result_not_a_fake_decoder() {
        let caps = query_caps(NvdecCodec::Vp9, NvdecChroma::Yuv444, 12, |_| 0).unwrap();
        assert!(!caps.supported);
        assert!(!caps.accepts_geometry(1920, 1080));
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    #[test]
    fn unsupported_host_does_not_load_nvdec() {
        assert!(NvdecApi::load().is_err());
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn direct_capabilities_without_libav() {
        let device = CodecDevice::new(0).unwrap();
        let api = NvdecApi::load().unwrap();
        let caps = api
            .capabilities(&device, NvdecCodec::H264, NvdecChroma::Yuv420, 8)
            .unwrap();
        assert!(caps.accepts_geometry(1920, 1080));
    }
}

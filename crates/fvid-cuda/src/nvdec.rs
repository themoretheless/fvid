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
#[cfg(test)]
mod tests {
    use super::*;
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

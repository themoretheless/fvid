//! Format-preserving owned CUDA 4:2:0 buffers and processors.
use crate::{
    ByteShader, Nv12Buffer, Nv12Processor, Nv12Transform, Nv12View, P010Buffer, P010Processor,
    P010View,
};
pub enum Yuv420Buffer {
    Nv12(Nv12Buffer),
    P010(P010Buffer),
}
#[derive(Clone, Copy, Debug)]
pub enum Yuv420View {
    Nv12(Nv12View),
    P010(P010View),
}
pub enum Yuv420Processor {
    Nv12(Nv12Processor),
    P010(P010Processor),
}
macro_rules! buffer_dispatch {
    ($self:expr,$v:ident,$call:expr) => {
        match $self {
            Yuv420Buffer::Nv12($v) => $call,
            Yuv420Buffer::P010($v) => $call,
        }
    };
}
macro_rules! processor_dispatch {
    ($self:expr,$v:ident,$call:expr) => {
        match $self {
            Yuv420Processor::Nv12($v) => $call,
            Yuv420Processor::P010($v) => $call,
        }
    };
}
impl Yuv420Buffer {
    pub fn new(ordinal: usize, width: u32, height: u32, depth: u8) -> Result<Self, String> {
        match depth {
            8 => Nv12Buffer::new(ordinal, width, height).map(Self::Nv12),
            10 => P010Buffer::new(ordinal, width, height).map(Self::P010),
            _ => Err("owned CUDA 4:2:0 buffers require eight or ten bits".into()),
        }
    }
    pub fn bit_depth(&self) -> u8 {
        match self {
            Self::Nv12(_) => 8,
            Self::P010(_) => 10,
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        buffer_dispatch!(self, b, b.dimensions())
    }
    pub fn pitch(&self) -> u32 {
        buffer_dispatch!(self, b, b.pitch())
    }
    pub fn byte_len(&self) -> usize {
        buffer_dispatch!(self, b, b.byte_len())
    }
    pub fn stream_handle(&self) -> Result<u64, String> {
        buffer_dispatch!(self, b, b.stream_handle())
    }
    pub fn synchronize(&self) -> Result<(), String> {
        buffer_dispatch!(self, b, b.synchronize())
    }
    pub fn fill_black(&mut self, full_range: bool) -> Result<(), String> {
        buffer_dispatch!(self, b, b.fill_black(full_range))
    }
    pub fn view(&self) -> Result<Yuv420View, String> {
        match self {
            Self::Nv12(b) => b.view().map(Yuv420View::Nv12),
            Self::P010(b) => b.view().map(Yuv420View::P010),
        }
    }
    pub fn nv12_view(&self) -> Result<Nv12View, String> {
        match self {
            Self::Nv12(b) => b.view(),
            Self::P010(_) => Err("P010 cannot be borrowed as NV12".into()),
        }
    }
}
impl Yuv420View {
    /// Borrow a contiguous decoder surface. Caller must retain its mapping and
    /// complete device work before unmap; this view is not an allocation owner.
    pub fn from_contiguous(
        pointer: u64,
        pitch: u32,
        width: u32,
        height: u32,
        depth: u8,
    ) -> Result<Self, String> {
        let bytes = match depth {
            8 => 1,
            10 => 2,
            _ => return Err("unsupported CUDA 4:2:0 depth".into()),
        };
        let row = width.checked_mul(bytes).ok_or("CUDA row overflow")?;
        let y_bytes = u64::from(pitch) * u64::from(height);
        let total = y_bytes
            .checked_add(y_bytes / 2)
            .ok_or("CUDA plane extent overflow")?;
        if pointer == 0
            || width == 0
            || height == 0
            || width % 2 != 0
            || height % 2 != 0
            || pitch < row
            || pitch % bytes != 0
            || pointer % u64::from(bytes) != 0
            || pointer.checked_add(total).is_none()
        {
            return Err("invalid CUDA 4:2:0 surface geometry/alignment/extent".into());
        }
        let uv = pointer
            .checked_add(y_bytes)
            .ok_or("CUDA UV pointer overflow")?;
        if depth == 8 {
            Ok(Self::Nv12(Nv12View {
                y: pointer,
                uv,
                pitch_y: pitch,
                pitch_uv: pitch,
                width,
                height,
            }))
        } else {
            Ok(Self::P010(P010View {
                y: pointer,
                uv,
                pitch_y: pitch,
                pitch_uv: pitch,
                width,
                height,
            }))
        }
    }
}
impl Yuv420Processor {
    pub fn new(ordinal: usize, depth: u8, shader: Option<&ByteShader>) -> Result<Self, String> {
        match (depth, shader) {
            (8, None) => Nv12Processor::new(ordinal).map(Self::Nv12),
            (8, Some(shader)) => Nv12Processor::with_shader(ordinal, shader).map(Self::Nv12),
            (10, None) => P010Processor::new(ordinal).map(Self::P010),
            (10, Some(shader)) => P010Processor::with_shader(ordinal, shader).map(Self::P010),
            _ => Err("owned CUDA processor requires eight or ten bits".into()),
        }
    }
    pub fn device_name(&self) -> &str {
        processor_dispatch!(self, p, p.device_name())
    }
    pub fn follow_stream(&mut self, stream: u64) {
        processor_dispatch!(self, p, p.follow_stream(stream))
    }
    pub fn synchronize(&self) -> Result<(), String> {
        processor_dispatch!(self, p, p.synchronize())
    }
    pub fn apply(
        &mut self,
        src: Yuv420View,
        dst: Yuv420View,
        t: Nv12Transform,
    ) -> Result<(), String> {
        match (self, src, dst) {
            (Self::Nv12(p), Yuv420View::Nv12(s), Yuv420View::Nv12(d)) => p.apply(s, d, t),
            (Self::P010(p), Yuv420View::P010(s), Yuv420View::P010(d)) => p.apply(s, d, t),
            _ => Err("CUDA 4:2:0 processor and surface formats differ".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borrowed_decoder_views_preserve_depth_and_checked_byte_geometry() {
        let Yuv420View::P010(view) = Yuv420View::from_contiguous(4096, 512, 130, 72, 10).unwrap()
        else {
            panic!("wrong format")
        };
        assert_eq!(view.uv, 4096 + 512 * 72);
        assert_eq!(view.width, 130);
        assert!(Yuv420View::from_contiguous(4096, 256, 130, 72, 10).is_err());
        assert!(Yuv420View::from_contiguous(4097, 512, 130, 72, 10).is_err());
        assert!(Yuv420View::from_contiguous(u64::MAX - 100, 512, 130, 72, 10).is_err());
        assert!(Yuv420View::from_contiguous(4096, 512, 130, 72, 12).is_err());
        assert!(matches!(
            Yuv420View::from_contiguous(4096, 256, 130, 72, 8).unwrap(),
            Yuv420View::Nv12(_)
        ));
    }
}

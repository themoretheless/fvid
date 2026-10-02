//! Owned names and tightly packed storage for supported pixel representations.
macro_rules! formats {
    ($( $variant:ident, $name:literal, $chroma:expr, $bytes:expr; )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum PixelFormat { $( $variant, )* }
        impl PixelFormat {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
            pub fn name(self) -> &'static str { match self { $( Self::$variant => $name, )* } }
            pub(crate) fn nul_name(self) -> &'static [u8] { match self { $( Self::$variant => concat!($name, "\0").as_bytes(), )* } }
            pub fn frame_bytes(self, width: usize, height: usize) -> Result<usize, String> {
                match self { $( Self::$variant => crate::owned_frame_layout::planar_bytes(width, height, $chroma, $bytes), )* }
            }
        }
    };
}
formats! {
    Yuv420p, "yuv420p", Some((1, 1)), 1;
    Yuv422p, "yuv422p", Some((1, 0)), 1;
    Yuv444p, "yuv444p", Some((0, 0)), 1;
    Gray8, "gray", None, 1;
    Rgb24, "rgb24", None, 3;
    Bgr24, "bgr24", None, 3;
    Rgba, "rgba", None, 4;
    Bgra, "bgra", None, 4;
    Nv12, "nv12", Some((1, 1)), 1;
    Nv21, "nv21", Some((1, 1)), 1;
    Yuv420p9Le, "yuv420p9le", Some((1, 1)), 2;
    Yuv420p9Be, "yuv420p9be", Some((1, 1)), 2;
    Yuv420p10Le, "yuv420p10le", Some((1, 1)), 2;
    Yuv420p10Be, "yuv420p10be", Some((1, 1)), 2;
    Yuv420p12Le, "yuv420p12le", Some((1, 1)), 2;
    Yuv420p12Be, "yuv420p12be", Some((1, 1)), 2;
    Yuv420p14Le, "yuv420p14le", Some((1, 1)), 2;
    Yuv420p14Be, "yuv420p14be", Some((1, 1)), 2;
    Yuv420p16Le, "yuv420p16le", Some((1, 1)), 2;
    Yuv420p16Be, "yuv420p16be", Some((1, 1)), 2;
    Yuv422p9Le, "yuv422p9le", Some((1, 0)), 2;
    Yuv422p9Be, "yuv422p9be", Some((1, 0)), 2;
    Yuv422p10Le, "yuv422p10le", Some((1, 0)), 2;
    Yuv422p10Be, "yuv422p10be", Some((1, 0)), 2;
    Yuv422p12Le, "yuv422p12le", Some((1, 0)), 2;
    Yuv422p12Be, "yuv422p12be", Some((1, 0)), 2;
    Yuv422p14Le, "yuv422p14le", Some((1, 0)), 2;
    Yuv422p14Be, "yuv422p14be", Some((1, 0)), 2;
    Yuv422p16Le, "yuv422p16le", Some((1, 0)), 2;
    Yuv422p16Be, "yuv422p16be", Some((1, 0)), 2;
    Yuv444p9Le, "yuv444p9le", Some((0, 0)), 2;
    Yuv444p9Be, "yuv444p9be", Some((0, 0)), 2;
    Yuv444p10Le, "yuv444p10le", Some((0, 0)), 2;
    Yuv444p10Be, "yuv444p10be", Some((0, 0)), 2;
    Yuv444p12Le, "yuv444p12le", Some((0, 0)), 2;
    Yuv444p12Be, "yuv444p12be", Some((0, 0)), 2;
    Yuv444p14Le, "yuv444p14le", Some((0, 0)), 2;
    Yuv444p14Be, "yuv444p14be", Some((0, 0)), 2;
    Yuv444p16Le, "yuv444p16le", Some((0, 0)), 2;
    Yuv444p16Be, "yuv444p16be", Some((0, 0)), 2;
    Gray16Le, "gray16le", None, 2;
    Gray16Be, "gray16be", None, 2;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_storage_handles_odd_chroma_and_high_depth() {
        for format in PixelFormat::ALL {
            assert!(format.frame_bytes(0, 1).is_err());
            assert!(format.frame_bytes(usize::MAX, 2).is_err());
        }
        assert_eq!(PixelFormat::Yuv420p.frame_bytes(3, 3).unwrap(), 17);
        assert_eq!(PixelFormat::Yuv422p14Be.frame_bytes(3, 3).unwrap(), 42);
        assert_eq!(PixelFormat::Yuv444p9Le.frame_bytes(3, 3).unwrap(), 54);
        assert_eq!(
            PixelFormat::Nv12.frame_bytes(3, 3),
            PixelFormat::Yuv420p.frame_bytes(3, 3)
        );
        assert_eq!(PixelFormat::Rgba.frame_bytes(3, 3).unwrap(), 36);
    }
}

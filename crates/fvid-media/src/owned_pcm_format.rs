//! PCM representation metadata owned by FVid, independent of backend enums.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    U8,
    I16,
    I32,
    I64,
    F32,
    F64,
}
impl Encoding {
    pub fn sample_bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I16 => 2,
            Self::I32 | Self::F32 => 4,
            Self::I64 | Self::F64 => 8,
        }
    }
    pub fn name(self, planar: bool) -> &'static str {
        match (self, planar) {
            (Self::U8, false) => "u8",
            (Self::U8, true) => "u8p",
            (Self::I16, false) => "s16",
            (Self::I16, true) => "s16p",
            (Self::I32, false) => "s32",
            (Self::I32, true) => "s32p",
            (Self::I64, false) => "s64",
            (Self::I64, true) => "s64p",
            (Self::F32, false) => "flt",
            (Self::F32, true) => "fltp",
            (Self::F64, false) => "dbl",
            (Self::F64, true) => "dblp",
        }
    }
    pub fn frame_bytes(self, channels: usize) -> Result<usize, String> {
        if !(1..=64).contains(&channels) {
            return Err("invalid PCM channel count".into());
        }
        Ok(self.sample_bytes() * channels)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_formats_have_explicit_sizes_names_and_checked_frame_geometry() {
        for (format, bytes, packed, planar) in [
            (Encoding::U8, 1, "u8", "u8p"),
            (Encoding::I16, 2, "s16", "s16p"),
            (Encoding::I32, 4, "s32", "s32p"),
            (Encoding::I64, 8, "s64", "s64p"),
            (Encoding::F32, 4, "flt", "fltp"),
            (Encoding::F64, 8, "dbl", "dblp"),
        ] {
            assert_eq!(format.sample_bytes(), bytes);
            assert_eq!(format.name(false), packed);
            assert_eq!(format.name(true), planar);
            assert_eq!(format.frame_bytes(64).unwrap(), bytes * 64);
            assert!(format.frame_bytes(0).is_err());
            assert!(format.frame_bytes(65).is_err());
        }
    }
}

//! Standard speaker masks used by owned PCM rematrixing.
/// FL/FR/FC, quad with BC, 5.0/5.1 back, 6.1 and 7.1.
pub fn standard_mask(channels: u16) -> Option<u64> {
    match channels {
        1 => Some(0x4),
        2 => Some(0x3),
        3 => Some(0x7),
        4 => Some(0x107),
        5 => Some(0x37),
        6 => Some(0x3f),
        7 => Some(0x70f),
        8 => Some(0x63f),
        _ => None,
    }
}

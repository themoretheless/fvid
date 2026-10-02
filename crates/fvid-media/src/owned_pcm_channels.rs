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

/// Conventional default output layout, distinct from explicit rematrix layouts.
/// In particular three unspecified output channels default to FL/FR/LFE (2.1),
/// while `standard_mask(3)` intentionally describes FL/FR/FC (3.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultLayout {
    pub channels: u16,
    pub mask: u64,
    pub name: &'static str,
}
pub fn default_layout(channels: u16) -> Option<DefaultLayout> {
    let (mask, name) = match channels {
        1 => (0x4, "mono"),
        2 => (0x3, "stereo"),
        3 => (0xb, "2.1"),
        4 => (0x107, "4.0"),
        5 => (0x37, "5.0"),
        6 => (0x3f, "5.1"),
        7 => (0x70f, "6.1"),
        8 => (0x63f, "7.1"),
        10 => (0x2d60f, "5.1.4"),
        12 => (0x2d63f, "7.1.4"),
        14 => (0x2d6ff, "9.1.4"),
        16 => (0x300002d6ff, "9.1.6"),
        24 => (0x1f80003ffff, "22.2"),
        _ => return None,
    };
    Some(DefaultLayout {
        channels,
        mask,
        name,
    })
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn unspecified_three_channels_retain_lfe_without_changing_explicit_surround() {
        let layout = default_layout(3).unwrap();
        assert_eq!(layout.mask, (1 << 0) | (1 << 1) | (1 << 3));
        assert_eq!(layout.name, "2.1");
        assert_eq!(standard_mask(3), Some((1 << 0) | (1 << 1) | (1 << 2)));
        for channels in [1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 14, 16, 24] {
            let layout = default_layout(channels).unwrap();
            assert_eq!(layout.mask.count_ones(), u32::from(layout.channels));
        }
        assert!(default_layout(0).is_none());
        assert!(default_layout(9).is_none());
    }
}

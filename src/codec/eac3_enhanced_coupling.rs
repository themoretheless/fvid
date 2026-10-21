//! Owned Annex E.2.5 enhanced coupling: band geometry and amplitude expansion.
use super::{Bits, BlockStrategy, CPL, Core, FBW, Header};
const SUB: [usize; 23] = [
    13, 19, 25, 31, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145, 157, 169, 181, 193, 205, 217, 229,
    241, 253,
];
const DEFAULT: [bool; 22] = [
    false, false, false, false, false, false, false, false, false, true, false, true, false, true,
    false, true, true, true, false, true, true, true,
];
#[derive(Clone)]
pub(super) struct Enhanced {
    pub active: bool,
    pub begin_code: usize,
    begin: usize,
    end: usize,
    merged: [bool; 22],
    gains: [[f32; 22]; FBW],
}
impl Default for Enhanced {
    fn default() -> Self {
        Self {
            active: false,
            begin_code: 0,
            begin: 0,
            end: 0,
            merged: DEFAULT,
            gains: [[0.; 22]; FBW],
        }
    }
}
impl Enhanced {
    pub fn open(
        &mut self,
        h: Header,
        s: &mut BlockStrategy,
        bits: &mut Bits<'_>,
        spx_begin: Option<usize>,
        continued: bool,
        was_enhanced: bool,
    ) -> bool {
        let code = bits.take(4) as usize;
        let begin = if code < 3 {
            code * 2
        } else if code < 13 {
            code + 2
        } else {
            code * 2 - 10
        };
        let end = spx_begin
            .map(|v| if v < 6 { v + 5 } else { v * 2 })
            .unwrap_or_else(|| bits.take(4) as usize + 7);
        if begin >= end || end > 22 || s.incpl[..h.nfchans].iter().filter(|&&v| v).count() < 2 {
            return false;
        }
        s.cpl_opened = !continued;
        s.cpl_moved = continued && (s.start[CPL], s.end[CPL]) != (SUB[begin], SUB[end]);
        // Switching coupling tools does not make an already participating
        // channel owe implicit coordinates again. Its explicit update flag
        // remains present in the stream. Geometry, not the tool, gates reuse.
        if bits.flag() {
            self.merged = DEFAULT;
            for sb in (begin + 1).max(9)..end {
                self.merged[sb] = bits.flag();
            }
        } else if !was_enhanced || !continued {
            self.merged = DEFAULT;
        }
        // The first active sub-band always begins its own band.
        self.merged[begin] = false;
        self.begin_code = code;
        self.begin = begin;
        self.end = end;
        s.cplinu = true;
        s.phsflginu = false;
        true
    }
    fn bands(&self) -> usize {
        (self.begin..self.end).filter(|&b| !self.merged[b]).count()
    }
    pub fn coordinates(&mut self, h: Header, s: &mut BlockStrategy, bits: &mut Bits<'_>) -> bool {
        bits.skip(1); // reserved before enhanced coordinates
        let bands = self.bands();
        let mut first = None;
        for ch in 0..h.nfchans {
            if !s.incpl[ch] {
                s.firstcplcos[ch] = true;
                continue;
            }
            let later = first.is_some();
            first.get_or_insert(ch);
            let owed = s.firstcplcos[ch];
            s.firstcplcos[ch] = false;
            let update = owed || bits.flag();
            let reserved = if owed { later } else { later && bits.flag() };
            if update {
                for band in 0..bands {
                    self.gains[ch][band] = amplitude(bits.take(5) as usize);
                }
            } else if s.cpl_opened || s.cpl_moved {
                return false;
            }
            if reserved {
                bits.skip(9 * (bands - 1));
            }
            if later {
                bits.skip(1);
            }
        }
        !bits.overrun
    }
    pub fn measure(&self, h: Header, s: &mut BlockStrategy) {
        for ch in 0..h.nfchans {
            if s.incpl[ch] {
                s.end[ch] = SUB[self.begin];
            }
        }
        s.start[CPL] = SUB[self.begin];
        s.end[CPL] = SUB[self.end];
    }
    pub fn rematrix(&self, s: &mut BlockStrategy) {
        let count = match self.begin_code {
            0 => 0,
            1 => 1,
            2 => 2,
            3 | 4 => 3,
            _ => 4,
        };
        let mut bands = [(13, 24), (25, 36), (37, 60), (61, 252)];
        if count > 0 {
            bands[count - 1].1 = SUB[self.begin] - 1;
        }
        if s.rematrix.count != count || s.rematrix.bands != bands {
            s.rematrix.flags = [false; 4];
        }
        s.rematrix.bands = bands;
        s.rematrix.count = count;
    }
    pub fn restore(&self, h: Header, core: &mut Core) {
        for ch in 0..h.nfchans {
            if !core.strategy.incpl[ch] {
                continue;
            }
            let mut band = 0;
            for sb in self.begin..self.end {
                if sb > self.begin && !self.merged[sb] {
                    band += 1;
                }
                for i in SUB[sb]..SUB[sb + 1] {
                    let value = core.coupled_mantissa(ch, i) * self.gains[ch][band];
                    core.coefficients(ch)[i] = value;
                }
            }
        }
    }
}
fn amplitude(code: usize) -> f32 {
    if code == 31 {
        0.
    } else if code == 0 {
        1.
    } else {
        [27., 23., 19., 16.][(code - 1) % 4] / (32. * (1u32 << ((code - 1) / 4)) as f32)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_normative_amplitudes_include_unity_and_silence() {
        let expected_q12 = [
            4096, 3456, 2944, 2432, 2048, 1728, 1472, 1216, 1024, 864, 736, 608, 512, 432, 368,
            304, 256, 216, 184, 152, 128, 108, 92, 76, 64, 54, 46, 38, 32, 27, 23, 0,
        ];
        for (code, expected) in expected_q12.into_iter().enumerate() {
            assert_eq!(
                super::amplitude(code),
                expected as f32 / 4096.0,
                "amplitude {code}"
            );
        }
    }
}

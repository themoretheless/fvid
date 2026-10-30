// Shared exact edit/normalization clock; wrappers supply Error, Edit and Result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Time {
    num: i128,
    den: u128,
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}
fn overflow() -> Error {
    invalid("MP4 audio edit clock overflow")
}
impl Time {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub fn new(num: i128, den: u128) -> Result<Self> {
        if den == 0 {
            return Err(invalid("zero MP4 edit clock"));
        }
        if num == 0 {
            return Ok(Self::ZERO);
        }
        let g = gcd(num.unsigned_abs(), den);
        Ok(Self {
            num: num / i128::try_from(g).map_err(|_| overflow())?,
            den: den / g,
        })
    }
    pub fn ticks(num: u64, scale: u32) -> Result<Self> {
        Self::new(i128::from(num), u128::from(scale))
    }
    pub fn signed_ticks(num: i64, scale: u32) -> Result<Self> {
        Self::new(i128::from(num), u128::from(scale))
    }
    pub fn add(self, rhs: Self) -> Result<Self> {
        let g = gcd(self.den, rhs.den);
        let a = rhs.den / g;
        let b = self.den / g;
        let num = self
            .num
            .checked_mul(i128::try_from(a).map_err(|_| overflow())?)
            .and_then(|v| {
                rhs.num
                    .checked_mul(i128::try_from(b).ok()?)
                    .and_then(|w| v.checked_add(w))
            })
            .ok_or_else(overflow)?;
        Self::new(num, self.den.checked_mul(a).ok_or_else(overflow)?)
    }
    pub fn sub(self, rhs: Self) -> Result<Self> {
        self.add(Self {
            num: rhs.num.checked_neg().ok_or_else(overflow)?,
            den: rhs.den,
        })
    }
    pub fn cmp(self, rhs: Self) -> Ordering {
        match (self.num < 0, rhs.num < 0) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                let mut a = self.num.unsigned_abs();
                let mut b = self.den;
                let mut c = rhs.num.unsigned_abs();
                let mut d = rhs.den;
                let mut flip = self.num < 0;
                loop {
                    let order = (a / b).cmp(&(c / d));
                    if order != Ordering::Equal {
                        return if flip { order.reverse() } else { order };
                    }
                    let (r, s) = (a % b, c % d);
                    if r == 0 || s == 0 {
                        let order = r.cmp(&s);
                        return if flip { order.reverse() } else { order };
                    }
                    a = b;
                    b = r;
                    c = d;
                    d = s;
                    flip = !flip;
                }
            }
        }
    }
    pub fn max(self, rhs: Self) -> Self {
        if self.cmp(rhs) == Ordering::Less {
            rhs
        } else {
            self
        }
    }
    /// Multiply a proper fraction by a 32-bit rate without overflowing u128.
    fn fraction_frames(remainder: u128, den: u128, rate: u32) -> (u128, bool) {
        let mut q = 0u128;
        let mut r = 0u128;
        for bit in (0..32).rev() {
            q *= 2;
            if r >= den - r {
                r -= den - r;
                q += 1;
            } else {
                r *= 2;
            }
            if (rate >> bit) & 1 != 0 {
                if r >= den - remainder {
                    r -= den - remainder;
                    q += 1;
                } else {
                    r += remainder;
                }
            }
        }
        (q, r != 0)
    }
    fn scaled(self, rate: u32, ceil: bool) -> Result<i128> {
        let n = self.num.unsigned_abs();
        let (fraction, remaining) = Self::fraction_frames(n % self.den, self.den, rate);
        let mut magnitude = (n / self.den)
            .checked_mul(u128::from(rate))
            .and_then(|v| v.checked_add(fraction))
            .ok_or_else(overflow)?;
        if remaining && (ceil != (self.num < 0)) {
            magnitude = magnitude.checked_add(1).ok_or_else(overflow)?;
        }
        let signed = i128::try_from(magnitude).map_err(|_| overflow())?;
        Ok(if self.num < 0 { -signed } else { signed })
    }
    pub fn ceil_frames(self, rate: u32) -> Result<i128> {
        self.scaled(rate, true)
    }
    pub fn floor_ticks(self, scale: u32) -> Result<i128> {
        self.scaled(scale, false)
    }
    /// Integer output-grid origin plus its exact proper fractional phase.
    #[allow(dead_code)] // The playback instantiation retains source clocks without resampling.
    pub fn output_origin(self, rate: u32) -> Result<(i128, (u128, u128))> {
        let floor = self.floor_ticks(rate)?;
        let remainder = self.sub(Self::new(floor, u128::from(rate))?)?;
        let g = gcd(remainder.den, u128::from(rate));
        let num = remainder
            .num
            .unsigned_abs()
            .checked_mul(u128::from(rate) / g)
            .ok_or_else(overflow)?;
        let den = remainder.den / g;
        if remainder.num < 0 || num >= den {
            return Err(overflow());
        }
        Ok((floor, (num, den)))
    }
    pub fn duration(self) -> Option<Duration> {
        if self.num < 0 {
            return None;
        }
        let seconds = self.num.unsigned_abs() / self.den;
        let (ns, _) =
            Self::fraction_frames(self.num.unsigned_abs() % self.den, self.den, 1_000_000_000);
        Some(Duration::new(u64::try_from(seconds).ok()?, ns as u32))
    }
    /// Preserve an exact packet timebase when it fits; otherwise round only the
    /// emitted timestamp to nanoseconds, retaining an exact internal cursor.
    pub fn stamp(self) -> Result<(u64, u32)> {
        if self.num < 0 {
            return Err(invalid("negative MP4 edit presentation time"));
        }
        if self.den <= u128::from(u32::MAX) && self.num <= i128::from(u64::MAX) {
            return Ok((self.num as u64, self.den as u32));
        }
        let ticks = self.floor_ticks(1_000_000_000)?;
        Ok((u64::try_from(ticks).map_err(|_| overflow())?, 1_000_000_000))
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Segment {
    pub start: Time,
    pub end: Time,
    pub media: Option<Time>,
}
pub(crate) struct Timeline {
    pub segments: Vec<Segment>,
    pub end: Time,
    pub cursor: std::cell::Cell<Time>,
    pub active: Option<usize>,
    pub minimum_starts: Vec<Time>,
}
impl Timeline {
    pub fn new(edits: &[Edit], media_scale: u32, movie_scale: u32) -> Result<Self> {
        if media_scale == 0 || movie_scale == 0 {
            return Err(invalid("zero MP4 audio edit clock"));
        }
        let mut segments = Vec::new();
        segments
            .try_reserve_exact(edits.len())
            .map_err(|_| invalid("MP4 edit clock allocation failed"))?;
        let mut ticks = 0u128;
        let mut start = Time::ZERO;
        for edit in edits {
            ticks = ticks
                .checked_add(u128::from(edit.duration))
                .ok_or_else(overflow)?;
            let end = Time::new(
                i128::try_from(ticks).map_err(|_| overflow())?,
                u128::from(movie_scale),
            )?;
            let media = match edit.media_time {
                -1 => None,
                n if n >= 0 => Some(Time::signed_ticks(n, media_scale)?),
                _ => return Err(invalid("invalid MP4 audio edit time")),
            };
            segments.push(Segment { start, end, media });
            start = end;
        }
        Ok(Self {
            segments,
            end: start,
            cursor: std::cell::Cell::new(Time::ZERO),
            active: None,
            minimum_starts: Vec::new(),
        })
    }
    #[allow(dead_code)] // Playback marks packet-free source tails before scheduling.
    pub fn declare_source_silence(&mut self, from: Time, to: Time) -> Result<()> {
        let mut split = Vec::new();
        split.try_reserve_exact(self.segments.len().checked_mul(3).ok_or_else(overflow)?)
            .map_err(|_| invalid("MP4 edit silence allocation failed"))?;
        for segment in &self.segments {
            let Some(media) = segment.media else { split.push(*segment); continue; };
            let source_end = media.add(segment.end.sub(segment.start)?)?;
            let begin = from.max(media);
            let end = if to.cmp(source_end) == Ordering::Less { to } else { source_end };
            if begin.cmp(end) != Ordering::Less { split.push(*segment); continue; }
            let a = segment.start.add(begin.sub(media)?)?;
            let b = segment.start.add(end.sub(media)?)?;
            if segment.start.cmp(a) == Ordering::Less {
                split.push(Segment { start: segment.start, end: a, media: Some(media) });
            }
            split.push(Segment { start: a, end: b, media: None });
            if b.cmp(segment.end) == Ordering::Less {
                split.push(Segment { start: b, end: segment.end, media: Some(end) });
            }
        }
        self.segments = split;
        Ok(())
    }
    pub fn locate(&self) -> Option<usize> {
        let at = self
            .segments
            .partition_point(|s| s.end.cmp(self.cursor.get()) != Ordering::Greater);
        (at < self.segments.len()).then_some(at)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_phase_and_signed_preroll_survive_fractional_container_clocks() {
        let p = Time::signed_ticks(-5000, 1_000_000).unwrap();
        assert_eq!(p.ceil_frames(48000).unwrap(), -240);
        let t = Time::ticks(95000, 1_000_000).unwrap();
        assert_eq!(
            t.add(Time::ticks(221, 44100).unwrap())
                .unwrap()
                .sub(t)
                .unwrap(),
            Time::ticks(221, 44100).unwrap()
        );
        assert_eq!(t.ceil_frames(44100).unwrap(), 4190);
        assert_eq!(t.floor_ticks(44100).unwrap(), 4189);
    }
    #[test]
    fn comparing_and_scaling_large_fractions_does_not_multiply_denominators() {
        let a = Time::new(1, u128::MAX - 2).unwrap();
        let b = Time::new(1, u128::MAX - 4).unwrap();
        assert_eq!(a.cmp(b), Ordering::Less);
        assert_eq!(a.ceil_frames(u32::MAX).unwrap(), 1);
        assert_eq!(a.floor_ticks(u32::MAX).unwrap(), 0);
    }
    #[test]
    fn normalization_origins_keep_signed_fractional_output_phase() {
        assert_eq!(
            Time::new(1, 7).unwrap().output_origin(48000).unwrap(),
            (6857, (1, 7))
        );
        assert_eq!(
            Time::new(-1, 7).unwrap().output_origin(48000).unwrap(),
            (-6858, (6, 7))
        );
        let at = Time::ticks(997, 48000)
            .unwrap()
            .add(Time::ticks(641, 32000).unwrap())
            .unwrap();
        assert_eq!(at.output_origin(48000).unwrap(), (1958, (1, 2)));
    }
}

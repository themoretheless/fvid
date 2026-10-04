//! Constant-memory selection of one frame per N input frames.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStep(u32);
impl FrameStep {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 128 || args.contains('\0') {
            return Err("invalid framestep options".into());
        }
        if args.is_empty() {
            return Ok(Self(1));
        }
        let mut step = 1;
        let mut positional = false;
        for option in args.split(':') {
            let value = if let Some(value) = option.strip_prefix("step=") {
                positional = true;
                value
            } else if !positional && !option.contains('=') {
                positional = true;
                option
            } else {
                return Err("unsupported framestep option".into());
            };
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("unsupported framestep step expression".into());
            }
            step = value
                .parse::<u32>()
                .map_err(|_| "framestep step overflow")?;
            if step == 0 || step > i32::MAX as u32 {
                return Err("framestep step must be 1..=2147483647".into());
            }
        }
        Ok(Self(step))
    }
    pub fn emits(self, input_index: u64) -> bool {
        input_index % u64::from(self.0) == 0
    }
    pub(crate) fn frame_rate(self, [n, d]: [i32; 2]) -> Result<[i32; 2]> {
        let (mut a, mut b) = (n as u64, d as u64 * u64::from(self.0));
        let (mut x, mut y) = (a, b);
        while y != 0 {
            (x, y) = (y, x % y);
        }
        a /= x;
        b /= x;
        Ok([
            i32::try_from(a).map_err(|_| "framestep frame rate overflow")?,
            i32::try_from(b).map_err(|_| "framestep frame rate overflow")?,
        ])
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn decimal_options_and_clock_reduction() {
        use super::FrameStep;
        for args in ["", "1", "step=1"] {
            assert!(FrameStep::parse(args).unwrap().emits(123));
        }
        for args in ["2", "step=2", "1:step=2", "step=1:step=2"] {
            let step = FrameStep::parse(args).unwrap();
            assert!(step.emits(0));
            assert!(!step.emits(1));
            assert!(step.emits(2));
            assert_eq!(step.frame_rate([4, 1]).unwrap(), [2, 1]);
        }
        for args in [
            "0",
            "-1",
            "2147483648",
            "2:3",
            "step=2:3",
            "step=",
            "x=2",
            "2.0",
            "1+1",
        ] {
            assert!(FrameStep::parse(args).is_err(), "{args}");
        }
        assert_eq!(
            FrameStep::parse("2147483647")
                .unwrap()
                .frame_rate([2147483647, 1])
                .unwrap(),
            [1, 1]
        );
        assert!(
            FrameStep::parse("2147483647")
                .unwrap()
                .frame_rate([1, 2147483647])
                .is_err()
        );
    }
}

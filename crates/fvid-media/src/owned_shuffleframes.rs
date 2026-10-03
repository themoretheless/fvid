//! Bounded-group temporal reordering; no external filter backend.
type Result<T> = std::result::Result<T, String>;
#[derive(Default)]
struct Frame {
    data: Vec<u8>,
    pts: u64,
    duration: u64,
}
pub(crate) struct ShuffleFrames {
    mapping: Vec<i32>,
    frames: Vec<Frame>,
    filled: usize,
}
impl ShuffleFrames {
    pub(crate) fn parse(args: &str) -> Result<Self> {
        if args.len() > 128 || args.contains('\0') {
            return Err("invalid shuffleframes options".into());
        }
        let map = if args.is_empty() {
            "0"
        } else {
            args.strip_prefix("mapping=").unwrap_or(args)
        };
        let mut mapping = Vec::new();
        for field in map.split([' ', '|']) {
            if field.is_empty() || field.bytes().any(|b| !b.is_ascii_digit() && b != b'-') {
                return Err("unsupported shuffleframes mapping".into());
            }
            mapping.push(
                field
                    .parse::<i32>()
                    .map_err(|_| "invalid shuffleframes index")?,
            );
        }
        if mapping.iter().any(|&n| n < -1 || n >= mapping.len() as i32) {
            return Err("shuffleframes index outside its group".into());
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(mapping.len())
            .map_err(|e| e.to_string())?;
        frames.resize_with(mapping.len(), Frame::default);
        Ok(Self {
            mapping,
            frames,
            filled: 0,
        })
    }
    pub(crate) fn push(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
        emit: &mut impl FnMut(&[u8], u64, u64) -> Result<()>,
    ) -> Result<u64> {
        if self.mapping == [0] {
            emit(data, pts, duration)?;
            return Ok(1);
        }
        let frame = &mut self.frames[self.filled];
        frame.data.clear();
        frame
            .data
            .try_reserve_exact(data.len())
            .map_err(|e| e.to_string())?;
        frame.data.extend_from_slice(data);
        frame.pts = pts;
        frame.duration = duration;
        self.filled += 1;
        if self.filled < self.mapping.len() {
            return Ok(0);
        }
        let mut emitted = 0;
        for (position, &source) in self.mapping.iter().enumerate() {
            if source < 0 {
                continue;
            }
            let frame = &self.frames[source as usize];
            emit(&frame.data, self.frames[position].pts, frame.duration)?;
            emitted += 1;
        }
        self.filled = 0;
        Ok(emitted)
    }
    // A partial terminal group is discarded; there is no flush output.
}
#[cfg(test)]
mod tests {
    #[test]
    fn maps_duplicates_drops_and_source_duration_without_emitting_partial_groups() {
        use super::ShuffleFrames;
        let mut filter = ShuffleFrames::parse("mapping=2|-1|2").unwrap();
        let mut output = Vec::new();
        for i in 0..7u8 {
            filter
                .push(
                    &[i],
                    u64::from(i) * 10,
                    u64::from(i) + 1,
                    &mut |data, pts, duration| {
                        output.push((data[0], pts, duration));
                        Ok(())
                    },
                )
                .unwrap();
        }
        assert_eq!(output, [(2, 0, 3), (2, 20, 3), (5, 30, 6), (5, 50, 6)]);
        for args in [
            "3 1 0", "-2 0", "0||1", " 0", "0 ", "x=0", "mapping=", "0:1",
        ] {
            assert!(ShuffleFrames::parse(args).is_err(), "{args}");
        }
    }
}

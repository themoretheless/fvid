// RFC 9043 sections 3.8.2 and 3.8.1.1.1. No implicit payload padding.
#[derive(Clone, Copy)]
struct RiceModel {
    count: i32,
    sum: i32,
    drift: i32,
    bias: i32,
}
impl Default for RiceModel {
    fn default() -> Self {
        Self {
            count: 1,
            sum: 4,
            drift: 0,
            bias: 0,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct RiceRun {
    index: usize,
    mode: u8,
    left: usize,
}
impl RiceRun {
    fn line(&mut self) {
        self.mode = 0;
        self.left = 0;
    }
}
const LOG2_RUN: [u8; 41] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13,
    14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
];
struct Rice<'a> {
    data: &'a [u8],
    at: usize,
}
impl Rice<'_> {
    fn bits(&mut self, count: u8) -> Result<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = *self
                .data
                .get(self.at / 8)
                .ok_or_else(|| invalid("truncated FFV1 Golomb Rice payload"))?;
            value = (value << 1) | u32::from((byte >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Ok(value)
    }
    fn scalar(&mut self, model: &mut RiceModel, bits: u8) -> Result<i32> {
        let mut k = 0;
        let mut scale = model.count;
        while scale < model.sum {
            scale *= 2;
            k += 1;
        }
        let mut unsigned = None;
        for prefix in 0..12 {
            if self.bits(1)? != 0 {
                unsigned = Some((prefix << k) + self.bits(k)?);
                break;
            }
        }
        let unsigned = if let Some(value) = unsigned {
            value
        } else {
            let value = self.bits(bits)? + 11;
            if value < 12u32 << k {
                return Err(invalid("noncanonical FFV1 Golomb Rice escape"));
            }
            value
        };
        let mut v = if unsigned & 1 == 0 {
            (unsigned >> 1) as i32
        } else {
            -((unsigned >> 1) as i32) - 1
        };
        if 2 * model.drift < -model.count {
            v = -1 - v;
        }
        let mask = (1i32 << bits) - 1;
        let value = (v + model.bias) & mask;
        let result = if value & (1 << (bits - 1)) != 0 {
            value - (1 << bits)
        } else {
            value
        };
        model.sum += v.abs();
        model.drift += v;
        if model.count == 128 {
            model.count >>= 1;
            model.sum >>= 1;
            model.drift >>= 1;
        }
        model.count += 1;
        if model.drift <= -model.count {
            model.bias = (model.bias - 1).max(-128);
            model.drift = (model.drift + model.count).max(-model.count + 1);
        } else if model.drift > 0 {
            model.bias = (model.bias + 1).min(127);
            model.drift = (model.drift - model.count).min(0);
        }
        Ok(result)
    }
    fn sample(
        &mut self,
        models: &mut [RiceModel],
        context: i32,
        bits: u8,
        run: &mut RiceRun,
        x: usize,
        width: usize,
    ) -> Result<i32> {
        let model = models
            .get_mut(context.unsigned_abs() as usize)
            .ok_or_else(|| invalid("invalid FFV1 Rice context"))?;
        if run.mode == 0 && context == 0 {
            run.mode = 1;
        }
        if run.mode == 0 {
            return self.scalar(model, bits);
        }
        if run.left == 0 && run.mode == 1 {
            let log = *LOG2_RUN
                .get(run.index)
                .ok_or_else(|| invalid("invalid FFV1 run index"))?;
            if self.bits(1)? != 0 {
                run.left = 1usize << log;
                if run.left <= width - x {
                    run.index += 1;
                }
            } else {
                run.left = self.bits(log)? as usize;
                run.index = run.index.saturating_sub(1);
                run.mode = 2;
            }
        }
        if run.left != 0 {
            run.left -= 1;
            return Ok(0);
        }
        run.mode = 0;
        let value = self.scalar(model, bits)?;
        Ok(if value >= 0 { value + 1 } else { value })
    }
    fn finish(&mut self) -> Result<()> {
        while self.at % 8 != 0 {
            if self.bits(1)? != 0 {
                return Err(invalid("nonzero FFV1 Golomb Rice padding"));
            }
        }
        // Remaining full bytes are reserved in versions 0/1, not sample data.
        Ok(())
    }
}

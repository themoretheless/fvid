const MAX_LINE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Yuv420,
    Yuv422,
    Yuv444,
}
impl PixelFormat {
    pub(crate) fn subsampling(self) -> (usize, usize) {
        match self {
            Self::Yuv420 => (2, 2),
            Self::Yuv422 => (2, 1),
            Self::Yuv444 => (1, 1),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Header {
    pub width: usize,
    pub height: usize,
    pub format: PixelFormat,
    pub(crate) tokens: Vec<String>,
}
impl Header {
    pub fn parse(line: &[u8]) -> Result<Self> {
        if !line.is_ascii() {
            return Err(invalid("Y4M header must be ASCII"));
        }
        let text = std::str::from_utf8(line).map_err(|_| invalid("Y4M header is not UTF-8"))?;
        let mut words = text.split_whitespace();
        if words.next() != Some("YUV4MPEG2") {
            return Err(invalid("expected YUV4MPEG2 input"));
        }
        let tokens: Vec<String> = words.map(str::to_owned).collect();
        let (mut width, mut height, mut format) = (None, None, None);
        for token in &tokens {
            let value = &token[1..];
            match token.as_bytes()[0] {
                b'W' => {
                    if width.is_some() {
                        return Err(invalid("duplicate width"));
                    }
                    width = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid width"))?,
                    );
                }
                b'H' => {
                    if height.is_some() {
                        return Err(invalid("duplicate height"));
                    }
                    height = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid height"))?,
                    );
                }
                b'C' => {
                    if format.is_some() {
                        return Err(invalid("duplicate pixel format"));
                    }
                    format = Some(match value {
                        "420" | "420jpeg" | "420mpeg2" | "420paldv" => PixelFormat::Yuv420,
                        "422" => PixelFormat::Yuv422,
                        "444" => PixelFormat::Yuv444,
                        _ => {
                            let (layout, depth) = value
                                .split_once('p')
                                .ok_or_else(|| invalid("unsupported Y4M pixel format"))?;
                            if !matches!(depth, "9" | "10" | "12" | "14" | "16") {
                                return Err(invalid("unsupported Y4M sample depth"));
                            }
                            match layout {
                                "420" => PixelFormat::Yuv420,
                                "422" => PixelFormat::Yuv422,
                                "444" => PixelFormat::Yuv444,
                                _ => return Err(invalid("unsupported Y4M chroma layout")),
                            }
                        }
                    });
                }
                b'I' if value != "p" && value != "?" => {
                    return Err(invalid("interlaced video is not supported"));
                }
                _ => {}
            }
        }
        let header = Self {
            width: width.ok_or_else(|| invalid("missing width"))?,
            height: height.ok_or_else(|| invalid("missing height"))?,
            format: format.unwrap_or(PixelFormat::Yuv420),
            tokens,
        };
        header.frame_len()?;
        Ok(header)
    }
    /// Validated, reduced frame rate; Y4M's omitted rate defaults to 25 fps.
    pub fn frame_rate(&self) -> Result<[i32; 2]> {
        let mut rate = None;
        for token in &self.tokens {
            if let Some(value) = token.strip_prefix('F') {
                if rate.is_some() {
                    return Err(invalid("duplicate Y4M frame rate"));
                }
                let (n, d) = value
                    .split_once(':')
                    .ok_or_else(|| invalid("invalid Y4M frame rate"))?;
                let n = n
                    .parse::<i32>()
                    .map_err(|_| invalid("invalid Y4M frame rate"))?;
                let d = d
                    .parse::<i32>()
                    .map_err(|_| invalid("invalid Y4M frame rate"))?;
                if n <= 0 || d <= 0 {
                    return Err(invalid("Y4M frame rate must be positive"));
                }
                rate = Some([n, d]);
            }
        }
        let [n, d] = rate.unwrap_or([25, 1]);
        let mut a = n;
        let mut b = d;
        while b != 0 {
            let r = a % b;
            a = b;
            b = r;
        }
        Ok([n / a, d / a])
    }
    pub fn depth(&self) -> u8 {
        self.tokens
            .iter()
            .find_map(|t| {
                t.strip_prefix('C')
                    .and_then(|v| v.split_once('p'))
                    .and_then(|(_, d)| d.parse().ok())
            })
            .unwrap_or(8)
    }
    pub fn frame_len(&self) -> Result<usize> {
        let (sx, sy) = self.format.subsampling();
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(sx)
            || !self.height.is_multiple_of(sy)
        {
            return Err(invalid("dimensions must be positive and chroma-aligned"));
        }
        let y = self
            .width
            .checked_mul(self.height)
            .ok_or_else(|| invalid("dimensions overflow"))?;
        y.checked_add(
            (y / sx / sy)
                .checked_mul(2)
                .ok_or_else(|| invalid("dimensions overflow"))?,
        )
        .and_then(|n| n.checked_mul(if self.depth() == 8 { 1 } else { 2 }))
        .ok_or_else(|| invalid("dimensions overflow"))
    }
    fn encode(&self, width: usize, height: usize) -> String {
        let mut s = format!("YUV4MPEG2 W{width} H{height}");
        for t in &self.tokens {
            if !t.starts_with('W') && !t.starts_with('H') {
                s.push(' ');
                s.push_str(t);
            }
        }
        s.push('\n');
        s
    }
}

/// Read a newline-terminated header without an unbounded allocation.
pub(crate) fn line<R: BufRead>(reader: &mut R, bytes: &mut Vec<u8>) -> Result<bool> {
    bytes.clear();
    loop {
        let available = reader.fill_buf().map_err(io_error)?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(false)
            } else {
                Err(invalid("truncated header line"))
            };
        }
        let n = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        if bytes.len() + n > MAX_LINE {
            return Err(invalid("header line exceeds 4096 bytes"));
        }
        let complete = available[n - 1] == b'\n';
        bytes.extend_from_slice(&available[..n]);
        reader.consume(n);
        if complete {
            return Ok(true);
        }
    }
}

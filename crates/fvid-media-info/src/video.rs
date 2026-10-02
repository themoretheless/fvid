//! Backend-independent video transformation requests and geometry.
//! Filter option strings preserve the public API; each backend validates support.
use std::path::PathBuf;
type Result<T> = std::result::Result<T, String>;

/// FFmpeg `transpose=` modes (all swap width/height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransposeMode {
    Clock,
    CClock,
    ClockFlip,
    CClockFlip,
}

impl TransposeMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "clock" => Ok(Self::Clock),
            "cclock" => Ok(Self::CClock),
            "clock_flip" => Ok(Self::ClockFlip),
            "cclock_flip" => Ok(Self::CClockFlip),
            _ => Err("transpose must be clock, cclock, clock_flip, or cclock_flip".into()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::CClock => "cclock",
            Self::ClockFlip => "clock_flip",
            Self::CClockFlip => "cclock_flip",
        }
    }

    /// Output size after a 90° transpose of `width`×`height`.
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        let _ = self;
        (height, width)
    }
}

/// FFmpeg-compatible `pad=W:H:X:Y:black` geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadRect {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
}

impl PadRect {
    pub fn validate(self, input_w: u32, input_h: u32) -> Result<()> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 4320
            || self.width < input_w
            || self.height < input_h
        {
            return Err(
                "pad size must contain the input and stay within 1..=8192 x 1..=4320".into(),
            );
        }
        if self.width % 2 != 0 || self.height % 2 != 0 || self.x % 2 != 0 || self.y % 2 != 0 {
            return Err("pad size and origin must be even for 4:2:0 chroma".into());
        }
        if self
            .x
            .checked_add(input_w)
            .is_none_or(|right| right > self.width)
            || self
                .y
                .checked_add(input_h)
                .is_none_or(|bottom| bottom > self.height)
        {
            return Err("pad offset must keep the input inside the output canvas".into());
        }
        Ok(())
    }
}

/// Arbitrary rotation in degrees; fair-pairs FFmpeg
/// `rotate=a=DEG*PI/180:ow=rotw(a):oh=roth(a):c=black`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateAngle {
    pub degrees: f64,
}

impl RotateAngle {
    pub fn parse(value: &str) -> Result<Self> {
        let degrees: f64 = value
            .parse()
            .map_err(|_| "rotate must be a number of degrees")?;
        Self::validate(degrees)?;
        Ok(Self { degrees })
    }

    fn validate(degrees: f64) -> Result<()> {
        if !degrees.is_finite() || !(-3600.0..=3600.0).contains(&degrees) {
            return Err("rotate degrees must be finite within -3600..=3600".into());
        }
        Ok(())
    }

    /// Output canvas size matching FFmpeg `rotw(a)` / `roth(a)` (trunc toward zero).
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        let a = self.degrees.to_radians();
        let (c, s) = (a.cos().abs(), a.sin().abs());
        let ow = ((width as f64) * c + (height as f64) * s) as u32;
        let oh = ((width as f64) * s + (height as f64) * c) as u32;
        (ow.max(1), oh.max(1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CropRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScaleSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlaySpec {
    pub path: PathBuf,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XfadeSpec {
    pub path: PathBuf,
    pub transition: String,
    pub duration_us: i64,
    pub offset_us: i64,
    /// Constant frame rate for xfade (from stream avg_frame_rate).
    pub fps_num: i32,
    pub fps_den: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LosslessTransform {
    pub crop: Option<CropRect>,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
    /// Exact output size after crop/flips/transpose/rotate/pad. Neighbor sampling matches
    /// FFmpeg `scale=W:H:flags=neighbor`.
    pub scale: Option<ScaleSize>,
    /// FFmpeg `epx=` pixel-art upscale after scale and before burn/overlay (e.g. `n=2`).
    pub epx: Option<String>,
    /// FFmpeg-compatible `transpose=` after crop/flips and before rotate/pad/scale.
    pub transpose: Option<TransposeMode>,
    /// FFmpeg-compatible `rotate=` after transpose and before pad/scale.
    pub rotate: Option<RotateAngle>,
    /// FFmpeg-compatible `pad=W:H:X:Y:black` after rotate and before scale.
    pub pad: Option<PadRect>,
    /// External text subtitle file burned via libavfilter `subtitles=` after geometry.
    pub burn_subs: Option<PathBuf>,
    /// External video composited via libavfilter `movie=` + `overlay=x:y` after burn-in.
    pub overlay: Option<OverlaySpec>,
    /// External video cross-faded via libavfilter `movie=` + `xfade=` (mutually exclusive with overlay).
    pub xfade: Option<XfadeSpec>,
    /// FFmpeg `yadif=` option string (e.g. `mode=0`); empty string uses defaults.
    pub yadif: Option<String>,
    /// FFmpeg `bwdif=` option string (e.g. `mode=0`); empty string uses defaults.
    pub bwdif: Option<String>,
    /// FFmpeg `w3fdif=` option string (e.g. `mode=0`); empty string uses defaults.
    pub w3fdif: Option<String>,
    /// FFmpeg `tblend=` option string (e.g. `all_mode=average`).
    pub tblend: Option<String>,
    /// FFmpeg `tmix=` option string (e.g. `frames=3`).
    pub tmix: Option<String>,
    /// FFmpeg `hqdn3d=` option string (e.g. `4:3:6:4.5`); empty string uses defaults.
    pub hqdn3d: Option<String>,
    /// FFmpeg `gblur=` option string (e.g. `sigma=1.5:steps=1`); empty uses defaults.
    pub gblur: Option<String>,
    /// FFmpeg `eq=` option string (e.g. `brightness=0.06:contrast=1.2`); empty uses defaults.
    pub eq: Option<String>,
    /// FFmpeg `unsharp=` option string (e.g. `5:5:1.0:5:5:0.0`); empty uses defaults.
    pub unsharp: Option<String>,
    /// FFmpeg `hue=` option string (e.g. `h=90:s=1.2`); empty uses defaults.
    pub hue: Option<String>,
    /// FFmpeg `avgblur=` option string (e.g. `sizeX=5:sizeY=5`); empty uses defaults.
    pub avgblur: Option<String>,
    /// FFmpeg `boxblur=` option string (e.g. `2:1`); empty uses defaults.
    pub boxblur: Option<String>,
    /// FFmpeg `negate` / `negate=1` (alpha); empty/`0` = components only.
    pub negate: Option<String>,
    /// FFmpeg `edgedetect=` option string (e.g. `mode=colormix`); empty uses defaults.
    pub edgedetect: Option<String>,
    /// FFmpeg `sobel=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub sobel: Option<String>,
    /// FFmpeg `prewitt=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub prewitt: Option<String>,
    /// FFmpeg `roberts=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub roberts: Option<String>,
    /// FFmpeg `kirsch=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub kirsch: Option<String>,
    /// FFmpeg `scharr=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub scharr: Option<String>,
    /// FFmpeg `atadenoise=` option string (e.g. `0a=0.02:0b=0.04`); empty uses defaults.
    pub atadenoise: Option<String>,
    /// FFmpeg `owdenoise=` option string (e.g. `depth=8:luma_strength=1.0`); empty uses defaults.
    pub owdenoise: Option<String>,
    /// FFmpeg `vaguedenoiser=` option string (e.g. `threshold=3`); empty uses defaults.
    pub vaguedenoiser: Option<String>,
    /// FFmpeg `nlmeans=` option string (e.g. `s=1.0`); empty uses defaults.
    pub nlmeans: Option<String>,
    /// FFmpeg `bm3d=` option string (e.g. `sigma=3`); empty uses defaults.
    pub bm3d: Option<String>,
    /// FFmpeg `dctdnoiz=` option string (e.g. `s=3`); empty uses defaults.
    pub dctdnoiz: Option<String>,
    /// FFmpeg `fftdnoiz=` option string (e.g. `sigma=1`); empty uses defaults.
    pub fftdnoiz: Option<String>,
    /// FFmpeg `smartblur=` option string (e.g. `lr=1.5:ls=-0.5`); empty uses defaults.
    pub smartblur: Option<String>,
    /// FFmpeg `sab=` option string (e.g. `lr=2:cr=2`); empty uses defaults.
    pub sab: Option<String>,
    /// FFmpeg `bilateral=` option string (e.g. `sigmaS=0.1:sigmaR=0.1`); empty uses defaults.
    pub bilateral: Option<String>,
    /// FFmpeg `cas=` option string (e.g. `strength=0.5`); empty uses defaults.
    pub cas: Option<String>,
    /// FFmpeg `vignette=` option string (e.g. `angle=PI/4`); empty uses defaults.
    pub vignette: Option<String>,
    /// FFmpeg `curves=` option string (e.g. `preset=vintage`); empty uses defaults.
    pub curves: Option<String>,
    /// FFmpeg `colorbalance=` option string (e.g. `rs=.1:gs=.05:bs=-.1`); empty uses defaults.
    pub colorbalance: Option<String>,
    /// FFmpeg `colorlevels=` option string (e.g. `rimin=0.1:gimin=0.1:bimin=0.1`); empty uses defaults.
    pub colorlevels: Option<String>,
    /// FFmpeg `colorchannelmixer=` option string (e.g. `rr=1.1:gg=0.9:bb=1.0`); empty uses defaults.
    pub colorchannelmixer: Option<String>,
    /// FFmpeg `deflicker=` option string (e.g. `mode=am:size=5`); empty uses defaults.
    pub deflicker: Option<String>,
    /// FFmpeg `photosensitivity=` option string (e.g. `f=5`); empty uses defaults.
    pub photosensitivity: Option<String>,
    /// FFmpeg `monochrome=` option string (e.g. `cb=0.2:cr=-0.1`); empty uses defaults.
    pub monochrome: Option<String>,
    /// FFmpeg `grayworld` / `grayworld=` (white balance); empty/`0` uses defaults.
    pub grayworld: Option<String>,
    /// FFmpeg `drawbox=` option string (e.g. `x=10:y=10:w=40:h=20:color=red`); empty uses defaults.
    pub drawbox: Option<String>,
    /// FFmpeg `drawgrid=` option string (e.g. `w=16:h=16:color=white`); empty uses defaults.
    pub drawgrid: Option<String>,
    /// FFmpeg `lagfun=` option string (e.g. `decay=0.95`); empty uses defaults.
    pub lagfun: Option<String>,
    /// FFmpeg `amplify=` option string (e.g. `radius=2:factor=2`); empty uses defaults.
    pub amplify: Option<String>,
    /// FFmpeg `bitplanenoise=` option string (e.g. `bitplane=1:filter=1`); empty uses defaults.
    pub bitplanenoise: Option<String>,
    /// FFmpeg `deband=` option string (e.g. `1thr=0.02`); empty uses defaults.
    pub deband: Option<String>,
    /// FFmpeg `gradfun=` option string (e.g. `strength=1.2`); empty uses defaults.
    pub gradfun: Option<String>,
    /// FFmpeg `lenscorrection=` option string (e.g. `k1=-0.1`); empty uses defaults.
    pub lenscorrection: Option<String>,
    /// FFmpeg `pixelize=` option string (e.g. `width=8:height=8`); empty uses defaults.
    pub pixelize: Option<String>,
    /// FFmpeg `removegrain=` option string (e.g. `m0=1`); empty uses defaults.
    pub removegrain: Option<String>,
    /// FFmpeg `yaepblur=` option string (e.g. `r=3`); empty uses defaults.
    pub yaepblur: Option<String>,
    /// FFmpeg `vibrance=` option string (e.g. `intensity=0.3:rbal=1`); empty uses defaults.
    pub vibrance: Option<String>,
    /// FFmpeg `dilation=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub dilation: Option<String>,
    /// FFmpeg `erosion=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub erosion: Option<String>,
    /// FFmpeg `colorize=` option string (e.g. `hue=120:saturation=0.5`); empty uses defaults.
    pub colorize: Option<String>,
    /// FFmpeg `exposure=` option string (e.g. `exposure=0.5`); empty uses defaults.
    pub exposure: Option<String>,
    /// FFmpeg `chromashift=` option string (e.g. `cbh=4`); empty uses defaults.
    pub chromashift: Option<String>,
    /// FFmpeg `colorcontrast=` option string (e.g. `rc=0.1:gm=0.1:by=0.1`); empty uses defaults.
    pub colorcontrast: Option<String>,
    /// FFmpeg `colorcorrect=` option string (e.g. `rl=0.1:bl=-0.1`); empty uses defaults.
    pub colorcorrect: Option<String>,
    /// FFmpeg `histeq=` option string (e.g. `strength=0.2`); empty uses defaults.
    pub histeq: Option<String>,
    /// FFmpeg `shuffleplanes=` option string (e.g. `map0=0:map1=2:map2=1`); empty uses defaults.
    pub shuffleplanes: Option<String>,
    /// FFmpeg `lutyuv=` option string (e.g. `y=val*0.8`); empty uses defaults.
    pub lutyuv: Option<String>,
    /// FFmpeg `colorhold=` option string (e.g. `similarity=0.2:blend=0.1`); empty uses defaults.
    pub colorhold: Option<String>,
    /// FFmpeg `fade=` option string (e.g. `t=in:s=0:n=4`); empty uses defaults.
    pub fade: Option<String>,
    /// FFmpeg `perspective=` option string; empty uses defaults.
    pub perspective: Option<String>,
    /// FFmpeg `lumakey=` option string (e.g. `threshold=0.1:tolerance=0.1:softness=0.1`); empty uses defaults.
    pub lumakey: Option<String>,
    /// FFmpeg `chromakey=` option string (e.g. `similarity=0.3:blend=0.1`); empty uses defaults.
    pub chromakey: Option<String>,
    /// FFmpeg `colorkey=` option string (e.g. `color=black:similarity=0.1:blend=0.1`); empty uses defaults.
    pub colorkey: Option<String>,
    /// FFmpeg `despill=` option string (e.g. `type=green:mix=0.5`); empty uses defaults.
    pub despill: Option<String>,
    /// FFmpeg `selectivecolor=` option string (e.g. `reds=0.2 0 0 0`); empty uses defaults.
    pub selectivecolor: Option<String>,
    /// FFmpeg `stereo3d=` option string (e.g. `sbsl:abl`); empty uses defaults.
    pub stereo3d: Option<String>,
    /// FFmpeg `field=` option string (e.g. `bottom`); empty uses defaults.
    pub field: Option<String>,
    /// FFmpeg `hqx=` pixel-art upscale after field (e.g. `n=2`).
    pub hqx: Option<String>,
    /// FFmpeg `xbr=` pixel-art upscale after hqx (e.g. `n=2`).
    pub xbr: Option<String>,
    /// FFmpeg `il=` interleave/deinterleave lines (e.g. `l=d:c=d`); empty uses defaults.
    pub il: Option<String>,
    /// FFmpeg `super2xsai` 2× pixel-art upscale after il; empty uses defaults.
    pub super2xsai: Option<String>,
    /// FFmpeg `kerndeint=` kernel deinterlace after super2xsai (e.g. `thresh=10`); empty uses defaults.
    pub kerndeint: Option<String>,
    /// FFmpeg `phase=` field phase shift after kerndeint (e.g. `mode=t`); empty uses defaults.
    pub phase: Option<String>,
    /// FFmpeg `estdif=` edge-slope deinterlace after phase (e.g. `mode=frame`); empty uses defaults.
    pub estdif: Option<String>,
    /// FFmpeg `tinterlace=` temporal field interlace after estdif (e.g. `mode=merge`); empty uses defaults.
    pub tinterlace: Option<String>,
    /// FFmpeg `separatefields` splits each frame into two fields after tinterlace; empty uses defaults.
    pub separatefields: Option<String>,
    /// FFmpeg `weave` joins successive fields into frames after separatefields; empty uses defaults.
    pub weave: Option<String>,
    /// FFmpeg `doubleweave` weaves each input field into a frame after weave; empty uses defaults.
    pub doubleweave: Option<String>,
    /// FFmpeg `framepack=` packs consecutive frames after doubleweave (e.g. `format=sbs`); empty uses defaults.
    pub framepack: Option<String>,
    /// FFmpeg `telecine=` applies a pulldown pattern after framepack (e.g. `pattern=23`); empty uses defaults.
    pub telecine: Option<String>,
    /// FFmpeg `pullup=` inverse telecine after telecine (e.g. `jl=0:jr=0`); empty uses defaults.
    pub pullup: Option<String>,
    /// FFmpeg `decimate=` drops one frame per cycle after pullup (e.g. `cycle=5`); empty uses defaults.
    pub decimate: Option<String>,
    /// FFmpeg `mpdecimate=` drops near-duplicate frames after decimate; empty uses defaults.
    pub mpdecimate: Option<String>,
    /// FFmpeg `framestep=` keeps one frame every N after mpdecimate (e.g. `2`, `step=2`); empty uses defaults.
    pub framestep: Option<String>,
    /// FFmpeg `tile=` packs consecutive frames into a grid after framestep (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub tile: Option<String>,
    /// FFmpeg `untile=` splits each frame into a grid of frames after tile (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub untile: Option<String>,
    /// FFmpeg `shuffleframes=` reorders frames after untile (e.g. `2 1 0`, `mapping=2|1|0`); empty uses defaults.
    pub shuffleframes: Option<String>,
    /// FFmpeg `reverse` emits frames in reverse order after shuffleframes; empty uses defaults.
    pub reverse: Option<String>,
    /// FFmpeg `loop=` repeats a short frame window after reverse (e.g. `loop=1:size=2:start=0`); empty uses defaults.
    pub r#loop: Option<String>,
    /// FFmpeg `thumbnail=` picks the most representative frame from every batch of N after loop (e.g. `n=3`, `3`); empty uses defaults.
    pub thumbnail: Option<String>,
    /// FFmpeg `freezedetect=` detects frozen input after thumbnail (e.g. `n=0.001:d=0.1`); empty uses defaults.
    pub freezedetect: Option<String>,
    /// FFmpeg `pseudocolor=` option string (e.g. `preset=magma`); empty uses defaults.
    pub pseudocolor: Option<String>,
    /// FFmpeg `minterpolate=` option string (e.g. `mi_mode=blend:fps=50`); empty uses defaults.
    pub minterpolate: Option<String>,
    /// FFmpeg `fps=` rate string (e.g. `25`, `12`, `30000/1001`).
    pub fps: Option<String>,
    /// FFmpeg `colorspace=` option string (e.g. `iall=bt470bg:all=bt709`).
    pub colorspace: Option<String>,
    /// FFmpeg `zscale=` option string (e.g. `matrixin=bt470bg:matrix=bt709`).
    pub zscale: Option<String>,
    /// FFmpeg `tonemap=` option string (e.g. `tonemap=hable`).
    pub tonemap: Option<String>,
    /// Convert to this pixel format after geometry (FFmpeg `format=`).
    pub pix_fmt: Option<String>,
    /// Half-open presentation-time interval in microseconds relative to container start.
    pub interval: Option<(i64, i64)>,
    /// Use demuxer seeking before decoding; requires an interval.
    pub seek: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodeTransform {
    pub crop: Option<CropRect>,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
    /// Exact output size after crop/flips/transpose/rotate/pad. Geometry-only paths
    /// use nearest-neighbor sampling; consult the selected backend for support.
    pub scale: Option<ScaleSize>,
    /// FFmpeg `epx=` pixel-art upscale after scale and before burn/overlay (e.g. `n=2`).
    pub epx: Option<String>,
    /// FFmpeg-compatible `transpose=` after crop/flips and before rotate/pad/scale.
    pub transpose: Option<TransposeMode>,
    /// FFmpeg-compatible `rotate=` after transpose and before pad/scale.
    pub rotate: Option<RotateAngle>,
    /// FFmpeg-compatible `pad=W:H:X:Y:black` after rotate and before scale.
    pub pad: Option<PadRect>,
    /// External text subtitle file burned via libavfilter `subtitles=` after geometry.
    pub burn_subs: Option<PathBuf>,
    /// External video composited via `movie=` + `overlay=` after burn-in.
    pub overlay: Option<crate::OverlaySpec>,
    /// FFmpeg `yadif=` option string (e.g. `mode=0`); empty uses defaults.
    pub yadif: Option<String>,
    /// FFmpeg `bwdif=` option string (e.g. `mode=0`); empty uses defaults.
    pub bwdif: Option<String>,
    /// FFmpeg `w3fdif=` option string (e.g. `mode=0`); empty uses defaults.
    pub w3fdif: Option<String>,
    /// FFmpeg `tblend=` option string (e.g. `all_mode=average`).
    pub tblend: Option<String>,
    /// FFmpeg `tmix=` option string (e.g. `frames=3`).
    pub tmix: Option<String>,
    /// FFmpeg `hqdn3d=` option string (e.g. `4:3:6:4.5`); empty uses defaults.
    pub hqdn3d: Option<String>,
    /// FFmpeg `gblur=` option string (e.g. `sigma=1.5:steps=1`); empty uses defaults.
    pub gblur: Option<String>,
    /// FFmpeg `eq=` option string (e.g. `brightness=0.06:contrast=1.2`); empty uses defaults.
    pub eq: Option<String>,
    /// FFmpeg `unsharp=` option string (e.g. `5:5:1.0:5:5:0.0`); empty uses defaults.
    pub unsharp: Option<String>,
    /// FFmpeg `hue=` option string (e.g. `h=90:s=1.2`); empty uses defaults.
    pub hue: Option<String>,
    /// FFmpeg `avgblur=` option string (e.g. `sizeX=5:sizeY=5`); empty uses defaults.
    pub avgblur: Option<String>,
    /// FFmpeg `boxblur=` option string (e.g. `2:1`); empty uses defaults.
    pub boxblur: Option<String>,
    /// FFmpeg `negate` / `negate=1`; empty/`0` = components only.
    pub negate: Option<String>,
    /// FFmpeg `edgedetect=` option string (e.g. `mode=colormix`); empty uses defaults.
    pub edgedetect: Option<String>,
    /// FFmpeg `sobel=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub sobel: Option<String>,
    /// FFmpeg `prewitt=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub prewitt: Option<String>,
    /// FFmpeg `roberts=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub roberts: Option<String>,
    /// FFmpeg `kirsch=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub kirsch: Option<String>,
    /// FFmpeg `scharr=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub scharr: Option<String>,
    /// FFmpeg `atadenoise=` option string (e.g. `0a=0.02:0b=0.04`); empty uses defaults.
    pub atadenoise: Option<String>,
    /// FFmpeg `owdenoise=` option string (e.g. `depth=8:luma_strength=1.0`); empty uses defaults.
    pub owdenoise: Option<String>,
    /// FFmpeg `vaguedenoiser=` option string (e.g. `threshold=3`); empty uses defaults.
    pub vaguedenoiser: Option<String>,
    /// FFmpeg `nlmeans=` option string (e.g. `s=1.0`); empty uses defaults.
    pub nlmeans: Option<String>,
    /// FFmpeg `bm3d=` option string (e.g. `sigma=3`); empty uses defaults.
    pub bm3d: Option<String>,
    /// FFmpeg `dctdnoiz=` option string (e.g. `s=3`); empty uses defaults.
    pub dctdnoiz: Option<String>,
    /// FFmpeg `fftdnoiz=` option string (e.g. `sigma=1`); empty uses defaults.
    pub fftdnoiz: Option<String>,
    /// FFmpeg `smartblur=` option string (e.g. `lr=1.5:ls=-0.5`); empty uses defaults.
    pub smartblur: Option<String>,
    /// FFmpeg `sab=` option string (e.g. `lr=2:cr=2`); empty uses defaults.
    pub sab: Option<String>,
    /// FFmpeg `bilateral=` option string (e.g. `sigmaS=0.1:sigmaR=0.1`); empty uses defaults.
    pub bilateral: Option<String>,
    /// FFmpeg `cas=` option string (e.g. `strength=0.5`); empty uses defaults.
    pub cas: Option<String>,
    /// FFmpeg `vignette=` option string (e.g. `angle=PI/4`); empty uses defaults.
    pub vignette: Option<String>,
    /// FFmpeg `curves=` option string (e.g. `preset=vintage`); empty uses defaults.
    pub curves: Option<String>,
    /// FFmpeg `colorbalance=` option string (e.g. `rs=.1:gs=.05:bs=-.1`); empty uses defaults.
    pub colorbalance: Option<String>,
    /// FFmpeg `colorlevels=` option string (e.g. `rimin=0.1:gimin=0.1:bimin=0.1`); empty uses defaults.
    pub colorlevels: Option<String>,
    /// FFmpeg `colorchannelmixer=` option string (e.g. `rr=1.1:gg=0.9:bb=1.0`); empty uses defaults.
    pub colorchannelmixer: Option<String>,
    /// FFmpeg `deflicker=` option string (e.g. `mode=am:size=5`); empty uses defaults.
    pub deflicker: Option<String>,
    /// FFmpeg `photosensitivity=` option string (e.g. `f=5`); empty uses defaults.
    pub photosensitivity: Option<String>,
    /// FFmpeg `monochrome=` option string (e.g. `cb=0.2:cr=-0.1`); empty uses defaults.
    pub monochrome: Option<String>,
    /// FFmpeg `grayworld` / `grayworld=` (white balance); empty/`0` uses defaults.
    pub grayworld: Option<String>,
    /// FFmpeg `drawbox=` option string (e.g. `x=10:y=10:w=40:h=20:color=red`); empty uses defaults.
    pub drawbox: Option<String>,
    /// FFmpeg `drawgrid=` option string (e.g. `w=16:h=16:color=white`); empty uses defaults.
    pub drawgrid: Option<String>,
    /// FFmpeg `lagfun=` option string (e.g. `decay=0.95`); empty uses defaults.
    pub lagfun: Option<String>,
    /// FFmpeg `amplify=` option string (e.g. `radius=2:factor=2`); empty uses defaults.
    pub amplify: Option<String>,
    /// FFmpeg `bitplanenoise=` option string (e.g. `bitplane=1:filter=1`); empty uses defaults.
    pub bitplanenoise: Option<String>,
    /// FFmpeg `deband=` option string (e.g. `1thr=0.02`); empty uses defaults.
    pub deband: Option<String>,
    /// FFmpeg `gradfun=` option string (e.g. `strength=1.2`); empty uses defaults.
    pub gradfun: Option<String>,
    /// FFmpeg `lenscorrection=` option string (e.g. `k1=-0.1`); empty uses defaults.
    pub lenscorrection: Option<String>,
    /// FFmpeg `pixelize=` option string (e.g. `width=8:height=8`); empty uses defaults.
    pub pixelize: Option<String>,
    /// FFmpeg `removegrain=` option string (e.g. `m0=1`); empty uses defaults.
    pub removegrain: Option<String>,
    /// FFmpeg `yaepblur=` option string (e.g. `r=3`); empty uses defaults.
    pub yaepblur: Option<String>,
    /// FFmpeg `vibrance=` option string (e.g. `intensity=0.3:rbal=1`); empty uses defaults.
    pub vibrance: Option<String>,
    /// FFmpeg `dilation=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub dilation: Option<String>,
    /// FFmpeg `erosion=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub erosion: Option<String>,
    /// FFmpeg `colorize=` option string (e.g. `hue=120:saturation=0.5`); empty uses defaults.
    pub colorize: Option<String>,
    /// FFmpeg `exposure=` option string (e.g. `exposure=0.5`); empty uses defaults.
    pub exposure: Option<String>,
    /// FFmpeg `chromashift=` option string (e.g. `cbh=4`); empty uses defaults.
    pub chromashift: Option<String>,
    /// FFmpeg `colorcontrast=` option string (e.g. `rc=0.1:gm=0.1:by=0.1`); empty uses defaults.
    pub colorcontrast: Option<String>,
    /// FFmpeg `colorcorrect=` option string (e.g. `rl=0.1:bl=-0.1`); empty uses defaults.
    pub colorcorrect: Option<String>,
    /// FFmpeg `histeq=` option string (e.g. `strength=0.2`); empty uses defaults.
    pub histeq: Option<String>,
    /// FFmpeg `shuffleplanes=` option string (e.g. `map0=0:map1=2:map2=1`); empty uses defaults.
    pub shuffleplanes: Option<String>,
    /// FFmpeg `lutyuv=` option string (e.g. `y=val*0.8`); empty uses defaults.
    pub lutyuv: Option<String>,
    /// FFmpeg `colorhold=` option string (e.g. `similarity=0.2:blend=0.1`); empty uses defaults.
    pub colorhold: Option<String>,
    /// FFmpeg `fade=` option string (e.g. `t=in:s=0:n=4`); empty uses defaults.
    pub fade: Option<String>,
    /// FFmpeg `perspective=` option string; empty uses defaults.
    pub perspective: Option<String>,
    /// FFmpeg `lumakey=` option string (e.g. `threshold=0.1:tolerance=0.1:softness=0.1`); empty uses defaults.
    pub lumakey: Option<String>,
    /// FFmpeg `chromakey=` option string (e.g. `similarity=0.3:blend=0.1`); empty uses defaults.
    pub chromakey: Option<String>,
    /// FFmpeg `colorkey=` option string (e.g. `color=black:similarity=0.1:blend=0.1`); empty uses defaults.
    pub colorkey: Option<String>,
    /// FFmpeg `despill=` option string (e.g. `type=green:mix=0.5`); empty uses defaults.
    pub despill: Option<String>,
    /// FFmpeg `selectivecolor=` option string (e.g. `reds=0.2 0 0 0`); empty uses defaults.
    pub selectivecolor: Option<String>,
    /// FFmpeg `stereo3d=` option string (e.g. `sbsl:abl`); empty uses defaults.
    pub stereo3d: Option<String>,
    /// FFmpeg `field=` option string (e.g. `bottom`); empty uses defaults.
    pub field: Option<String>,
    /// FFmpeg `hqx=` pixel-art upscale after field (e.g. `n=2`).
    pub hqx: Option<String>,
    /// FFmpeg `xbr=` pixel-art upscale after hqx (e.g. `n=2`).
    pub xbr: Option<String>,
    /// FFmpeg `il=` interleave/deinterleave lines (e.g. `l=d:c=d`); empty uses defaults.
    pub il: Option<String>,
    /// FFmpeg `super2xsai` 2× pixel-art upscale after il; empty uses defaults.
    pub super2xsai: Option<String>,
    /// FFmpeg `kerndeint=` kernel deinterlace after super2xsai (e.g. `thresh=10`); empty uses defaults.
    pub kerndeint: Option<String>,
    /// FFmpeg `phase=` field phase shift after kerndeint (e.g. `mode=t`); empty uses defaults.
    pub phase: Option<String>,
    /// FFmpeg `estdif=` edge-slope deinterlace after phase (e.g. `mode=frame`); empty uses defaults.
    pub estdif: Option<String>,
    /// FFmpeg `tinterlace=` temporal field interlace after estdif (e.g. `mode=merge`); empty uses defaults.
    pub tinterlace: Option<String>,
    /// FFmpeg `separatefields` splits each frame into two fields after tinterlace; empty uses defaults.
    pub separatefields: Option<String>,
    /// FFmpeg `weave` joins successive fields into frames after separatefields; empty uses defaults.
    pub weave: Option<String>,
    /// FFmpeg `doubleweave` weaves each input field into a frame after weave; empty uses defaults.
    pub doubleweave: Option<String>,
    /// FFmpeg `framepack=` packs consecutive frames after doubleweave (e.g. `format=sbs`); empty uses defaults.
    pub framepack: Option<String>,
    /// FFmpeg `telecine=` applies a pulldown pattern after framepack (e.g. `pattern=23`); empty uses defaults.
    pub telecine: Option<String>,
    /// FFmpeg `pullup=` inverse telecine after telecine (e.g. `jl=0:jr=0`); empty uses defaults.
    pub pullup: Option<String>,
    /// FFmpeg `decimate=` drops one frame per cycle after pullup (e.g. `cycle=5`); empty uses defaults.
    pub decimate: Option<String>,
    /// FFmpeg `mpdecimate=` drops near-duplicate frames after decimate; empty uses defaults.
    pub mpdecimate: Option<String>,
    /// FFmpeg `framestep=` keeps one frame every N after mpdecimate (e.g. `2`, `step=2`); empty uses defaults.
    pub framestep: Option<String>,
    /// FFmpeg `tile=` packs consecutive frames into a grid after framestep (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub tile: Option<String>,
    /// FFmpeg `untile=` splits each frame into a grid of frames after tile (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub untile: Option<String>,
    /// FFmpeg `shuffleframes=` reorders frames after untile (e.g. `2 1 0`, `mapping=2|1|0`); empty uses defaults.
    pub shuffleframes: Option<String>,
    /// FFmpeg `reverse` emits frames in reverse order after shuffleframes; empty uses defaults.
    pub reverse: Option<String>,
    /// FFmpeg `loop=` repeats a short frame window after reverse (e.g. `loop=1:size=2:start=0`); empty uses defaults.
    pub r#loop: Option<String>,
    /// FFmpeg `thumbnail=` picks the most representative frame from every batch of N after loop (e.g. `n=3`, `3`); empty uses defaults.
    pub thumbnail: Option<String>,
    /// FFmpeg `freezedetect=` detects frozen input after thumbnail (e.g. `n=0.001:d=0.1`); empty uses defaults.
    pub freezedetect: Option<String>,
    /// FFmpeg `pseudocolor=` option string (e.g. `preset=magma`); empty uses defaults.
    pub pseudocolor: Option<String>,
    /// FFmpeg `minterpolate=` option string (e.g. `mi_mode=blend:fps=50`); empty uses defaults.
    pub minterpolate: Option<String>,
    /// FFmpeg `fps=` rate string (e.g. `25`, `12`, `30000/1001`).
    pub fps: Option<String>,
    /// FFmpeg `colorspace=` option string (e.g. `iall=bt470bg:all=bt709`).
    pub colorspace: Option<String>,
    /// FFmpeg `zscale=` option string (e.g. `matrixin=bt470bg:matrix=bt709`).
    pub zscale: Option<String>,
    /// FFmpeg `tonemap=` option string (e.g. `tonemap=hable`).
    pub tonemap: Option<String>,
    /// Convert to this pixel format after geometry (FFmpeg `format=`).
    pub pix_fmt: Option<String>,
    /// Half-open presentation interval in microseconds from container start.
    pub interval: Option<(i64, i64)>,
    /// Demuxer to read the file with, for a format whose bytes carry no signature — the
    /// equivalent of ffmpeg's `-f`. `None` requests format detection by the backend.
    pub input_format: Option<String>,
}

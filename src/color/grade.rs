//! One decision per picture: what its colour codes mean, and what to show instead.
//!
//! A [`Grade`] joins the signal a file states ([`ColourDescription`]) and the
//! light it carries ([`HdrMetadata`]) into a [`CubePlan`] for the panel the
//! caller has, bakes that plan into a grid, and puts a grading LUT after it.
//! Everything downstream — the CPU converter and the fragment shader alike —
//! then has one lookup to make and decides nothing about colour itself.
//!
//! The bake is what makes the two paths agree: a plan is a chain of transfer,
//! gamut and tone-mapping stages, and once it is a grid, a CPU loop and a GPU
//! texture read apply the same numbers. A grid is also what a camera vendor's
//! "identity" LUT is, so the file a caller passes in and the conversion fvid
//! derives for itself are the same kind of object by the time they are applied.
use crate::color::hdr::{ColourDescription, HdrMetadata};
use crate::color::log::Log;
use crate::color::lut::{CubePlan, Interpolation, Lut, Lut3d};
use crate::color::primaries::Primaries;
use crate::color::tonemap::{DisplayTarget, ToneMap};
use crate::color::transfer::Transfer;

/// The panel and destination signal a caller grades for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Camera log curve the coded values carry, which replaces the file's
    /// transfer and, where the file names no primaries, its primaries too.
    pub log: Option<Log>,
    /// The working gamut the coded values are stated in, named by the caller
    /// instead of read off the file or the curve. It outranks both, which is how
    /// a container that labels its bytes wrongly gets corrected; see
    /// [`Grade::new`].
    pub gamut: Option<Primaries>,
    /// Highlight compression to run. `None` on BT.2100 material still compresses;
    /// see [`Grade::new`].
    pub tone_map: Option<ToneMap>,
    /// The panel the picture is shown on.
    pub target: DisplayTarget,
    /// Transfer the output codes are written with.
    pub to: Transfer,
    /// Primaries the output codes are stated in.
    pub dest: Primaries,
    /// Grid edge length to bake the plan at.
    pub size: usize,
    /// How the grid is read between its nodes.
    pub interpolation: Interpolation,
}

impl Default for Settings {
    /// An ordinary desktop picture on an SDR panel: sRGB codes over BT.709
    /// primaries, nothing decoded and nothing compressed.
    fn default() -> Self {
        Self {
            log: None,
            gamut: None,
            tone_map: None,
            target: DisplayTarget::sdr(100.0),
            to: Transfer::Srgb,
            dest: Primaries::BT709,
            size: 33,
            interpolation: Interpolation::Tetrahedral,
        }
    }
}

impl Settings {
    /// A video surface: BT.709 codes rather than the desktop's sRGB ones.
    pub fn video(target: DisplayTarget) -> Self {
        Self {
            to: Transfer::Bt709,
            target,
            ..Self::default()
        }
    }
}

/// A baked conversion plus the grading LUT that follows it.
#[derive(Clone, Debug)]
pub struct Grade {
    plan: CubePlan,
    cube: Lut,
    lut: Option<Lut>,
    interpolation: Interpolation,
    /// Output codes per channel, when the conversion keeps channels apart.
    ///
    /// Measured on a 1080p frame in release: reading the grid costs 10.3 ns a
    /// pixel, twenty-one milliseconds a frame, and does not move with the grid's
    /// edge length. Three byte lookups cost 1.0 ns, and agree with the grid to
    /// within a code — the residual is tetrahedral selection seeing a different
    /// pair of companions than the grey axis does. So the common conversion, one
    /// whose primaries and highlights are untouched and only its curve changes,
    /// is flattened to tables before a pixel is touched.
    tables: Option<[Vec<u8>; 3]>,
}

impl Grade {
    /// Read `signal` and `hdr` the way a stream states them and write them for
    /// `settings`. A `lut` the caller loaded is applied after the conversion,
    /// which is the order a grading LUT authored for display-referred material
    /// expects.
    ///
    /// Highlight compression is on for BT.2100 material even when
    /// [`Settings::tone_map`] is `None`, unless the destination curve is BT.2100
    /// too: an unmapped PQ picture on an SDR panel is a flat grey one, and a
    /// caller who really wants that asks for a BT.2100 `to`. The curve picked
    /// for a caller who asks for none follows the headroom: content that stays
    /// within the panel needs no shoulder, and content that reaches past it
    /// needs one that keeps highlights instead of folding them to white.
    pub fn new(
        signal: ColourDescription,
        hdr: &HdrMetadata,
        settings: Settings,
        lut: Option<Lut>,
    ) -> Self {
        let from = match settings.log {
            // A log curve is the transfer; the file's own code says nothing more.
            Some(_) => Transfer::Linear,
            None => signal.transfer_function(),
        };
        let content = hdr.content_light(settings.target.peak_nits);
        let tone_map = match settings.tone_map {
            Some(mode) => Some(mode),
            None if from.is_hdr() && !settings.to.is_hdr() => {
                // Measured against the other four on a 1 000 cd/m² master going
                // onto a 100-nit panel: `linear` and `gamma` take the whole
                // picture down with the highlights, `reinhard` and `hable` move
                // even codes the panel already shows, and Möbius holds every
                // value under its joint put while it rolls the rest in. Where
                // the content never exceeds the panel there is nothing to roll,
                // and the clip is exactly the identity below white.
                Some(if content.max_cll > settings.target.peak_nits {
                    ToneMap::Mobius
                } else {
                    ToneMap::Clip
                })
            }
            None => None,
        };
        // Which primaries the coded values are stated in. A gamut the caller
        // names comes first, because naming one is how you correct a container
        // that labels these bytes wrongly or not at all. After that the file is
        // asked: a camera that named its own gamut knows these particular bytes
        // better than any profile does. Only when neither names one does a log
        // curve bring its vendor's working gamut with it, which is what the
        // camera recorded into, and BT.709 answers for material that states
        // nothing.
        let source = settings
            .gamut
            .or_else(|| signal.primary_set())
            .or_else(|| settings.log.and_then(Log::gamut))
            .unwrap_or(Primaries::BT709);
        let plan = CubePlan {
            from,
            log: settings.log,
            source,
            to: settings.to,
            dest: settings.dest,
            size: settings.size,
            sdr_white_nits: settings.target.paper_white_nits,
            // The panel travels with the plan whether or not something
            // compresses onto it: HLG's codes mean "whatever this panel
            // reaches", so a caller who named a peak has to be heard even on an
            // unmapped route. Whether a shoulder ran is `tone_map`'s to say.
            target: Some(settings.target),
            content,
            tone_map,
        };
        let cube = Lut::Three(plan.build());
        // HLG's light-dependent OOTF and a tone curve both read the other two
        // channels while mapping one, so only a plan without either — and
        // without a grid-shaped LUT after it — separates into per-channel codes.
        let mixes_channels = plan.tone_map.is_some()
            || plan.source != plan.dest
            || (plan.from == Transfer::Hlg && plan.log.is_none())
            || matches!(lut, Some(Lut::Three(_)));
        let interpolation = settings.interpolation;
        let mut tables = (!mixes_channels).then(|| byte_tables(&cube, lut.as_ref(), interpolation));
        // That is the cheap no; this is the measured yes. A plan can clear every
        // clause above and still fold channels together: a log curve goes
        // negative below its toe, and the gamut compress that guards the grid's
        // ends desaturates any negative channel toward luma. So an S-Log3 or
        // V-Log route with no tone map looks separable and is not — a byte table
        // baked on the grey axis is twenty codes out for a pixel that is not
        // grey. Comparing the tables against the grid off that axis is the only
        // check that catches it, and giving up the fast path costs speed and
        // never colour.
        if tables
            .as_ref()
            .is_some_and(|tables| !keeps_channels_apart(&cube, lut.as_ref(), interpolation, tables))
        {
            tables = None;
        }
        Self {
            cube,
            plan,
            lut,
            interpolation,
            tables,
        }
    }

    /// The plan behind the grid, for an OSD line that says what is being done.
    pub fn plan(&self) -> CubePlan {
        self.plan
    }

    /// The LUT the caller supplied, if any.
    pub fn lut(&self) -> Option<&Lut> {
        self.lut.as_ref()
    }

    /// How the baked grid and that LUT are read between their nodes, which an
    /// OSD line says along with the plan.
    pub fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    /// Code values in, code values out.
    pub fn rgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mapped = self.cube.sample(rgb, self.interpolation);
        match &self.lut {
            None => mapped,
            Some(lut) => lut.sample(mapped, self.interpolation),
        }
    }

    /// The table a fragment shader can bind for this grade, if one is enough.
    ///
    /// A grade that is one lookup hands the shader the very table the CPU reads:
    /// the byte tables when the plan keeps its channels apart — they already
    /// fold in a 1D LUT chained after the conversion — and the plan's own grid
    /// when it does not. Either way both routes read the same numbers, which is
    /// the point; a rebake at the shader's resolution would be a different
    /// picture, and the two are compared byte for byte in `player_gpu`'s
    /// readback tests.
    ///
    /// A grid-shaped LUT after the conversion is a second, three-dimensional
    /// table, and one binding cannot hold it read after the first. Joining them
    /// is not a rounding detail either — measured at
    /// `a_chained_grade_is_not_a_single_lookup` below, a composed grid moves a
    /// code by as much as six at the edge length a caller is given by default,
    /// and no edge length fixes it. So a chained 3D grade answers `None` and
    /// stays on the route that takes both steps.
    pub fn shader_look(&self) -> Option<ShaderLook> {
        match (&self.cube, &self.tables) {
            (Lut::Three(_), Some(tables)) => Some(ShaderLook::Tables(tables.clone())),
            (Lut::Three(cube), None) if self.lut.is_none() => Some(ShaderLook::Grid(cube.clone())),
            _ => None,
        }
    }

    /// True when a fragment shader can show this grade from one lookup, which is
    /// what lets a plane picture keep its planes and still be graded. Says the
    /// same as [`shader_look`](Self::shader_look) without copying a table.
    pub fn is_shader_look(&self) -> bool {
        self.shader_look().is_some()
    }

    /// True when applying this grade could not change a pixel: the grid is its
    /// own input and no LUT follows it. A caller skips the lookup on this.
    pub fn is_identity(&self) -> bool {
        // Half a code step at 8 bits; a grade finer than that is not a picture.
        let tol = 0.5 / 255.0;
        self.cube.is_identity(tol) && self.lut.as_ref().is_none_or(|lut| lut.is_identity(tol))
    }

    /// Apply the grade to a packed RGB8 image, in place.
    ///
    /// One thread reading the grid costs about 19 ms for a 1080p frame and 73 ms
    /// for a 4K one, or 49 ms and 195 ms once a 3D LUT follows it, which spends
    /// the whole 24 fps budget on the colour before a pixel of decoding is
    /// counted. So the picture is cut into spans and each span is graded by a
    /// worker of its own: the same frames then take 3 ms, 11 ms and 22 ms on this
    /// machine. The workers share nothing but the read-only grid, so the bytes a
    /// pixel ends up with do not depend on which worker was given it.
    pub fn apply(&self, rgb: &mut [u8]) {
        if self.is_identity() {
            return;
        }
        let workers = crate::span_workers(rgb.len());
        if workers < 2 {
            self.paint(rgb);
            return;
        }
        let span = rgb.len().div_ceil(workers).next_multiple_of(3);
        std::thread::scope(|scope| {
            for chunk in rgb.chunks_mut(span) {
                scope.spawn(|| self.paint(chunk));
            }
        });
    }

    /// Grade one span of packed pixels, on the thread that calls this.
    fn paint(&self, rgb: &mut [u8]) {
        match &self.tables {
            Some([red, green, blue]) => {
                for pixel in rgb.as_chunks_mut::<3>().0 {
                    let [r, g, b] = *pixel;
                    pixel[0] = red[r as usize];
                    pixel[1] = green[g as usize];
                    pixel[2] = blue[b as usize];
                }
            }
            None => {
                for pixel in rgb.as_chunks_mut::<3>().0 {
                    let codes = [
                        f32::from(pixel[0]) / 255.0,
                        f32::from(pixel[1]) / 255.0,
                        f32::from(pixel[2]) / 255.0,
                    ];
                    for (out, value) in pixel.iter_mut().zip(self.rgb(codes)) {
                        *out = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
    }
}

/// The table a fragment shader binds for a grade that is one lookup: whichever
/// of the two the CPU reads for it, so that the picture on the screen and the
/// same picture graded for a snapshot come out of the same numbers. A grade whose
/// channels stay apart is read as 256 output bytes a channel; one that mixes them
/// is read as the grid it was baked at, between its nodes.
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderLook {
    /// Output bytes per input code, per channel: the CPU's fast path, and the
    /// only kind of table that is exact at every one of the 256 codes.
    Tables([Vec<u8>; 3]),
    /// The grid the plan was baked at, read with the grade's interpolation.
    Grid(Lut3d),
}

/// The 256 output codes of each channel, read off the two stages at once.
fn byte_tables(cube: &Lut, lut: Option<&Lut>, interp: Interpolation) -> [Vec<u8>; 3] {
    [0, 1, 2].map(|channel| {
        (0..=255u8)
            .map(|code| {
                let v = f32::from(code) / 255.0;
                let mapped = cube.sample([v, v, v], interp);
                let graded = lut.map_or(mapped, |lut| lut.sample(mapped, interp));
                (graded[channel].clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect()
    })
}

/// True when each channel's byte table answers for the whole grid, whatever the
/// other two codes are.
///
/// The tables are baked along the grey axis, which is only the whole answer when
/// a plan maps each channel on its own. The tolerance is one output code: a byte
/// table rounds, so one code of difference between it and a float sample is the
/// table's own step and not a mixed channel — a genuinely separable curve route
/// was measured at 1.5e-8 of deviation, millionths of a code — while the
/// smallest real mix found was twenty-two codes of it for S-Log3 and forty-one
/// for V-Log. So the bound catches the defect with a margin and never
/// second-guesses a rounding step.
fn keeps_channels_apart(
    cube: &Lut,
    lut: Option<&Lut>,
    interp: Interpolation,
    tables: &[Vec<u8>; 3],
) -> bool {
    // The tables answer byte codes, so the probe asks them in byte codes: four
    // unequal ones per channel, none of them grey, with 33 and 158 landing in
    // the toe and the shoulder where a log curve's `to_linear` is negative and
    // above one.
    const CODES: [u8; 4] = [0, 33, 158, 255];
    for &r in &CODES {
        for &g in &CODES {
            for &b in &CODES {
                let inputs = [r, g, b];
                let mapped = cube.sample(inputs.map(|code| f32::from(code) / 255.0), interp);
                let graded = lut.map_or(mapped, |lut| lut.sample(mapped, interp));
                for (channel, &value) in graded.iter().enumerate() {
                    let shown = value.clamp(0.0, 1.0) * 255.0;
                    let tabled = f32::from(tables[channel][usize::from(inputs[channel])]);
                    if (shown - tabled).abs() > 1.0 {
                        return false;
                    }
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::color::hdr::MasteringDisplay;
    use crate::color::lut::{Lut1d, Lut3d};
    use crate::color::tonemap::ContentLight;

    fn bt709() -> ColourDescription {
        ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        }
    }

    fn bt2100(transfer: u8) -> ColourDescription {
        ColourDescription {
            transfer,
            primaries: 9,
            ..bt709()
        }
    }

    /// Code values on the grey axis, where a curve has a single answer.
    fn grey(grade: &Grade, code: f32) -> [f32; 3] {
        grade.rgb([code, code, code])
    }

    fn close(v: f32, want: f32, tol: f32) -> bool {
        (v - want).abs() <= tol
    }

    /// A grade that reads the grid for every channel of every pixel, so the
    /// work is the slow kind and a frame of it is worth cutting up.
    fn mixing() -> Grade {
        Grade::new(
            bt2100(16),
            &HdrMetadata {
                light: ContentLight {
                    max_cll: 1_000.0,
                    max_fall: 400.0,
                },
                ..Default::default()
            },
            Settings::video(DisplayTarget::sdr(100.0)),
            None,
        )
    }

    #[test]
    fn a_frame_is_graded_the_same_whichever_worker_paints_it() {
        let grade = mixing();
        assert_eq!(grade.plan().tone_map, Some(ToneMap::Mobius));
        // Enough pixels for apply() to cut the frame between workers.
        let pixels = 256 * 256;
        let source: Vec<u8> = (0..pixels * 3).map(|i| (i * 7 % 256) as u8).collect();
        let want: Vec<u8> = source
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| {
                let codes = [
                    f32::from(p[0]) / 255.0,
                    f32::from(p[1]) / 255.0,
                    f32::from(p[2]) / 255.0,
                ];
                grade
                    .rgb(codes)
                    .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            })
            .collect();
        let mut frame = source.clone();
        grade.apply(&mut frame);
        assert_eq!(frame, want);
        // The bytes past the last whole pixel are not a pixel, and the split
        // that hands each worker a span of them must leave them as they were.
        let mut ragged = source;
        ragged.extend([11, 22]);
        grade.apply(&mut ragged);
        assert_eq!(&ragged[..want.len()], &want[..]);
        assert_eq!(&ragged[want.len()..], [11, 22]);
    }

    #[test]
    fn a_small_frame_is_painted_by_the_thread_that_called_it() {
        assert_eq!(crate::span_workers(3 * 1024), 1);
        assert!(crate::span_workers(3 * 1920 * 1080) > 1);
    }

    #[test]
    fn a_picture_on_the_panel_and_curve_it_states_is_left_alone() {
        let grade = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings::video(DisplayTarget::sdr(240.0)),
            None,
        );
        assert!(grade.is_identity());
        for code in [0.0, 0.25, 0.5, 1.0] {
            assert_eq!(grey(&grade, code), [code, code, code]);
        }
    }

    #[test]
    fn the_same_light_on_a_desktop_surface_is_re_encoded() {
        // Nothing about the panel changes; only the curve the codes carry does,
        // so this is the one case where leaving the picture alone is wrong.
        let grade = Grade::new(bt709(), &HdrMetadata::default(), Settings::default(), None);
        assert!(!grade.is_identity());
        for code in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let linear = Transfer::Bt709.eotf(code).unwrap();
            let want = Transfer::Srgb.oetf(linear).unwrap();
            assert!(
                close(grey(&grade, code)[0], want, 0.002),
                "{code} -> {} over {want}",
                grey(&grade, code)[0]
            );
        }
    }

    #[test]
    fn hdr_is_compressed_for_an_sdr_panel_even_when_nobody_chose_a_curve() {
        let settings = Settings::video(DisplayTarget::sdr(203.0));
        assert!(settings.tone_map.is_none());
        let grade = Grade::new(bt2100(16), &HdrMetadata::default(), settings, None);
        // Nothing is stated above this panel's peak, so the pick is the curve
        // that invents no shoulder.
        assert_eq!(grade.plan().tone_map, Some(ToneMap::Clip));
        assert_eq!(grade.plan().target, Some(DisplayTarget::sdr(203.0)));
        // PQ's full scale is 10 000 cd/m² onto a 203-nit panel: a shoulder has
        // to bring the top of the code range under white.
        assert!((grey(&grade, 1.0)[0] - 1.0).abs() < 0.02);
        assert!(grey(&grade, 0.5)[0] < 0.75);
        assert!(grey(&grade, 0.5)[0] > grey(&grade, 0.4)[0]);
    }

    #[test]
    fn an_hdr_destination_keeps_the_full_scale_and_runs_no_shoulder() {
        let grade = Grade::new(
            bt2100(16),
            &HdrMetadata::default(),
            Settings {
                to: Transfer::Pq,
                dest: Primaries::BT2020,
                ..Settings::video(DisplayTarget::hdr(1_000.0))
            },
            None,
        );
        assert!(grade.plan().tone_map.is_none());
        assert!(grade.is_identity());
    }

    /// HLG is the one HDR curve whose codes only mean light on the panel they
    /// were written for, so the panel a caller names has to reach an unmapped
    /// plan too. Before the target travelled with the plan every HLG
    /// destination was written against BT.2100's 1 000 cd/m² reference, which
    /// left a 400-nit screen showing the same code as a screen two and a half
    /// times as bright.
    #[test]
    fn a_named_panel_is_what_an_hlg_code_means() {
        for peak in [400.0, 1_000.0] {
            let target = DisplayTarget::hdr(peak);
            let grade = Grade::new(
                bt709(),
                &HdrMetadata::default(),
                Settings {
                    to: Transfer::Hlg,
                    dest: Primaries::BT2020,
                    ..Settings::video(target)
                },
                None,
            );
            assert!(grade.plan().tone_map.is_none());
            assert_eq!(grade.plan().target, Some(target));
            // The light an SDR code states on this panel's white, read back out
            // of the code the grade wrote.
            let want = Transfer::Bt709.eotf(0.5).unwrap() * target.paper_white_nits;
            let back = Transfer::Hlg.eotf(grey(&grade, 0.5)[0]).unwrap() * peak;
            assert!(
                close(back, want, want * 0.02),
                "a {peak}-nit panel got {:?} for code 0.5, meaning {back} cd/m² not {want}",
                grey(&grade, 0.5)
            );
        }
    }

    #[test]
    fn a_stream_that_states_its_light_compresses_from_it() {
        let settings = Settings {
            tone_map: Some(ToneMap::Reinhard),
            ..Settings::video(DisplayTarget::sdr(203.0))
        };
        let unstated = Grade::new(bt2100(16), &HdrMetadata::default(), settings, None);
        let volume = MasteringDisplay::from_corners(
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
            1_000.0,
            0.0001,
        )
        .unwrap();
        let stated = Grade::new(
            bt2100(16),
            &HdrMetadata {
                mastering: Some(volume),
                light: ContentLight {
                    max_cll: 1_000.0,
                    max_fall: 400.0,
                },
            },
            settings,
            None,
        );
        // With no MaxCLL the content peak is the panel's own headroom, and a
        // curve that reads the peak therefore maps the two pictures differently.
        assert_eq!(unstated.plan().content.max_cll, 203.0);
        assert_eq!(stated.plan().content.max_cll, 1_000.0);
        assert_ne!(grey(&unstated, 0.75), grey(&stated, 0.75));
        // The volume's own corners decide the gamut, not the container's code.
        assert_eq!(stated.plan().source, Primaries::BT2020);
    }

    /// The curve a caller who names none leaves fvid to pick follows how much of
    /// the light the panel cannot show, not the coding it came in with. Measured
    /// on a 1 000 cd/m² master going onto a 100-nit panel against the other four:
    /// `clip` keeps every code the panel reaches and folds the rest to one white,
    /// `linear` and `gamma` take the whole picture down with the highlights,
    /// `reinhard` and `hable` move even the codes the panel already shows, and
    /// `mobius` leaves everything under its joint exactly where it was.
    #[test]
    fn a_master_beyond_the_panels_reach_rolls_instead_of_burning() {
        let settings = Settings::video(DisplayTarget::sdr(100.0));
        let bright = HdrMetadata {
            mastering: None,
            light: ContentLight {
                max_cll: 1_000.0,
                max_fall: 400.0,
            },
        };
        let rolled = Grade::new(bt2100(16), &bright, settings, None);
        assert_eq!(rolled.plan().tone_map, Some(ToneMap::Mobius));
        // A file that states no peak leaves the panel's own headroom as the only
        // answer, and then there is nothing to roll.
        let blind = Grade::new(bt2100(16), &HdrMetadata::default(), settings, None);
        assert_eq!(blind.plan().tone_map, Some(ToneMap::Clip));
        // Which is the same picture below the joint: 18 cd/m² is identical on
        // both routes, so the shoulder costs the darks nothing.
        assert_eq!(grey(&rolled, 0.3), grey(&blind, 0.3));
        // Above the panel the two part: 189 cd/m² is a highlight the clip has no
        // number for, and the roll puts it under white with room left over.
        let (hot_rolled, hot_blind) = (rolled.rgb([0.62; 3])[0], blind.rgb([0.62; 3])[0]);
        assert!(hot_blind > 0.999, "{hot_blind}");
        assert!(
            (0.85..0.96).contains(&hot_rolled),
            "{hot_rolled} not a rolled highlight"
        );
        // Three highlights the clip cannot tell apart stay three separate ones.
        let codes = [0.62, 0.70, 0.75];
        let r: Vec<f32> = codes.iter().map(|c| rolled.rgb([*c; 3])[0]).collect();
        let b: Vec<f32> = codes.iter().map(|c| blind.rgb([*c; 3])[0]).collect();
        assert!(r[0] < r[1] && r[1] < r[2], "{r:?}");
        assert!(b.iter().all(|v| (v - 1.0).abs() < 1e-3), "{b:?}");
        // Content that stays inside the panel needs no shoulder either, and HLG,
        // whose scene light is the panel's own by definition, keeps that path.
        let dim = HdrMetadata {
            mastering: None,
            light: ContentLight {
                max_cll: 60.0,
                max_fall: 30.0,
            },
        };
        assert_eq!(
            Grade::new(bt2100(16), &dim, settings, None).plan().tone_map,
            Some(ToneMap::Clip)
        );
        assert_eq!(
            Grade::new(bt2100(18), &HdrMetadata::default(), settings, None)
                .plan()
                .tone_map,
            Some(ToneMap::Clip)
        );
        // A curve named from the command line still beats the headroom.
        let asked = Grade::new(
            bt2100(16),
            &bright,
            Settings {
                tone_map: Some(ToneMap::Hable),
                ..settings
            },
            None,
        );
        assert_eq!(asked.plan().tone_map, Some(ToneMap::Hable));
    }

    #[test]
    fn a_camera_log_replaces_the_signal_the_file_states() {
        let settings = Settings {
            log: Some(Log::SLog3),
            ..Settings::video(DisplayTarget::sdr(240.0))
        };
        let grade = Grade::new(bt709(), &HdrMetadata::default(), settings, None);
        assert_eq!(grade.plan().log, Some(Log::SLog3));
        assert_eq!(grade.plan().from, Transfer::Linear);
        // Sony's S-Log3 technical summary tabulates 10-bit codes for a full
        // range: 0 % at 95, 18 % grey at 420 and 90 % white at 598. Run through
        // the grade, each lands where BT.709 writes that reflectance. The
        // container still says BT.709 primaries, so only the curve is at work.
        let written = |reflectance: f32| Transfer::Bt709.oetf(reflectance).unwrap();
        for (ten_bit, reflectance) in [(95.0, 0.0f32), (420.0, 0.18), (598.0, 0.90)] {
            let signal = ten_bit / 1023.0;
            let want = written(reflectance);
            let got = grey(&grade, signal)[0];
            assert!(close(got, want, 0.003), "{ten_bit} -> {got} over {want}");
        }
        // Below the toe the curve goes negative, and a code value cannot.
        assert!(grey(&grade, 0.0)[0] < 0.002);
        // Above 90 % the log holds six more stops, which video has to clip.
        assert!(grey(&grade, 1.0)[0] > 0.99);
    }

    /// A log curve brings its vendor's working gamut with it, since that is the
    /// triangle the camera recorded into — but only where the file itself names
    /// no primaries, and only where the vendor published one at all.
    #[test]
    fn a_log_curve_lends_its_gamut_where_the_file_names_none() {
        let silent = ColourDescription::default();
        let s_log3 = Settings {
            log: Some(Log::SLog3),
            ..Settings::video(DisplayTarget::sdr(240.0))
        };
        let grade = Grade::new(silent, &HdrMetadata::default(), s_log3, None);
        assert_eq!(grade.plan().source, Primaries::S_GAMUT3);
        // A file that names its own primaries is the better authority on these
        // particular bytes, whatever curve it is read with.
        let named = Grade::new(bt709(), &HdrMetadata::default(), s_log3, None);
        assert_eq!(named.plan().source, Primaries::BT709);
        // S-Log2 predates a published S-Gamut table, so a silent file is left in
        // the space the destination is asked for rather than a guessed one.
        let s_log2 = Settings {
            log: Some(Log::SLog2),
            ..s_log3
        };
        let grade = Grade::new(silent, &HdrMetadata::default(), s_log2, None);
        assert_eq!(grade.plan().source, Primaries::BT709);
        // The two ARRI curves lend different triangles, which is what their
        // specifications say and what the plan has to carry for the bake.
        let logc4 = Settings {
            log: Some(Log::LogC4),
            ..s_log3
        };
        let grade = Grade::new(silent, &HdrMetadata::default(), logc4, None);
        assert_eq!(grade.plan().source, Primaries::ALEX3_EXPANDED);
        let logc = Settings {
            log: Some(Log::LogC),
            ..logc4
        };
        let grade = Grade::new(silent, &HdrMetadata::default(), logc, None);
        assert_eq!(grade.plan().source, Primaries::ALEX3_WIDE);
    }

    /// A caller who names a working gamut is correcting the file, not adding to
    /// it: a clip can carry S-Gamut3.Cine pixels under a BT.709 tag, and the one
    /// way to see them is to read the bytes as that gamut and ignore the label.
    /// So the name outranks both authorities the plan consults — the primaries
    /// the file states and the triangle a log curve lends — and it is what makes
    /// the gamuts no curve lends reachable at all: S-Gamut3.Cine, D-Gamut and
    /// F-Gamut C among them.
    #[test]
    fn a_gamut_the_caller_names_outranks_the_file_and_the_curve() {
        let cine = Primaries::S_GAMUT3_CINE;
        let settings = Settings {
            log: Some(Log::SLog3),
            gamut: Some(cine),
            ..Settings::video(DisplayTarget::sdr(240.0))
        };
        // Ahead of S-Gamut3, the triangle S-Log3 would otherwise lend.
        let grade = Grade::new(
            ColourDescription::default(),
            &HdrMetadata::default(),
            settings,
            None,
        );
        assert_eq!(grade.plan().source, cine);
        // Ahead of the primaries the file states for itself.
        let grade = Grade::new(bt709(), &HdrMetadata::default(), settings, None);
        assert_eq!(grade.plan().source, cine);
        // Naming nothing leaves the two authorities in charge, as before.
        let unset = Settings {
            gamut: None,
            ..settings
        };
        let grade = Grade::new(bt709(), &HdrMetadata::default(), unset, None);
        assert_eq!(grade.plan().source, Primaries::BT709);
        let grade = Grade::new(
            ColourDescription::default(),
            &HdrMetadata::default(),
            unset,
            None,
        );
        assert_eq!(grade.plan().source, Primaries::S_GAMUT3);
    }

    /// The player's help text promises an order — the log unfolds the codes, the
    /// tone curve fits the highlights, and a `.cube` or `.3dl` look is applied
    /// *after* them — so that order is what this test holds. Each leg is pinned
    /// on its own elsewhere; here they are walked together on the grey diagonal
    /// of the grid the settings bake at, where a channel is a single number:
    /// Sony's published constants unfold the code, the published Hable filmic
    /// response fits it, BT.709's OETF writes it back, and the cube's own text
    /// is walked by hand. Inputs are nodes of that grid (the settings' own 33),
    /// which is where a baked grid carries its curve without a step in between.
    /// The widest gap between the two orders there measures 0.617 — 157 codes —
    /// and the grid stands 2.1e-7 off the arithmetic, so what fails is the order
    /// and not a rounding that could fall either way.
    #[test]
    fn a_told_grade_applies_the_log_then_the_tone_map_then_the_cube() {
        // Three different shapes, so a leg sent to the wrong channel or dropped
        // on the floor shows up on one of them.
        const CUBE: &str = "LUT_1D_SIZE 5
0.00 0.00 1.00
0.25 0.60 0.70
0.60 0.25 0.35
0.85 0.90 0.10
1.00 1.00 0.00
";
        let nodes: Vec<[f64; 3]> = CUBE
            .lines()
            .filter(|line| {
                line.as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
            })
            .map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                [
                    parts[0].parse().expect("red"),
                    parts[1].parse().expect("green"),
                    parts[2].parse().expect("blue"),
                ]
            })
            .collect();
        assert_eq!(nodes.len(), 5);
        let cube = |code: f64, channel: usize| -> f64 {
            let pos = code.clamp(0.0, 1.0) * 4.0;
            let low = pos.floor() as usize;
            let high = (low + 1).min(4);
            let frac = pos - low as f64;
            nodes[low][channel] * (1.0 - frac) + nodes[high][channel] * frac
        };
        // Sony's S-Log3 summary: 18 % reflectance at code 420 of 1 023, one
        // and a half decades per 261.5 codes, and 0.01 of offset to subtract.
        let slog3 = |signal: f64| -> f64 {
            let cv = signal * 1023.0;
            if cv > 171.210_294_7 {
                10f64.powf((cv - 420.0) / 261.5) * 0.19 - 0.01
            } else {
                (cv - 95.0) * 0.011_25 / (171.210_294_7 - 95.0)
            }
        };
        // Hable's Uncharted 2 filmic response, as published, normalised by its
        // own value at the panel's peak — which is code 1.0 here, the file
        // naming no content peak of its own.
        let hable = |x: f64| -> f64 {
            let (a, b, c, d, e, f) = (0.15, 0.50, 0.10, 0.20, 0.02, 0.30);
            (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f
        };
        let fit = |light: f64| (hable(light) / hable(1.0)).clamp(0.0, 1.0);
        let written = |light: f64| -> f64 {
            if light <= 0.018 {
                4.5 * light
            } else {
                1.099 * light.max(0.0).powf(0.45) - 0.099
            }
        };
        // The promised order, and the one that would follow from wiring the
        // cube in front of the grade instead of behind it.
        let forward = |code: f64, channel: usize| cube(written(fit(slog3(code))), channel);
        let reversed = |code: f64, channel: usize| written(fit(slog3(cube(code, channel))));
        let settings = Settings {
            log: Some(Log::SLog3),
            tone_map: Some(ToneMap::Hable),
            ..Settings::video(DisplayTarget::sdr(240.0))
        };
        let grade = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            settings,
            Some(Lut::from_cube(CUBE).expect("a written 1D cube is a cube")),
        );
        let mut apart = 0.0f64;
        let mut worst = 0.0f64;
        for node in 0..=32 {
            let code = node as f64 / 32.0;
            let shown = grade.rgb([code as f32; 3]);
            for channel in 0..3 {
                let want = forward(code, channel);
                let got = f64::from(shown[channel]);
                apart = apart.max((want - reversed(code, channel)).abs());
                worst = worst.max((want - got).abs());
                assert!(
                    (want - got).abs() <= 1e-5,
                    "node {node} channel {channel}: shown {got} over {want}"
                );
            }
        }
        assert!(
            apart > 0.05,
            "the two orders never differ by more than {apart}, so this test could not tell them apart"
        );
        assert!(
            worst < 1e-5,
            "the grid is {worst} off the curve it bakes, which is more than a float rounding"
        );
    }

    /// Every code of every channel, both routes to the same byte.
    fn tables_match_the_grid(grade: &Grade) {
        assert!(grade.tables.is_some());
        for channel in 0..3 {
            for code in 0..=255u8 {
                let mut pixel = [0u8; 3];
                pixel[channel] = code;
                grade.apply(&mut pixel);
                let value = f32::from(code) / 255.0;
                let want = (grade.rgb([value, value, value])[channel] * 255.0).round() as i32;
                assert!(
                    (i32::from(pixel[channel]) - want).abs() <= 1,
                    "channel {channel} code {code}: {} over {want}",
                    pixel[channel]
                );
            }
        }
    }

    /// A conversion that only reshapes each channel's curve is 256 output bytes
    /// a channel, and the grid says the same thing node for node.
    #[test]
    fn a_curve_only_conversion_runs_off_byte_tables_that_the_grid_agrees_with() {
        let settings = Settings::video(DisplayTarget::sdr(240.0));
        // Gamma-2.2 codes onto the panel's own curve over the primaries it
        // already shows: nothing moves between channels, and nothing of the
        // plan's goes negative, so the grey axis answers for the whole cube.
        let gamma = ColourDescription {
            transfer: 4,
            ..bt709()
        };
        tables_match_the_grid(&Grade::new(gamma, &HdrMetadata::default(), settings, None));
        // A per-channel LUT is baked into the same three tables, not a fourth
        // lookup at the pixel.
        let contrast = {
            let ramp = vec![0.0, 0.3, 1.0];
            Lut1d {
                data: [ramp.clone(), ramp.clone(), ramp],
                domain_min: [0.0; 3],
                domain_max: [1.0; 3],
            }
        };
        tables_match_the_grid(&Grade::new(
            gamma,
            &HdrMetadata::default(),
            settings,
            Some(Lut::One(contrast)),
        ));
    }

    /// A camera log curve is the plan that looks separable and is not: no tone
    /// map runs, the camera's working gamut is the panel's own once the caller
    /// names BT.709 for both ends, and no LUT follows — every cheap clause says
    /// per-channel tables. The curve's `to_linear` is negative below the toe,
    /// and the gamut compress that keeps the grid inside its ends desaturates
    /// any negative channel toward luma, which reads the other two. So the
    /// measured guard has to fire, and what a whole pixel looks like is the
    /// grid's answer rather than three tables' — measured here at the pixel the
    /// tables would have got wrong, and by more than a code.
    #[test]
    fn a_camera_log_plan_gets_no_byte_tables() {
        for log in [Log::SLog3, Log::VLog, Log::CLog2, Log::LogC] {
            let grade = Grade::new(
                bt709(),
                &HdrMetadata::default(),
                Settings {
                    log: Some(log),
                    ..Settings::video(DisplayTarget::sdr(240.0))
                },
                None,
            );
            assert_eq!(grade.plan().tone_map, None);
            assert_eq!(grade.plan().source, grade.plan().dest);
            assert!(grade.lut().is_none());
            assert!(grade.tables.is_none(), "{log:?} was given the fast path");
            // The route that is left reads the whole pixel, so a saturated one
            // comes out where the grid says rather than where three independent
            // curves say.
            let pixel = [255u8, 0u8, 33u8];
            let mut shown = pixel;
            grade.apply(&mut shown);
            let codes = pixel.map(|code| f32::from(code) / 255.0);
            let want = grade
                .rgb(codes)
                .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
            assert_eq!(shown, want, "{log:?} graded a pixel off the grid");
            // And the shader is handed that same grid, not the tables that were
            // refused it.
            assert!(matches!(grade.shader_look(), Some(ShaderLook::Grid(_))));
        }
    }

    #[test]
    fn only_a_conversion_that_keeps_the_channels_apart_gets_tables() {
        let video = Settings::video(DisplayTarget::sdr(203.0));
        // A gamut change moves every output channel by all three inputs.
        let wide = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings {
                dest: Primaries::BT2020,
                ..video
            },
            None,
        );
        assert!(wide.tables.is_none());
        // So does a tone curve, which reads luma.
        let mapped = Grade::new(bt2100(16), &HdrMetadata::default(), video, None);
        assert!(mapped.plan().tone_map.is_some());
        assert!(mapped.tables.is_none());
        // HLG's scene-light OOTF mixes channels before any curve is written: the
        // same signal on an HDR panel, with nothing mapped and the gamut
        // untouched, still has no per-channel answer.
        let hlg = Grade::new(
            bt2100(18),
            &HdrMetadata::default(),
            Settings {
                to: Transfer::Hlg,
                dest: Primaries::BT2020,
                ..video
            },
            None,
        );
        assert_eq!(hlg.plan().tone_map, None);
        assert_eq!(hlg.plan().source, hlg.plan().dest);
        assert!(hlg.tables.is_none());
        // A grid-shaped LUT after the conversion ends the per-channel route...
        let cube_lut = Lut::Three(Lut3d::from_fn(4, |rgb| rgb.map(|v| 1.0 - v)));
        let graded = Grade::new(bt709(), &HdrMetadata::default(), video, Some(cube_lut));
        assert!(graded.tables.is_none());
        // ...while a per-channel LUT stays inside it, and the tables carry it.
        let desaturate = Lut1d {
            data: [
                vec![0.2, 0.5, 0.8],
                vec![0.0, 0.5, 1.0],
                vec![0.8, 0.5, 0.2],
            ],
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
        };
        let lifted = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            video,
            Some(Lut::One(desaturate)),
        );
        assert!(lifted.tables.is_some());
        assert!(!lifted.is_identity());
        tables_match_the_grid(&lifted);
    }

    #[test]
    fn a_grading_lut_is_applied_after_the_derived_conversion() {
        let flip_red = Lut::Three(Lut3d::from_fn(4, |rgb| [1.0 - rgb[0], rgb[1], rgb[2]]));
        let grade = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings::video(DisplayTarget::sdr(240.0)),
            Some(flip_red.clone()),
        );
        // The cube alone is the identity, so anything that moves came from the LUT.
        assert!(!grade.is_identity());
        assert_eq!(grade.lut(), Some(&flip_red));
        let out = grade.rgb([0.25, 0.5, 0.75]);
        assert!(close(out[0], 0.75, 1e-3), "{}", out[0]);
        assert!(close(out[1], 0.5, 1e-3) && close(out[2], 0.75, 1e-3));
    }

    #[test]
    fn a_curve_the_module_cannot_name_hands_its_codes_to_the_lut_as_they_came() {
        // The player's own assembly for a file whose curve fvid has no
        // implementation for: an unassigned transfer code (17) over BT.709
        // primaries, asked to leave the transfer at what the file states.
        let unnamed = ColourDescription {
            primaries: 1,
            transfer: 17,
            matrix: 1,
            full_range: false,
        };
        let settings = || {
            let mut s = Settings::video(DisplayTarget::sdr(240.0));
            s.to = unnamed.transfer_function();
            s.dest = unnamed.primary_set().unwrap();
            s
        };
        assert_eq!(settings().to, Transfer::Unknown);
        // With nothing to decode to and nothing to re-encode into, the codes are
        // the picture, so a grade that names no LUT costs nothing.
        let bare = Grade::new(unnamed, &HdrMetadata::default(), settings(), None);
        assert!(bare.is_identity());
        assert!(
            close(grey(&bare, 0.4)[0], 0.4, 1e-6),
            "{:?}",
            grey(&bare, 0.4)
        );
        // And a LUT lands on the code, not on a curve guessed under it: decoding
        // 0.4 as BT.709 first moves all three channels, the same probe reading
        // [0.826 869, 0.173 131, 0.173 131], where the passthrough touches only
        // the channel the LUT flips.
        let flip_red = Lut::Three(Lut3d::from_fn(4, |rgb| [1.0 - rgb[0], rgb[1], rgb[2]]));
        let graded = Grade::new(unnamed, &HdrMetadata::default(), settings(), Some(flip_red));
        let out = graded.rgb([0.4, 0.4, 0.4]);
        assert!(close(out[0], 0.6, 1e-3), "{}", out[0]);
        assert!(
            close(out[1], 0.4, 1e-3) && close(out[2], 0.4, 1e-3),
            "{out:?}"
        );
    }

    #[test]
    fn a_row_of_pixels_is_rewritten_in_place_and_a_identity_grade_skips_it() {
        let grade = Grade::new(bt709(), &HdrMetadata::default(), Settings::default(), None);
        let mut row = vec![0u8, 128, 255, 64, 64, 64];
        let before = row.clone();
        grade.apply(&mut row);
        assert_ne!(row, before);
        let want = (grey(&grade, 128.0 / 255.0)[0] * 255.0).round() as u8;
        assert_eq!(row[1], want);
        let plain = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings::video(DisplayTarget::sdr(240.0)),
            None,
        );
        let mut unchanged = row.clone();
        plain.apply(&mut unchanged);
        assert_eq!(unchanged, row);
    }

    #[test]
    fn a_buffer_that_is_not_whole_pixels_is_not_read_past_its_end() {
        let grade = Grade::new(bt709(), &HdrMetadata::default(), Settings::default(), None);
        let mut tail = vec![1u8, 2, 3, 4];
        grade.apply(&mut tail);
        assert_eq!(tail.len(), 4);
        // The stray byte is not a colour value, so no channel ever saw it.
        assert_eq!(tail[3], 4);
        grade.apply(&mut []);
    }

    /// BT.2100 PQ master with its own light figures.
    fn hdr10() -> HdrMetadata {
        HdrMetadata {
            light: ContentLight {
                max_cll: 1_000.0,
                max_fall: 400.0,
            },
            ..Default::default()
        }
    }

    /// A look with a shoulder of its own and cross-talk between two channels,
    /// so a grade that carries it is joined from two curved tables rather than
    /// one flat ramp.
    fn look() -> Lut {
        Lut::Three(Lut3d::from_fn(17, |v| {
            let s = |x: f32| {
                let smooth = x * x * (3.0 - 2.0 * x);
                x + 0.25 * (smooth - x)
            };
            [
                s(v[0]) + 0.05 * (v[1] - v[2]),
                s(v[1]),
                s(v[2]) - 0.05 * (v[1] - v[2]),
            ]
        }))
    }

    /// What a grade hands the fragment shader is the table the CPU reads, not a
    /// rebake of it: the plan's own nodes where the conversion mixes its channels,
    /// the byte tables where it does not. Both kinds are the same numbers, so both
    /// routes owe the caller the same bytes — which is what `player_gpu`'s readback
    /// tests hold them to.
    #[test]
    fn a_grade_hands_the_shader_the_table_it_reads() {
        let plain = mixing();
        let ShaderLook::Grid(grid) = plain.shader_look().expect("no LUT, one grid") else {
            panic!("a plan that moves between primaries has no byte tables");
        };
        let Lut::Three(cube) = &plain.cube else {
            panic!("a baked plan is a grid");
        };
        assert_eq!(grid.size, 33);
        assert_eq!(grid.data.len(), cube.data.len());
        for (a, b) in grid.data.iter().zip(&cube.data) {
            assert_eq!(a, b);
        }
        // A curve-only plan runs off three byte tables on the CPU, so the shader
        // is given exactly those bytes. Handing it the grid instead would be a
        // second, approximate colour decision where an exact one is available:
        // the tables answer all 256 codes, the grid only its nodes.
        let curve = Grade::new(
            ColourDescription {
                transfer: 4,
                ..bt709()
            },
            &HdrMetadata::default(),
            Settings::default(),
            None,
        );
        let tables = curve.tables.as_ref().expect("a curve alone");
        let ShaderLook::Tables(look) = curve.shader_look().expect("no LUT, one lookup") else {
            panic!("a grade with byte tables gives the shader those tables");
        };
        assert_eq!(&look, tables);
        // A camera-log plan looks like three tables and is not one, so it reaches
        // the shader as a grid — see `a_camera_log_plan_gets_no_byte_tables`.
        let log = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings {
                log: Some(Log::SLog3),
                ..Settings::default()
            },
            None,
        );
        assert!(log.tables.is_none());
        assert!(matches!(log.shader_look(), Some(ShaderLook::Grid(_))));
        // A LUT after the conversion is a second table, and no kind covers it.
        let chained = Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings::default(),
            Some(Lut::Three(Lut3d::from_fn(4, |rgb| rgb.map(|v| 1.0 - v)))),
        );
        assert!(chained.shader_look().is_none());
        // What one lookup is worth is decided the same way either it is asked.
        for grade in [&plain, &curve, &log, &chained] {
            assert_eq!(grade.is_shader_look(), grade.shader_look().is_some());
        }
        assert!(!plain.is_identity() && !curve.is_identity() && !log.is_identity());
    }

    /// A LUT after the baked grid cannot be folded into it, which is why
    /// [`Grade::shader_look`] refuses a chained grade instead of handing the
    /// shader a table that lies. The comparison is a composed grid read at the
    /// very edge length the grade was baked at, against the two-step read the
    /// grade itself makes, over 64³ sample points and every interpolation:
    /// nearest lands on the same node either way (0 of 786 432 channels moved),
    /// but the reads that mix nodes part by up to six codes at a 17 grid, four
    /// at 33 and three at 64 — and getting *bigger* does not close it, because
    /// the two routes interpolate different functions, not the same one at
    /// different resolutions.
    #[test]
    fn a_chained_grade_is_not_a_single_lookup() {
        let step = |code: u8| f32::from(code) / 255.0;
        for (size, nearest, trilinear, tetrahedral) in
            [(17usize, 0i32, 6i32, 6i32), (33, 0, 4, 5), (64, 0, 3, 3)]
        {
            for (interp, want) in [
                (Interpolation::Nearest, nearest),
                (Interpolation::Trilinear, trilinear),
                (Interpolation::Tetrahedral, tetrahedral),
            ] {
                let grade = Grade::new(
                    bt2100(16),
                    &hdr10(),
                    Settings {
                        size,
                        interpolation: interp,
                        ..Settings::video(DisplayTarget::sdr(100.0))
                    },
                    Some(look()),
                );
                assert!(
                    !grade.is_shader_look() && grade.shader_look().is_none(),
                    "{interp:?}: a grade with a LUT after its grid is two tables, not one"
                );
                let composed = Lut3d::from_fn(size, |rgb| grade.rgb(rgb));
                let mut worst = 0i32;
                for r in 0..64u8 {
                    for g in 0..64u8 {
                        for b in 0..64u8 {
                            let codes = [step(r * 4), step(g * 4), step(b * 4)];
                            let two = grade.rgb(codes);
                            let one = composed.sample(codes, interp);
                            for ch in 0..3 {
                                let a = (two[ch].clamp(0.0, 1.0) * 255.0).round() as i32;
                                let b2 = (one[ch].clamp(0.0, 1.0) * 255.0).round() as i32;
                                worst = worst.max((a - b2).abs());
                            }
                        }
                    }
                }
                assert_eq!(
                    worst, want,
                    "{interp:?} at a {size} grid: the codes one composed read is off by"
                );
            }
        }
    }
}

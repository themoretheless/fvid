//! libavfilter helpers for conversion filters fair-paired with FFmpeg vf.
use super::*;
use std::path::Path;
use std::ptr;

pub use fvid_media_info::{TransposeMode, PadRect, RotateAngle};

pub(crate) struct FilterGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    width: i32,
    height: i32,
    format: i32,
    time_base: AVRational,
    filter_name: &'static str,
    filter_args: String,
}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        // SAFETY: Graph owns linked filter contexts; free once if allocated.
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

/// Dual-input overlay graph: main `buffer` + `movie=` → `overlay` → `buffersink`.
pub(crate) struct OverlayGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    width: i32,
    height: i32,
    format: i32,
    time_base: AVRational,
    filename: String,
    x: i32,
    y: i32,
}

impl Drop for OverlayGraph {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

impl OverlayGraph {
    unsafe fn open(
        filename: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let movie = avfilter_get_by_name(c"movie".as_ptr());
            let overlay = avfilter_get_by_name(c"overlay".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || movie.is_null() || overlay.is_null() || buffersink.is_null() {
                return Err("overlay/movie filters unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("overlay graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filename: filename.to_owned(),
                x,
                y,
            };
            let args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create overlay main buffer",
            )?;
            let movie_ctx = avfilter_graph_alloc_filter(built.graph, movie, c"movie".as_ptr());
            if movie_ctx.is_null() {
                return Err("allocate movie filter failed".into());
            }
            // Key=value form avoids Drive: path separators being parsed as options.
            let movie_init = format!("filename='{filename}'");
            let movie_init = cstring(&movie_init)?;
            check(
                avfilter_init_str(movie_ctx, movie_init.as_ptr()),
                "initialize movie overlay source",
            )?;
            let mut overlay_ctx = ptr::null_mut();
            let overlay_args = format!("{x}:{y}");
            let overlay_args = cstring(&overlay_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut overlay_ctx,
                    overlay,
                    c"overlay".as_ptr(),
                    overlay_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create overlay filter",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create overlay sink",
            )?;
            check(
                avfilter_link(built.src, 0, overlay_ctx, 0),
                "link main buffer to overlay",
            )?;
            check(
                avfilter_link(movie_ctx, 0, overlay_ctx, 1),
                "link movie to overlay",
            )?;
            check(
                avfilter_link(overlay_ctx, 0, built.sink, 0),
                "link overlay to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure overlay graph",
            )?;
            Ok(built)
        }
    }

    fn matches(
        &self,
        filename: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
    ) -> bool {
        self.filename == filename
            && self.x == x
            && self.y == y
            && self.width == width
            && self.height == height
            && self.format == format
            && self.time_base.num == time_base.num
            && self.time_base.den == time_base.den
    }
}

/// Dual-input xfade graph: main `buffer` + `movie=` → `xfade` → `buffersink`.
pub(crate) struct XfadeGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    width: i32,
    height: i32,
    format: i32,
    time_base: AVRational,
    filename: String,
    transition: String,
    duration: String,
    offset: String,
}

impl Drop for XfadeGraph {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

impl XfadeGraph {
    unsafe fn open(
        filename: &str,
        transition: &str,
        duration: &str,
        offset: &str,
        width: i32,
        height: i32,
        format: i32,
        sample_aspect_ratio: AVRational,
        fps_num: i32,
        fps_den: i32,
    ) -> Result<Self> {
        unsafe {
            let fps_num = if fps_num > 0 { fps_num } else { 25 };
            let fps_den = if fps_den > 0 { fps_den } else { 1 };
            // CFR time base matching frame_rate (required by xfade).
            let time_base = AVRational {
                num: fps_den,
                den: fps_num,
            };
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let movie = avfilter_get_by_name(c"movie".as_ptr());
            let fps = avfilter_get_by_name(c"fps".as_ptr());
            let xfade = avfilter_get_by_name(c"xfade".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null()
                || movie.is_null()
                || fps.is_null()
                || xfade.is_null()
                || buffersink.is_null()
            {
                return Err("xfade/movie/fps filters unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("xfade graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filename: filename.to_owned(),
                transition: transition.to_owned(),
                duration: duration.to_owned(),
                offset: offset.to_owned(),
            };
            let args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}:frame_rate={fps_num}/{fps_den}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade main buffer",
            )?;
            let movie_ctx = avfilter_graph_alloc_filter(built.graph, movie, c"movie".as_ptr());
            if movie_ctx.is_null() {
                return Err("allocate movie filter failed".into());
            }
            let movie_init = format!("filename='{filename}'");
            let movie_init = cstring(&movie_init)?;
            check(
                avfilter_init_str(movie_ctx, movie_init.as_ptr()),
                "initialize movie xfade source",
            )?;
            // Align second input CFR/timebase with the main buffer.
            let mut fps_ctx = ptr::null_mut();
            let fps_args = format!("{fps_num}/{fps_den}");
            let fps_args = cstring(&fps_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut fps_ctx,
                    fps,
                    c"fps".as_ptr(),
                    fps_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create fps after movie for xfade",
            )?;
            let mut xfade_ctx = ptr::null_mut();
            let xfade_args = format!("transition={transition}:duration={duration}:offset={offset}");
            let xfade_args = cstring(&xfade_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut xfade_ctx,
                    xfade,
                    c"xfade".as_ptr(),
                    xfade_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade filter",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create xfade sink",
            )?;
            check(
                avfilter_link(built.src, 0, xfade_ctx, 0),
                "link main buffer to xfade",
            )?;
            check(avfilter_link(movie_ctx, 0, fps_ctx, 0), "link movie to fps")?;
            check(avfilter_link(fps_ctx, 0, xfade_ctx, 1), "link fps to xfade")?;
            check(
                avfilter_link(xfade_ctx, 0, built.sink, 0),
                "link xfade to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure xfade graph",
            )?;
            Ok(built)
        }
    }

    fn matches(
        &self,
        filename: &str,
        transition: &str,
        duration: &str,
        offset: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
    ) -> bool {
        self.filename == filename
            && self.transition == transition
            && self.duration == duration
            && self.offset == offset
            && self.width == width
            && self.height == height
            && self.format == format
            && self.time_base.num == time_base.num
            && self.time_base.den == time_base.den
    }

    fn filter_time_base(&self) -> AVRational {
        self.time_base
    }
}

impl FilterGraph {
    unsafe fn open(
        filter_name: &'static str,
        filter_args: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let filter = avfilter_get_by_name(cstring(filter_name)?.as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || filter.is_null() || buffersink.is_null() {
                return Err(format!(
                    "{filter_name} filter unavailable in linked libavfilter"
                ));
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("filter graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filter_name,
                filter_args: filter_args.to_owned(),
            };
            let args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer source",
            )?;
            let mut mid = ptr::null_mut();
            let mid_args = cstring(filter_args)?;
            let mid_name = cstring(filter_name)?;
            check(
                avfilter_graph_create_filter(
                    &mut mid,
                    filter,
                    mid_name.as_ptr(),
                    mid_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                &format!("create {filter_name} filter"),
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer sink",
            )?;
            check(
                avfilter_link(built.src, 0, mid, 0),
                &format!("link buffer to {filter_name}"),
            )?;
            check(
                avfilter_link(mid, 0, built.sink, 0),
                &format!("link {filter_name} to sink"),
            )?;
            check(
                configure_filter_graph(built.graph),
                &format!("configure {filter_name} graph"),
            )?;
            Ok(built)
        }
    }

    unsafe fn open_from_frame(
        filter_name: &'static str,
        filter_args: &str,
        frame: &AVFrame,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let space = if frame.colorspace != AVColorSpace_AVCOL_SPC_UNSPECIFIED {
                let name = av_color_space_name(frame.colorspace);
                if name.is_null() {
                    None
                } else {
                    Some(string(name))
                }
            } else {
                None
            };
            let range = if frame.color_range != AVColorRange_AVCOL_RANGE_UNSPECIFIED {
                let name = av_color_range_name(frame.color_range);
                if name.is_null() {
                    None
                } else {
                    Some(string(name))
                }
            } else {
                None
            };
            Self::open_with_color(
                filter_name,
                filter_args,
                frame.width,
                frame.height,
                frame.format,
                time_base,
                sample_aspect_ratio,
                space.as_deref(),
                range.as_deref(),
            )
        }
    }

    /// Like [`Self::open_from_frame`] but sets `frame_rate=` on the buffer (required by telecine).
    unsafe fn open_cfr_from_frame(
        filter_name: &'static str,
        filter_args: &str,
        frame: &AVFrame,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let space = if frame.colorspace != AVColorSpace_AVCOL_SPC_UNSPECIFIED {
                let name = av_color_space_name(frame.colorspace);
                if name.is_null() {
                    None
                } else {
                    Some(string(name))
                }
            } else {
                None
            };
            let range = if frame.color_range != AVColorRange_AVCOL_RANGE_UNSPECIFIED {
                let name = av_color_range_name(frame.color_range);
                if name.is_null() {
                    None
                } else {
                    Some(string(name))
                }
            } else {
                None
            };
            Self::open_with_color_and_fps(
                filter_name,
                filter_args,
                frame.width,
                frame.height,
                frame.format,
                time_base,
                sample_aspect_ratio,
                time_base.den.max(1),
                time_base.num.max(1),
                space.as_deref(),
                range.as_deref(),
            )
        }
    }

    unsafe fn open_with_color(
        filter_name: &'static str,
        filter_args: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
        colorspace: Option<&str>,
        color_range: Option<&str>,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let filter = avfilter_get_by_name(cstring(filter_name)?.as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || filter.is_null() || buffersink.is_null() {
                return Err(format!(
                    "{filter_name} filter unavailable in linked libavfilter"
                ));
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("filter graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filter_name,
                filter_args: filter_args.to_owned(),
            };
            let mut args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            if let Some(space) = colorspace {
                args.push_str(&format!(":colorspace={space}"));
            }
            if let Some(range) = color_range {
                args.push_str(&format!(":range={range}"));
            }
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer source",
            )?;
            let mut mid = ptr::null_mut();
            let mid_args = cstring(filter_args)?;
            let mid_name = cstring(filter_name)?;
            check(
                avfilter_graph_create_filter(
                    &mut mid,
                    filter,
                    mid_name.as_ptr(),
                    mid_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                &format!("create {filter_name} filter"),
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer sink",
            )?;
            check(
                avfilter_link(built.src, 0, mid, 0),
                &format!("link buffer to {filter_name}"),
            )?;
            check(
                avfilter_link(mid, 0, built.sink, 0),
                &format!("link {filter_name} to sink"),
            )?;
            check(
                configure_filter_graph(built.graph),
                &format!("configure {filter_name} graph"),
            )?;
            Ok(built)
        }
    }

    unsafe fn open_with_color_and_fps(
        filter_name: &'static str,
        filter_args: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
        fps_num: i32,
        fps_den: i32,
        colorspace: Option<&str>,
        color_range: Option<&str>,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let filter = avfilter_get_by_name(cstring(filter_name)?.as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || filter.is_null() || buffersink.is_null() {
                return Err(format!(
                    "{filter_name} filter unavailable in linked libavfilter"
                ));
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("filter graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filter_name,
                filter_args: filter_args.to_owned(),
            };
            let mut args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}:frame_rate={fps_num}/{fps_den}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            if let Some(space) = colorspace {
                args.push_str(&format!(":colorspace={space}"));
            }
            if let Some(range) = color_range {
                args.push_str(&format!(":range={range}"));
            }
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer source",
            )?;
            let mut mid = ptr::null_mut();
            let mid_args = cstring(filter_args)?;
            let mid_name = cstring(filter_name)?;
            check(
                avfilter_graph_create_filter(
                    &mut mid,
                    filter,
                    mid_name.as_ptr(),
                    mid_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                &format!("create {filter_name} filter"),
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer sink",
            )?;
            check(
                avfilter_link(built.src, 0, mid, 0),
                &format!("link buffer to {filter_name}"),
            )?;
            check(
                avfilter_link(mid, 0, built.sink, 0),
                &format!("link {filter_name} to sink"),
            )?;
            check(
                configure_filter_graph(built.graph),
                &format!("configure {filter_name} graph"),
            )?;
            Ok(built)
        }
    }

    /// Open `subtitles` with filename via `av_opt_set` (Windows paths contain `:`).
    unsafe fn open_subtitles(
        filename: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let filter = avfilter_get_by_name(c"subtitles".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || filter.is_null() || buffersink.is_null() {
                return Err("subtitles filter unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("filter graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                format,
                time_base,
                filter_name: "subtitles",
                filter_args: filename.to_owned(),
            };
            let args = format!(
                "video_size={width}x{height}:pix_fmt={format}:time_base={}/{}:pixel_aspect={}/{}",
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer source",
            )?;
            let mid = avfilter_graph_alloc_filter(built.graph, filter, c"subtitles".as_ptr());
            if mid.is_null() {
                return Err("allocate subtitles filter failed".into());
            }
            // Key=value form avoids treating Drive: as an option separator.
            let init = format!("filename='{filename}'");
            let init = cstring(&init)?;
            check(
                avfilter_init_str(mid, init.as_ptr()),
                "initialize subtitles filter",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer sink",
            )?;
            check(
                avfilter_link(built.src, 0, mid, 0),
                "link buffer to subtitles",
            )?;
            check(
                avfilter_link(mid, 0, built.sink, 0),
                "link subtitles to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure subtitles graph",
            )?;
            Ok(built)
        }
    }

    unsafe fn open_parsed_chain(
        filter_name: &'static str,
        chain: &str,
        frame: &AVFrame,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || buffersink.is_null() {
                return Err("buffer/buffersink unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("filter graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width: frame.width,
                height: frame.height,
                format: frame.format,
                time_base,
                filter_name,
                filter_args: chain.to_owned(),
            };
            let mut args = format!(
                "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
                frame.width,
                frame.height,
                frame.format,
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            if frame.colorspace != AVColorSpace_AVCOL_SPC_UNSPECIFIED {
                let name = av_color_space_name(frame.colorspace);
                if !name.is_null() {
                    args.push_str(&format!(":colorspace={}", string(name)));
                }
            }
            if frame.color_range != AVColorRange_AVCOL_RANGE_UNSPECIFIED {
                let name = av_color_range_name(frame.color_range);
                if !name.is_null() {
                    args.push_str(&format!(":range={}", string(name)));
                }
            }
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer source for parsed chain",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create buffer sink for parsed chain",
            )?;
            let mut outputs = avfilter_inout_alloc();
            let mut inputs = avfilter_inout_alloc();
            if outputs.is_null() || inputs.is_null() {
                avfilter_inout_free(&mut outputs);
                avfilter_inout_free(&mut inputs);
                return Err("filter inout allocation failed".into());
            }
            (*outputs).name = av_strdup(c"in".as_ptr());
            (*outputs).filter_ctx = built.src;
            (*outputs).pad_idx = 0;
            (*outputs).next = ptr::null_mut();
            (*inputs).name = av_strdup(c"out".as_ptr());
            (*inputs).filter_ctx = built.sink;
            (*inputs).pad_idx = 0;
            (*inputs).next = ptr::null_mut();
            let chain_c = cstring(chain)?;
            let code = avfilter_graph_parse_ptr(
                built.graph,
                chain_c.as_ptr(),
                &mut inputs,
                &mut outputs,
                ptr::null_mut(),
            );
            avfilter_inout_free(&mut inputs);
            avfilter_inout_free(&mut outputs);
            check(code, "parse filter chain")?;
            check(
                configure_filter_graph(built.graph),
                "configure parsed filter chain",
            )?;
            Ok(built)
        }
    }

    fn matches(
        &self,
        filter_name: &str,
        filter_args: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
    ) -> bool {
        self.filter_name == filter_name
            && self.filter_args == filter_args
            && self.width == width
            && self.height == height
            && self.format == format
            && self.time_base.num == time_base.num
            && self.time_base.den == time_base.den
    }
}

/// Dual-input framepack graph: consecutive frames alternate left/right inputs.
pub(crate) struct FramepackGraph {
    graph: *mut AVFilterGraph,
    src_left: *mut AVFilterContext,
    src_right: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    width: i32,
    height: i32,
    format: i32,
    time_base: AVRational,
    filter_args: String,
    feed_left: bool,
}

impl Drop for FramepackGraph {
    fn drop(&mut self) {
        unsafe {
            if !self.graph.is_null() {
                avfilter_graph_free(&mut self.graph);
            }
        }
    }
}

impl FramepackGraph {
    fn matches(
        &self,
        filter_args: &str,
        width: i32,
        height: i32,
        format: i32,
        time_base: AVRational,
    ) -> bool {
        self.filter_args == filter_args
            && self.width == width
            && self.height == height
            && self.format == format
            && self.time_base.num == time_base.num
            && self.time_base.den == time_base.den
    }

    unsafe fn open_from_frame(
        filter_args: &str,
        frame: &AVFrame,
        time_base: AVRational,
        sample_aspect_ratio: AVRational,
    ) -> Result<Self> {
        unsafe {
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let framepack = avfilter_get_by_name(c"framepack".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || framepack.is_null() || buffersink.is_null() {
                return Err("framepack filter unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("framepack graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src_left: ptr::null_mut(),
                src_right: ptr::null_mut(),
                sink: ptr::null_mut(),
                width: frame.width,
                height: frame.height,
                format: frame.format,
                time_base,
                filter_args: filter_args.to_owned(),
                feed_left: true,
            };
            let mut args = format!(
                "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
                frame.width,
                frame.height,
                frame.format,
                time_base.num,
                time_base.den,
                sample_aspect_ratio.num.max(1),
                sample_aspect_ratio.den.max(1)
            );
            if frame.colorspace != AVColorSpace_AVCOL_SPC_UNSPECIFIED {
                let name = av_color_space_name(frame.colorspace);
                if !name.is_null() {
                    args.push_str(&format!(":colorspace={}", string(name)));
                }
            }
            if frame.color_range != AVColorRange_AVCOL_RANGE_UNSPECIFIED {
                let name = av_color_range_name(frame.color_range);
                if !name.is_null() {
                    args.push_str(&format!(":range={}", string(name)));
                }
            }
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src_left,
                    buffersrc,
                    c"left".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create framepack left buffer",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src_right,
                    buffersrc,
                    c"right".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create framepack right buffer",
            )?;
            let mut mid = ptr::null_mut();
            let mid_args = cstring(filter_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut mid,
                    framepack,
                    c"framepack".as_ptr(),
                    mid_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create framepack filter",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create framepack sink",
            )?;
            check(
                avfilter_link(built.src_left, 0, mid, 0),
                "link left to framepack",
            )?;
            check(
                avfilter_link(built.src_right, 0, mid, 1),
                "link right to framepack",
            )?;
            check(
                avfilter_link(mid, 0, built.sink, 0),
                "link framepack to sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure framepack graph",
            )?;
            Ok(built)
        }
    }

    unsafe fn write_and_drain(
        &mut self,
        dst: *mut AVFrame,
        src: *mut AVFrame,
        mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            let target = if self.feed_left {
                self.src_left
            } else {
                self.src_right
            };
            check(
                av_buffersrc_write_frame(target, src),
                "feed framepack source",
            )?;
            self.feed_left = !self.feed_left;
            if self.feed_left {
                return Ok(());
            }
            loop {
                av_frame_unref(dst);
                let code = av_buffersink_get_frame(self.sink, dst);
                if code == -libc::EAGAIN || code == EOF {
                    return Ok(());
                }
                check(code, "receive framepack frame")?;
                let d = &mut *dst;
                d.pict_type = 0;
                d.quality = 0;
                emit(dst)?;
            }
        }
    }

    unsafe fn flush(
        &mut self,
        dst: *mut AVFrame,
        mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            check(
                av_buffersrc_write_frame(self.src_left, ptr::null()),
                "flush framepack left source",
            )?;
            check(
                av_buffersrc_write_frame(self.src_right, ptr::null()),
                "flush framepack right source",
            )?;
            loop {
                av_frame_unref(dst);
                let code = av_buffersink_get_frame(self.sink, dst);
                if code == -libc::EAGAIN || code == EOF {
                    return Ok(());
                }
                check(code, "receive flushed framepack frame")?;
                let d = &mut *dst;
                d.pict_type = 0;
                d.quality = 0;
                emit(dst)?;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FramepackFormat {
    Sbs,
    Tab,
    Lines,
    Columns,
    Frameseq,
}

fn parse_framepack_format(args: &str) -> Result<FramepackFormat> {
    if args.is_empty() {
        return Ok(FramepackFormat::Sbs);
    }
    for part in args.split(':') {
        match part {
            "sbs" => return Ok(FramepackFormat::Sbs),
            "tab" => return Ok(FramepackFormat::Tab),
            "lines" => return Ok(FramepackFormat::Lines),
            "columns" => return Ok(FramepackFormat::Columns),
            "frameseq" => return Ok(FramepackFormat::Frameseq),
            _ => {}
        }
        if let Some((key, value)) = part.split_once('=') {
            if key == "format" {
                return match value {
                    "sbs" => Ok(FramepackFormat::Sbs),
                    "tab" => Ok(FramepackFormat::Tab),
                    "lines" => Ok(FramepackFormat::Lines),
                    "columns" => Ok(FramepackFormat::Columns),
                    "frameseq" => Ok(FramepackFormat::Frameseq),
                    _ => Err(format!("unsupported framepack format: {value}").into()),
                };
            }
        }
    }
    Err(
        "framepack args must be empty (default sbs) or specify sbs|tab|lines|columns|frameseq or format="
            .into(),
    )
}

/// Apply a single libavfilter video filter into a reusable destination frame.
pub(crate) unsafe fn apply_video_filter(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    filter_name: &'static str,
    filter_args: &str,
) -> Result<()> {
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err(format!("{filter_name} requires a valid frame geometry"));
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(
                filter_name,
                filter_args,
                s.width,
                s.height,
                s.format,
                time_base,
            ),
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                filter_name,
                filter_args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph
            .as_mut()
            .ok_or_else(|| format!("{filter_name} graph missing"))?;
        check(
            av_buffersrc_write_frame(active.src, src),
            &format!("feed {filter_name} source"),
        )?;
        av_frame_unref(dst);
        check(
            av_buffersink_get_frame(active.sink, dst),
            &format!("receive {filter_name} frame"),
        )?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(())
    }
}

/// Apply FFmpeg-compatible `colorspace=` into a reusable destination frame.
/// `args` is the option string after `colorspace=` (e.g. `iall=bt470bg:all=bt709`).
pub(crate) unsafe fn colorspace_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorspace_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorspace", args) }
}

pub(crate) fn validate_colorspace_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 128 || args.contains('\0') {
        return Err("colorspace args must be 1..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_'))
    {
        return Err(
            "colorspace args may only contain [A-Za-z0-9=_:-] (FFmpeg colorspace= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `zscale=` then `format=` (fair-pairs `-vf zscale=ARGS,format=PIX`).
/// `args` is the option string after `zscale=` (e.g. `matrixin=bt470bg:matrix=bt709`).
pub(crate) unsafe fn zscale_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    out_pix_fmt: &str,
) -> Result<()> {
    validate_zscale_args(args)?;
    if out_pix_fmt.is_empty()
        || out_pix_fmt.len() > 32
        || !out_pix_fmt
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("zscale output pix_fmt must be a short FFmpeg format name".into());
    }
    let chain = format!("zscale={args},format={out_pix_fmt}");
    unsafe { apply_parsed_video_filter(graph, dst, src, "zscale", &chain) }
}

pub(crate) fn validate_zscale_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 128 || args.contains('\0') {
        return Err("zscale args must be 1..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_'))
    {
        return Err("zscale args may only contain [A-Za-z0-9=_:-] (FFmpeg zscale= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `tonemap=` then `format=` (fair-pairs `-vf tonemap=ARGS,format=PIX`).
/// `args` is the option string after `tonemap=` (e.g. `tonemap=hable`).
pub(crate) unsafe fn tonemap_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    out_pix_fmt: &str,
) -> Result<()> {
    validate_tonemap_args(args)?;
    if out_pix_fmt.is_empty()
        || out_pix_fmt.len() > 32
        || !out_pix_fmt
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("tonemap output pix_fmt must be a short FFmpeg format name".into());
    }
    let chain = format!("tonemap={args},format={out_pix_fmt}");
    unsafe { apply_parsed_video_filter(graph, dst, src, "tonemap", &chain) }
}

/// Apply a parsed libavfilter video chain (comma-separated) matching FFmpeg `-vf`.
pub(crate) unsafe fn apply_parsed_video_filter(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    filter_name: &'static str,
    chain: &str,
) -> Result<()> {
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err(format!("{filter_name} requires a valid frame geometry"));
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches(filter_name, chain, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_parsed_chain(
                filter_name,
                chain,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph
            .as_mut()
            .ok_or_else(|| format!("{filter_name} graph missing"))?;
        check(
            av_buffersrc_write_frame(active.src, src),
            &format!("feed {filter_name} source"),
        )?;
        av_frame_unref(dst);
        check(
            av_buffersink_get_frame(active.sink, dst),
            &format!("receive {filter_name} frame"),
        )?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(())
    }
}

pub(crate) fn validate_tonemap_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 128 || args.contains('\0') {
        return Err("tonemap args must be 1..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "tonemap args may only contain [A-Za-z0-9=.:_-] (FFmpeg tonemap= options)".into(),
        );
    }
    Ok(())
}

/// Blend successive frames via libavfilter `tblend=` (fair-pairs `-vf tblend=ARGS`).
/// Returns `false` when the filter holds the first frame (EAGAIN); FFmpeg emits N−1 frames.
pub(crate) unsafe fn tblend_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_tblend_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("tblend requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("tblend", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "tblend", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("tblend graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed tblend source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive tblend frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

pub(crate) fn validate_tblend_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 128 || args.contains('\0') {
        return Err("tblend args must be 1..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_'))
    {
        return Err("tblend args may only contain [A-Za-z0-9=_:-] (FFmpeg tblend= options)".into());
    }
    Ok(())
}

/// Feed one frame into `tmix=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn tmix_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_tmix_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("tmix requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("tmix", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "tmix", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("tmix graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed tmix source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive tmix frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `tmix=` after the last input frame and emit remaining output.
pub(crate) unsafe fn tmix_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush tmix source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed tmix frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_tmix_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 128 || args.contains('\0') {
        return Err("tmix args must be 1..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.' | b' '))
    {
        return Err("tmix args may only contain [A-Za-z0-9=.:_ -] (FFmpeg tmix= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `hqdn3d=` into a reusable destination frame.
/// `args` is the option string after `hqdn3d=` (e.g. `4:3:6:4.5`); empty uses filter defaults.
pub(crate) unsafe fn hqdn3d_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_hqdn3d_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "hqdn3d", args) }
}

/// Feed one frame into `yadif=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn yadif_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_yadif_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("yadif requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("yadif", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "yadif", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("yadif graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed yadif source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive yadif frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `yadif=` after the last input frame and emit remaining output.
pub(crate) unsafe fn yadif_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush yadif source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed yadif frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_yadif_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("yadif args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("yadif args may only contain [A-Za-z0-9=.:_-] (FFmpeg yadif= options)".into());
    }
    Ok(())
}

/// Feed one frame into `bwdif=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn bwdif_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_bwdif_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("bwdif requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("bwdif", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "bwdif", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("bwdif graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed bwdif source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive bwdif frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `bwdif=` after the last input frame and emit remaining output.
pub(crate) unsafe fn bwdif_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush bwdif source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed bwdif frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_bwdif_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("bwdif args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("bwdif args may only contain [A-Za-z0-9=.:_-] (FFmpeg bwdif= options)".into());
    }
    Ok(())
}

/// Feed one frame into `w3fdif=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn w3fdif_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_w3fdif_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("w3fdif requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("w3fdif", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "w3fdif", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("w3fdif graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed w3fdif source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive w3fdif frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `w3fdif=` after the last input frame and emit remaining output.
pub(crate) unsafe fn w3fdif_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush w3fdif source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed w3fdif frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_w3fdif_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("w3fdif args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "w3fdif args may only contain [A-Za-z0-9=.:_-] (FFmpeg w3fdif= options)".into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_hqdn3d_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("hqdn3d args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b':' | b'.' | b'-'))
    {
        return Err(
            "hqdn3d args may only contain [0-9:.-] (FFmpeg hqdn3d= luma/chroma strengths)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `gblur=` into a reusable destination frame.
/// `args` is the option string after `gblur=` (e.g. `sigma=1.5:steps=1`); empty uses defaults.
pub(crate) unsafe fn gblur_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_gblur_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "gblur", args) }
}

pub(crate) fn validate_gblur_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("gblur args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("gblur args may only contain [A-Za-z0-9=.:_-] (FFmpeg gblur= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `eq=` into a reusable destination frame.
/// `args` is the option string after `eq=` (e.g. `brightness=0.06:contrast=1.2`); empty uses defaults.
pub(crate) unsafe fn eq_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_eq_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "eq", args) }
}

pub(crate) fn validate_eq_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("eq args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("eq args may only contain [A-Za-z0-9=.:_-] (FFmpeg eq= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `unsharp=` into a reusable destination frame.
/// `args` is the option string after `unsharp=` (e.g. `5:5:1.0:5:5:0.0`); empty uses defaults.
pub(crate) unsafe fn unsharp_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_unsharp_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "unsharp", args) }
}

pub(crate) fn validate_unsharp_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("unsharp args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "unsharp args may only contain [A-Za-z0-9=.:_-] (FFmpeg unsharp= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `hue=` into a reusable destination frame.
/// `args` is the option string after `hue=` (e.g. `h=90:s=1.2`); empty uses defaults.
pub(crate) unsafe fn hue_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_hue_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "hue", args) }
}

pub(crate) fn validate_hue_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("hue args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("hue args may only contain [A-Za-z0-9=.:_-] (FFmpeg hue= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `avgblur=` into a reusable destination frame.
/// `args` is the option string after `avgblur=` (e.g. `sizeX=5:sizeY=5`); empty uses defaults.
pub(crate) unsafe fn avgblur_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_avgblur_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "avgblur", args) }
}

pub(crate) fn validate_avgblur_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("avgblur args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "avgblur args may only contain [A-Za-z0-9=.:_-] (FFmpeg avgblur= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `boxblur=` into a reusable destination frame.
/// `args` is the option string after `boxblur=` (e.g. `2:1`); empty uses defaults.
pub(crate) unsafe fn boxblur_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_boxblur_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "boxblur", args) }
}

pub(crate) fn validate_boxblur_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("boxblur args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "boxblur args may only contain [A-Za-z0-9=.:_-] (FFmpeg boxblur= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `smartblur=` into a reusable destination frame.
/// `args` is the option string after `smartblur=` (e.g. `lr=1.5:ls=-0.5`); empty uses defaults.
pub(crate) unsafe fn smartblur_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_smartblur_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "smartblur", args) }
}

pub(crate) fn validate_smartblur_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("smartblur args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "smartblur args may only contain [A-Za-z0-9=.:_-] (FFmpeg smartblur= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `sab=` into a reusable destination frame.
/// `args` is the option string after `sab=` (e.g. `lr=2:cr=2`); empty uses defaults.
pub(crate) unsafe fn sab_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_sab_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "sab", args) }
}

pub(crate) fn validate_sab_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("sab args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("sab args may only contain [A-Za-z0-9=.:_-] (FFmpeg sab= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `bilateral=` into a reusable destination frame.
/// `args` is the option string after `bilateral=` (e.g. `sigmaS=0.1:sigmaR=0.1`); empty uses defaults.
pub(crate) unsafe fn bilateral_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_bilateral_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "bilateral", args) }
}

pub(crate) fn validate_bilateral_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("bilateral args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "bilateral args may only contain [A-Za-z0-9=.:_-] (FFmpeg bilateral= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `cas=` into a reusable destination frame.
/// `args` is the option string after `cas=` (e.g. `strength=0.5`); empty uses defaults.
pub(crate) unsafe fn cas_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_cas_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "cas", args) }
}

pub(crate) fn validate_cas_args(args: &str) -> Result<()> {
    if args.len() > 64 || args.contains('\0') {
        return Err("cas args must be 0..=64 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("cas args may only contain [A-Za-z0-9=.:_-] (FFmpeg cas= options)".into());
    }
    Ok(())
}

/// Parse FFmpeg `epx=` scale factor (`n=2|3`, default 3).
pub(crate) fn parse_epx_factor(args: &str) -> Result<u32> {
    if args.is_empty() {
        return Ok(3);
    }
    let value = args.strip_prefix("n=").unwrap_or(args);
    let n: u32 = value
        .parse()
        .map_err(|_| "epx args must be n=2|3 or 2|3".to_string())?;
    if (2..=3).contains(&n) {
        Ok(n)
    } else {
        Err("epx scale factor n must be 2 or 3".into())
    }
}

/// Output size after `epx=` (integer upscale by n).
pub(crate) fn epx_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    let n = parse_epx_factor(args)?;
    width
        .checked_mul(n)
        .and_then(|w| height.checked_mul(n).map(|h| (w, h)))
        .ok_or_else(|| "epx output geometry overflow".into())
}

/// Apply FFmpeg-compatible `epx=` into a reusable destination frame.
/// `args` is the option string after `epx=` (e.g. `n=2`); empty uses defaults.
/// EPX only accepts packed RGB; fair-pair inserts `format=argb` before and
/// restores the source pixel format after (matches FFmpeg `-vf epx= -pix_fmt`).
pub(crate) unsafe fn epx_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_epx_args(args)?;
    let src_fmt = string(av_get_pix_fmt_name((*src).format));
    if src_fmt.is_empty() {
        return Err("epx requires a known source pixel format".into());
    }
    let chain = if args.is_empty() {
        format!("format=argb,epx,format={src_fmt}")
    } else {
        format!("format=argb,epx={args},format={src_fmt}")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "epx", &chain) }
}

pub(crate) fn validate_epx_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("epx args must be 0..=16 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'n' | b'='))
    {
        return Err("epx args may only contain [0-9n=] (FFmpeg epx= options)".into());
    }
    parse_epx_factor(args)?;
    Ok(())
}

/// Parse FFmpeg `hqx=` scale factor (`n=2|3|4`, default 3).
pub(crate) fn parse_hqx_factor(args: &str) -> Result<u32> {
    if args.is_empty() {
        return Ok(3);
    }
    let value = args.strip_prefix("n=").unwrap_or(args);
    let n: u32 = value
        .parse()
        .map_err(|_| "hqx args must be n=2|3|4 or 2|3|4".to_string())?;
    if (2..=4).contains(&n) {
        Ok(n)
    } else {
        Err("hqx scale factor n must be 2, 3, or 4".into())
    }
}

/// Output size after `hqx=` (integer upscale by n).
pub(crate) fn hqx_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    let n = parse_hqx_factor(args)?;
    width
        .checked_mul(n)
        .and_then(|w| height.checked_mul(n).map(|h| (w, h)))
        .ok_or_else(|| "hqx output geometry overflow".into())
}

/// Apply FFmpeg-compatible `hqx=` into a reusable destination frame.
/// `args` is the option string after `hqx=` (e.g. `n=2`); empty uses defaults.
/// HQX only accepts packed RGB; fair-pair inserts `format=argb` before and
/// restores the source pixel format after (matches FFmpeg `-vf hqx= -pix_fmt`).
pub(crate) unsafe fn hqx_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_hqx_args(args)?;
    let src_fmt = string(av_get_pix_fmt_name((*src).format));
    if src_fmt.is_empty() {
        return Err("hqx requires a known source pixel format".into());
    }
    let chain = if args.is_empty() {
        format!("format=argb,hqx,format={src_fmt}")
    } else {
        format!("format=argb,hqx={args},format={src_fmt}")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "hqx", &chain) }
}

pub(crate) fn validate_hqx_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("hqx args must be 0..=16 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'n' | b'='))
    {
        return Err("hqx args may only contain [0-9n=] (FFmpeg hqx= options)".into());
    }
    parse_hqx_factor(args)?;
    Ok(())
}

/// Parse FFmpeg `xbr=` scale factor (`n=2|3|4`, default 3).
pub(crate) fn parse_xbr_factor(args: &str) -> Result<u32> {
    if args.is_empty() {
        return Ok(3);
    }
    let value = args.strip_prefix("n=").unwrap_or(args);
    let n: u32 = value
        .parse()
        .map_err(|_| "xbr args must be n=2|3|4 or 2|3|4".to_string())?;
    if (2..=4).contains(&n) {
        Ok(n)
    } else {
        Err("xbr scale factor n must be 2, 3, or 4".into())
    }
}

/// Output size after `xbr=` (integer upscale by n).
pub(crate) fn xbr_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    let n = parse_xbr_factor(args)?;
    width
        .checked_mul(n)
        .and_then(|w| height.checked_mul(n).map(|h| (w, h)))
        .ok_or_else(|| "xbr output geometry overflow".into())
}

/// Apply FFmpeg-compatible `xbr=` into a reusable destination frame.
/// `args` is the option string after `xbr=` (e.g. `n=2`); empty uses defaults.
/// xBR only accepts packed RGB; fair-pair inserts `format=argb` before and
/// restores the source pixel format after (matches FFmpeg `-vf xbr= -pix_fmt`).
pub(crate) unsafe fn xbr_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_xbr_args(args)?;
    let src_fmt = string(av_get_pix_fmt_name((*src).format));
    if src_fmt.is_empty() {
        return Err("xbr requires a known source pixel format".into());
    }
    let chain = if args.is_empty() {
        format!("format=argb,xbr,format={src_fmt}")
    } else {
        format!("format=argb,xbr={args},format={src_fmt}")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "xbr", &chain) }
}

pub(crate) fn validate_xbr_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("xbr args must be 0..=16 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'n' | b'='))
    {
        return Err("xbr args may only contain [0-9n=] (FFmpeg xbr= options)".into());
    }
    parse_xbr_factor(args)?;
    Ok(())
}

/// Output size after `super2xsai` (always 2× upscale).
pub(crate) fn super2xsai_output_size(width: u32, height: u32, _args: &str) -> Result<(u32, u32)> {
    width
        .checked_mul(2)
        .and_then(|w| height.checked_mul(2).map(|h| (w, h)))
        .ok_or_else(|| "super2xsai output geometry overflow".into())
}

/// Apply FFmpeg-compatible `super2xsai` into a reusable destination frame.
/// `args` is unused (filter has no options); empty uses defaults.
/// Super2xSaI only accepts packed RGB; fair-pair inserts `format=argb` before and
/// restores the source pixel format after (matches FFmpeg `-vf super2xsai -pix_fmt`).
pub(crate) unsafe fn super2xsai_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_super2xsai_args(args)?;
    let src_fmt = string(av_get_pix_fmt_name((*src).format));
    if src_fmt.is_empty() {
        return Err("super2xsai requires a known source pixel format".into());
    }
    let chain = if args.is_empty() {
        format!("format=argb,super2xsai,format={src_fmt}")
    } else {
        format!("format=argb,super2xsai={args},format={src_fmt}")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "super2xsai", &chain) }
}

pub(crate) fn validate_super2xsai_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("super2xsai args must be 0..=16 bytes without NUL".into());
    }
    if !args.is_empty() {
        return Err("super2xsai accepts no options".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `il=` into a reusable destination frame.
/// `args` is the option string after `il=` (e.g. `l=d:c=d`); empty uses defaults.
pub(crate) unsafe fn il_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_il_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "il", args) }
}

pub(crate) fn validate_il_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("il args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("il args may only contain [A-Za-z0-9=.:_-] (FFmpeg il= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `kerndeint=` into a reusable destination frame.
/// `args` is the option string after `kerndeint=` (e.g. `thresh=10`); empty uses defaults.
pub(crate) unsafe fn kerndeint_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_kerndeint_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "kerndeint", args) }
}

pub(crate) fn validate_kerndeint_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("kerndeint args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "kerndeint args may only contain [A-Za-z0-9=.:_-] (FFmpeg kerndeint= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `estdif=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn estdif_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_estdif_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("estdif requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("estdif", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "estdif", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("estdif graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed estdif source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive estdif frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `estdif=` after the last input frame and emit remaining output.
pub(crate) unsafe fn estdif_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush estdif source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed estdif frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_estdif_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("estdif args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "estdif args may only contain [A-Za-z0-9=.:_-] (FFmpeg estdif= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `tinterlace=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn tinterlace_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_tinterlace_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("tinterlace requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("tinterlace", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "tinterlace",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("tinterlace graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed tinterlace source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive tinterlace frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `tinterlace=` after the last input frame and emit remaining output.
pub(crate) unsafe fn tinterlace_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush tinterlace source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed tinterlace frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_tinterlace_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("tinterlace args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "tinterlace args may only contain [A-Za-z0-9=.:_-] (FFmpeg tinterlace= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `separatefields` and emit each produced field (0..N).
pub(crate) unsafe fn separatefields_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_separatefields_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("separatefields requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(
                "separatefields",
                args,
                s.width,
                s.height,
                s.format,
                time_base,
            ),
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "separatefields",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("separatefields graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed separatefields source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive separatefields frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `separatefields` after the last input frame and emit remaining output.
pub(crate) unsafe fn separatefields_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush separatefields source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed separatefields frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_separatefields_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("separatefields args must be 0..=16 bytes without NUL".into());
    }
    if !args.is_empty() {
        return Err("separatefields accepts no options".into());
    }
    Ok(())
}

/// Output size after `separatefields` (height halved; width unchanged).
pub(crate) fn separatefields_output_size(
    width: u32,
    height: u32,
    args: &str,
) -> Result<(u32, u32)> {
    validate_separatefields_args(args)?;
    let out_h = height / 2;
    if out_h == 0 {
        return Err("separatefields requires height >= 2".into());
    }
    Ok((width, out_h))
}

/// Feed one frame into `weave` and emit each produced frame (0..N).
pub(crate) unsafe fn weave_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_weave_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("weave requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("weave", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "weave", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("weave graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed weave source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive weave frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `weave` after the last input frame and emit remaining output.
pub(crate) unsafe fn weave_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush weave source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed weave frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_weave_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("weave args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("weave args may only contain [A-Za-z0-9=.:_-] (FFmpeg weave= options)".into());
    }
    Ok(())
}

/// Output size after `weave` (height doubled; width unchanged).
pub(crate) fn weave_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_weave_args(args)?;
    let out_h = height
        .checked_mul(2)
        .ok_or("weave output height overflow")?;
    Ok((width, out_h))
}

/// Feed one frame into `doubleweave` and emit each produced frame (0..N).
pub(crate) unsafe fn doubleweave_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_doubleweave_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("doubleweave requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("doubleweave", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "doubleweave",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("doubleweave graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed doubleweave source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive doubleweave frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `doubleweave` after the last input frame and emit remaining output.
pub(crate) unsafe fn doubleweave_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush doubleweave source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed doubleweave frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_doubleweave_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("doubleweave args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "doubleweave args may only contain [A-Za-z0-9=.:_-] (FFmpeg doubleweave= options)"
                .into(),
        );
    }
    Ok(())
}

/// Output size after `doubleweave` (height doubled; width unchanged).
pub(crate) fn doubleweave_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_doubleweave_args(args)?;
    let out_h = height
        .checked_mul(2)
        .ok_or("doubleweave output height overflow")?;
    Ok((width, out_h))
}

/// Feed one frame into `framepack` (alternating left/right) and emit each produced frame (0..N).
pub(crate) unsafe fn framepack_push_frame(
    graph: &mut Option<FramepackGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_framepack_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("framepack requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(args, s.width, s.height, s.format, time_base),
            None => true,
        };
        if needs_new {
            *graph = Some(FramepackGraph::open_from_frame(args, s, time_base, sar)?);
        }
        let active = graph.as_mut().ok_or("framepack graph missing")?;
        active.write_and_drain(dst, src, |packed| emit(packed))?;
        Ok(())
    }
}

/// Flush `framepack` after the last input frame and emit remaining output.
pub(crate) unsafe fn framepack_flush(
    graph: &mut Option<FramepackGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe { active.flush(dst, |packed| emit(packed)) }
}

pub(crate) fn validate_framepack_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("framepack args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "framepack args may only contain [A-Za-z0-9=.:_-] (FFmpeg framepack= options)".into(),
        );
    }
    let _ = parse_framepack_format(args)?;
    Ok(())
}

/// Output size after `framepack=` (matches FFmpeg vf_framepack config_output geometry).
pub(crate) fn framepack_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_framepack_args(args)?;
    let mode = parse_framepack_format(args)?;
    match mode {
        FramepackFormat::Sbs | FramepackFormat::Columns => {
            let out_w = width
                .checked_mul(2)
                .ok_or("framepack output width overflow")?;
            Ok((out_w, height))
        }
        FramepackFormat::Tab | FramepackFormat::Lines => {
            let out_h = height
                .checked_mul(2)
                .ok_or("framepack output height overflow")?;
            Ok((width, out_h))
        }
        FramepackFormat::Frameseq => Ok((width, height)),
    }
}

/// Feed one frame into `telecine=` and emit each produced frame (0..N).
pub(crate) unsafe fn telecine_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_telecine_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("telecine requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("telecine", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "telecine", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("telecine graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed telecine source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive telecine frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `telecine=` after the last input frame and emit remaining output.
pub(crate) unsafe fn telecine_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush telecine source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed telecine frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_telecine_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("telecine args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "telecine args may only contain [A-Za-z0-9=.:_-] (FFmpeg telecine= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `pullup=` and emit each produced frame (0..N).
pub(crate) unsafe fn pullup_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_pullup_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("pullup requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("pullup", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "pullup", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("pullup graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed pullup source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive pullup frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `pullup=` after the last input frame and emit remaining output.
pub(crate) unsafe fn pullup_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush pullup source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed pullup frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_pullup_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("pullup args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "pullup args may only contain [A-Za-z0-9=.:_-] (FFmpeg pullup= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `decimate=` and emit each produced frame (0..N).
pub(crate) unsafe fn decimate_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_decimate_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("decimate requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("decimate", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "decimate", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("decimate graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed decimate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive decimate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `decimate=` after the last input frame and emit remaining output.
pub(crate) unsafe fn decimate_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush decimate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed decimate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_decimate_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("decimate args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "decimate args may only contain [A-Za-z0-9=.:_-] (FFmpeg decimate= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `mpdecimate=` and emit each produced frame (0..N).
pub(crate) unsafe fn mpdecimate_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_mpdecimate_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("mpdecimate requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("mpdecimate", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "mpdecimate",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("mpdecimate graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed mpdecimate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive mpdecimate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `mpdecimate=` after the last input frame and emit remaining output.
pub(crate) unsafe fn mpdecimate_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush mpdecimate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed mpdecimate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_mpdecimate_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("mpdecimate args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "mpdecimate args may only contain [A-Za-z0-9=.:_-] (FFmpeg mpdecimate= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `framestep=` and emit each produced frame (0..N).
pub(crate) unsafe fn framestep_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_framestep_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("framestep requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("framestep", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "framestep",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("framestep graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed framestep source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive framestep frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `framestep=` after the last input frame and emit remaining output.
pub(crate) unsafe fn framestep_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush framestep source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed framestep frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_framestep_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("framestep args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "framestep args may only contain [A-Za-z0-9=.:_-] (FFmpeg framestep= options)".into(),
        );
    }
    Ok(())
}

/// Run `framestep=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_framestep_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        framestep_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

/// Parse `tile=` grid dimensions from shorthand `CxR` or `layout=CxR`.
fn parse_tile_layout(args: &str) -> Result<(u32, u32)> {
    let parse_cxr = |s: &str| -> Result<(u32, u32)> {
        let (cols, rows) = s
            .split_once('x')
            .ok_or("tile layout must be CxR (e.g. 2x2 or layout=2x2)")?;
        let cols: u32 = cols
            .parse()
            .map_err(|_| "tile layout columns must be a positive integer")?;
        let rows: u32 = rows
            .parse()
            .map_err(|_| "tile layout rows must be a positive integer")?;
        if cols == 0 || rows == 0 {
            return Err("tile layout columns and rows must be positive".into());
        }
        Ok((cols, rows))
    };
    if args.is_empty() {
        return Ok((6, 5));
    }
    for part in args.split(':') {
        if let Some(val) = part.strip_prefix("layout=") {
            return parse_cxr(val);
        }
        if !part.contains('=') && part.contains('x') {
            return parse_cxr(part);
        }
    }
    parse_cxr(args)
}

pub(crate) fn validate_tile_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("tile args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("tile args may only contain [A-Za-z0-9=.:_-] (FFmpeg tile= options)".into());
    }
    let _ = parse_tile_layout(args)?;
    Ok(())
}

/// Output size after `tile=` (matches FFmpeg vf_tile config_output geometry).
pub(crate) fn tile_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_tile_args(args)?;
    let (cols, rows) = parse_tile_layout(args)?;
    let out_w = width
        .checked_mul(cols)
        .ok_or("tile output width overflow")?;
    let out_h = height
        .checked_mul(rows)
        .ok_or("tile output height overflow")?;
    Ok((out_w, out_h))
}

/// Feed one frame into `tile=` and emit each produced frame (0..N).
pub(crate) unsafe fn tile_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_tile_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("tile requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("tile", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "tile", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("tile graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed tile source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive tile frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `tile=` after the last input frame and emit remaining output.
pub(crate) unsafe fn tile_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush tile source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed tile frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `tile=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_tile_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        tile_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

/// Parse `untile=` grid dimensions from shorthand `CxR` or `layout=CxR`.
fn parse_untile_layout(args: &str) -> Result<(u32, u32)> {
    let parse_cxr = |s: &str| -> Result<(u32, u32)> {
        let (cols, rows) = s
            .split_once('x')
            .ok_or("untile layout must be CxR (e.g. 2x2 or layout=2x2)")?;
        let cols: u32 = cols
            .parse()
            .map_err(|_| "untile layout columns must be a positive integer")?;
        let rows: u32 = rows
            .parse()
            .map_err(|_| "untile layout rows must be a positive integer")?;
        if cols == 0 || rows == 0 {
            return Err("untile layout columns and rows must be positive".into());
        }
        Ok((cols, rows))
    };
    if args.is_empty() {
        return Ok((6, 5));
    }
    for part in args.split(':') {
        if let Some(val) = part.strip_prefix("layout=") {
            return parse_cxr(val);
        }
        if !part.contains('=') && part.contains('x') {
            return parse_cxr(part);
        }
    }
    parse_cxr(args)
}

pub(crate) fn validate_untile_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("untile args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "untile args may only contain [A-Za-z0-9=.:_-] (FFmpeg untile= options)".into(),
        );
    }
    let _ = parse_untile_layout(args)?;
    Ok(())
}

/// Output size after `untile=` (matches FFmpeg vf_untile config_output geometry).
pub(crate) fn untile_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_untile_args(args)?;
    let (cols, rows) = parse_untile_layout(args)?;
    if width % cols != 0 || height % rows != 0 {
        return Err(format!(
            "untile layout {cols}x{rows} requires input dimensions divisible by grid size (got {width}x{height})"
        )
        .into());
    }
    Ok((width / cols, height / rows))
}

/// Feed one frame into `untile=` and emit each produced frame (0..N).
pub(crate) unsafe fn untile_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_untile_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("untile requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("untile", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "untile", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("untile graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed untile source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive untile frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `untile=` after the last input frame and emit remaining output.
pub(crate) unsafe fn untile_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush untile source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed untile frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `untile=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_untile_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        untile_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

pub(crate) fn validate_shuffleframes_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("shuffleframes args must be 0..=128 bytes without NUL".into());
    }
    if !args.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.' | b' ' | b'|')
    }) || args
        .bytes()
        .any(|b| matches!(b, b';' | b'\'' | b'"' | b'`' | b'&' | b'$'))
    {
        return Err(
            "shuffleframes args may only contain [A-Za-z0-9=.:_| -] (FFmpeg shuffleframes= options)"
                .into(),
        );
    }
    Ok(())
}

/// Feed one frame into `shuffleframes=` and emit each produced frame (0..N).
pub(crate) unsafe fn shuffleframes_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_shuffleframes_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("shuffleframes requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(
                "shuffleframes",
                args,
                s.width,
                s.height,
                s.format,
                time_base,
            ),
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "shuffleframes",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("shuffleframes graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed shuffleframes source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive shuffleframes frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `shuffleframes=` after the last input frame and emit remaining output.
pub(crate) unsafe fn shuffleframes_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush shuffleframes source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed shuffleframes frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `shuffleframes=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_shuffleframes_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        shuffleframes_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

pub(crate) fn validate_reverse_args(args: &str) -> Result<()> {
    if args.len() > 16 || args.contains('\0') {
        return Err("reverse args must be 0..=16 bytes without NUL".into());
    }
    if !args.is_empty() {
        return Err("reverse accepts no options".into());
    }
    Ok(())
}

/// Feed one frame into `reverse` and emit each produced frame (0..N).
pub(crate) unsafe fn reverse_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_reverse_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("reverse requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("reverse", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "reverse", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("reverse graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed reverse source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive reverse frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `reverse` after the last input frame and emit remaining output.
pub(crate) unsafe fn reverse_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush reverse source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed reverse frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `reverse` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_reverse_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        reverse_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

pub(crate) fn validate_loop_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("loop args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("loop args may only contain [A-Za-z0-9=.:_-] (FFmpeg loop= options)".into());
    }
    Ok(())
}

/// Feed one frame into `loop=` and emit each produced frame (0..N).
pub(crate) unsafe fn loop_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_loop_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("loop requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("loop", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "loop", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("loop graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed loop source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive loop frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `loop=` after the last input frame and emit remaining output.
pub(crate) unsafe fn loop_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush loop source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed loop frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `loop=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_loop_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        loop_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

pub(crate) fn validate_thumbnail_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("thumbnail args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "thumbnail args may only contain [A-Za-z0-9=.:_-] (FFmpeg thumbnail= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `thumbnail=` and emit each produced frame (0..N).
pub(crate) unsafe fn thumbnail_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    validate_thumbnail_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("thumbnail requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("thumbnail", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_cfr_from_frame(
                "thumbnail",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("thumbnail graph missing")?;
        if s.time_base.num <= 0 || s.time_base.den <= 0 {
            (*src).time_base = time_base;
        }
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed thumbnail source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive thumbnail frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `thumbnail=` after the last input frame and emit remaining output.
pub(crate) unsafe fn thumbnail_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush thumbnail source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed thumbnail frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Run `thumbnail=` when `args` is set, otherwise pass the frame through.
pub(crate) unsafe fn push_thumbnail_or_emit(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = args {
        thumbnail_push_frame(graph, dst, src, args, emit)
    } else {
        emit(src)
    }
}

/// Apply FFmpeg-compatible `freezedetect=` into a reusable destination frame.
/// `args` is the option string after `freezedetect=` (e.g. `n=0.001:d=0.1`); empty uses defaults.
pub(crate) unsafe fn freezedetect_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_freezedetect_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "freezedetect", args) }
}

pub(crate) fn validate_freezedetect_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("freezedetect args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "freezedetect args may only contain [A-Za-z0-9=.:_-] (FFmpeg freezedetect= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `phase=` into a reusable destination frame.
/// `args` is the option string after `phase=` (e.g. `mode=t`); empty uses defaults.
pub(crate) unsafe fn phase_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_phase_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "phase", args) }
}

pub(crate) fn validate_phase_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("phase args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("phase args may only contain [A-Za-z0-9=.:_-] (FFmpeg phase= options)".into());
    }
    Ok(())
}

/// Feed one frame into `phase=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn phase_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_phase_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("phase requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("phase", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "phase", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("phase graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed phase source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive phase frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `phase=` after the last input frame and emit remaining output.
pub(crate) unsafe fn phase_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush phase source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed phase frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Apply `phase=` one-in/one-out; switches to push mode when the first frame buffers.
pub(crate) unsafe fn phase_apply_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    push_mode: &mut bool,
) -> Result<bool> {
    if *push_mode {
        return unsafe { phase_push_frame(graph, dst, src, args) };
    }
    validate_phase_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("phase requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("phase", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "phase", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("phase graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed phase source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            *push_mode = true;
            return Ok(false);
        }
        check(code, "receive phase frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Apply FFmpeg-compatible `vignette=` into a reusable destination frame.
/// `args` is the option string after `vignette=` (e.g. `angle=PI/4`); empty uses defaults.
pub(crate) unsafe fn vignette_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_vignette_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "vignette", args) }
}

pub(crate) fn validate_vignette_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("vignette args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.' | b'/'))
    {
        return Err(
            "vignette args may only contain [A-Za-z0-9=.:_/-] (FFmpeg vignette= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `curves=` into a reusable destination frame.
/// `args` is the option string after `curves=` (e.g. `preset=vintage`); empty uses defaults.
pub(crate) unsafe fn curves_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_curves_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "curves", args) }
}

pub(crate) fn validate_curves_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("curves args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "curves args may only contain [A-Za-z0-9=.:_-] (FFmpeg curves= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorbalance=` into a reusable destination frame.
/// `args` is the option string after `colorbalance=` (e.g. `rs=.1:gs=.05:bs=-.1`); empty uses defaults.
pub(crate) unsafe fn colorbalance_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorbalance_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorbalance", args) }
}

pub(crate) fn validate_colorbalance_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorbalance args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorbalance args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorbalance= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorlevels=` into a reusable destination frame.
/// `args` is the option string after `colorlevels=` (e.g. `rimin=0.1:gimin=0.1:bimin=0.1`); empty uses defaults.
pub(crate) unsafe fn colorlevels_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorlevels_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorlevels", args) }
}

pub(crate) fn validate_colorlevels_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorlevels args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorlevels args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorlevels= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorchannelmixer=` into a reusable destination frame.
/// `args` is the option string after `colorchannelmixer=` (e.g. `rr=1.1:gg=0.9:bb=1.0`); empty uses defaults.
pub(crate) unsafe fn colorchannelmixer_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorchannelmixer_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorchannelmixer", args) }
}

pub(crate) fn validate_colorchannelmixer_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorchannelmixer args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorchannelmixer args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorchannelmixer= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `negate` into a reusable destination frame.
/// `args` is empty (defaults) or `1` to also negate alpha (`negate=negate_alpha=1`).
pub(crate) unsafe fn negate_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_negate_args(args)?;
    let filter_args = if args.is_empty() || args == "0" {
        ""
    } else {
        "negate_alpha=1"
    };
    unsafe { apply_video_filter(graph, dst, src, "negate", filter_args) }
}

pub(crate) fn validate_negate_args(args: &str) -> Result<()> {
    if args.len() > 8 || args.contains('\0') {
        return Err("negate args must be 0..=8 bytes without NUL".into());
    }
    if !(args.is_empty() || args == "0" || args == "1") {
        return Err("negate args must be empty, 0, or 1 (alpha)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `edgedetect=` into a reusable destination frame.
/// `args` is the option string after `edgedetect=` (e.g. `mode=colormix`); empty uses defaults.
pub(crate) unsafe fn edgedetect_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_edgedetect_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "edgedetect", args) }
}

pub(crate) fn validate_edgedetect_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("edgedetect args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "edgedetect args may only contain [A-Za-z0-9=.:_-] (FFmpeg edgedetect= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `sobel=` into a reusable destination frame.
/// `args` is the option string after `sobel=` (e.g. `scale=2:delta=10`); empty uses defaults.
pub(crate) unsafe fn sobel_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_sobel_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "sobel", args) }
}

pub(crate) fn validate_sobel_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("sobel args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("sobel args may only contain [A-Za-z0-9=.:_-] (FFmpeg sobel= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `prewitt=` into a reusable destination frame.
/// `args` is the option string after `prewitt=` (e.g. `scale=2:delta=10`); empty uses defaults.
pub(crate) unsafe fn prewitt_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_prewitt_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "prewitt", args) }
}

pub(crate) fn validate_prewitt_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("prewitt args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "prewitt args may only contain [A-Za-z0-9=.:_-] (FFmpeg prewitt= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `roberts=` into a reusable destination frame.
/// `args` is the option string after `roberts=` (e.g. `scale=2:delta=10`); empty uses defaults.
pub(crate) unsafe fn roberts_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_roberts_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "roberts", args) }
}

pub(crate) fn validate_roberts_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("roberts args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "roberts args may only contain [A-Za-z0-9=.:_-] (FFmpeg roberts= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `kirsch=` into a reusable destination frame.
/// `args` is the option string after `kirsch=` (e.g. `scale=2:delta=10`); empty uses defaults.
pub(crate) unsafe fn kirsch_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_kirsch_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "kirsch", args) }
}

pub(crate) fn validate_kirsch_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("kirsch args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "kirsch args may only contain [A-Za-z0-9=.:_-] (FFmpeg kirsch= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `scharr=` into a reusable destination frame.
/// `args` is the option string after `scharr=` (e.g. `scale=2:delta=10`); empty uses defaults.
pub(crate) unsafe fn scharr_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_scharr_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "scharr", args) }
}

pub(crate) fn validate_scharr_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("scharr args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "scharr args may only contain [A-Za-z0-9=.:_-] (FFmpeg scharr= options)".into(),
        );
    }
    Ok(())
}

/// Feed one frame into `atadenoise=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn atadenoise_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_atadenoise_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("atadenoise requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("atadenoise", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "atadenoise",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("atadenoise graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed atadenoise source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive atadenoise frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `atadenoise=` after the last input frame and emit remaining output.
pub(crate) unsafe fn atadenoise_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush atadenoise source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed atadenoise frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_atadenoise_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("atadenoise args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "atadenoise args may only contain [A-Za-z0-9=.:_-] (FFmpeg atadenoise= options)".into(),
        );
    }
    Ok(())
}

/// Apply per-frame `owdenoise=` (fair-pairs FFmpeg `-vf owdenoise=ARGS`).
pub(crate) unsafe fn owdenoise_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_owdenoise_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "owdenoise", args) }
}

pub(crate) fn validate_owdenoise_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("owdenoise args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "owdenoise args may only contain [A-Za-z0-9=.:_-] (FFmpeg owdenoise= options)".into(),
        );
    }
    Ok(())
}

/// Apply per-frame `vaguedenoiser=` (fair-pairs FFmpeg `-vf vaguedenoiser=ARGS`).
pub(crate) unsafe fn vaguedenoiser_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_vaguedenoiser_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "vaguedenoiser", args) }
}

pub(crate) fn validate_vaguedenoiser_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("vaguedenoiser args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "vaguedenoiser args may only contain [A-Za-z0-9=.:_-] (FFmpeg vaguedenoiser= options)"
                .into(),
        );
    }
    Ok(())
}

/// Feed one frame into `deflicker=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn deflicker_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_deflicker_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("deflicker requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("deflicker", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "deflicker",
                args,
                s,
                time_base,
                sar,
            )?);
        }
        let active = graph.as_mut().ok_or("deflicker graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed deflicker source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive deflicker frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `deflicker=` after the last input frame and emit remaining output.
pub(crate) unsafe fn deflicker_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush deflicker source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed deflicker frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_deflicker_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("deflicker args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "deflicker args may only contain [A-Za-z0-9=.:_-] (FFmpeg deflicker= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `photosensitivity=` into a reusable destination frame.
/// Parsed chain lets libavfilter auto-insert RGB conversion (filter accepts RGB24/BGR24 only).
pub(crate) unsafe fn photosensitivity_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_photosensitivity_args(args)?;
    let chain = if args.is_empty() {
        "photosensitivity".to_string()
    } else {
        format!("photosensitivity={args}")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "photosensitivity", &chain) }
}

pub(crate) fn validate_photosensitivity_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("photosensitivity args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "photosensitivity args may only contain [A-Za-z0-9=.:_-] (FFmpeg photosensitivity= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `monochrome=` into a reusable destination frame.
/// `args` is the option string after `monochrome=` (e.g. `cb=0.2:cr=-0.1`); empty uses defaults.
pub(crate) unsafe fn monochrome_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_monochrome_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "monochrome", args) }
}

pub(crate) fn validate_monochrome_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("monochrome args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "monochrome args may only contain [A-Za-z0-9=.:_-] (FFmpeg monochrome= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `grayworld` into a reusable destination frame.
/// `args` is empty/`0` (defaults) or the option string after `grayworld=` (e.g. timeline `enable=`).
pub(crate) unsafe fn grayworld_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_grayworld_args(args)?;
    let filter_args = if args.is_empty() || args == "0" {
        ""
    } else {
        args
    };
    unsafe { apply_video_filter(graph, dst, src, "grayworld", filter_args) }
}

pub(crate) fn validate_grayworld_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("grayworld args must be 0..=128 bytes without NUL".into());
    }
    if !(args.is_empty()
        || args == "0"
        || args.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'=' | b':'
                        | b'-'
                        | b'_'
                        | b'.'
                        | b'\''
                        | b'('
                        | b')'
                        | b'+'
                        | b','
                        | b'/'
                        | b' '
                )
        }))
    {
        return Err(
            "grayworld args may only contain [A-Za-z0-9=.:_'()+-,/ ] (FFmpeg grayworld= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `drawbox=` into a reusable destination frame.
/// `args` is the option string after `drawbox=` (e.g. `x=10:y=10:w=40:h=20:color=red`).
pub(crate) unsafe fn drawbox_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_drawbox_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "drawbox", args) }
}

pub(crate) fn validate_drawbox_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("drawbox args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "drawbox args may only contain [A-Za-z0-9=.:_-] (FFmpeg drawbox= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `drawgrid=` into a reusable destination frame.
/// `args` is the option string after `drawgrid=` (e.g. `w=16:h=16:color=white`).
pub(crate) unsafe fn drawgrid_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_drawgrid_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "drawgrid", args) }
}

pub(crate) fn validate_drawgrid_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("drawgrid args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "drawgrid args may only contain [A-Za-z0-9=.:_-] (FFmpeg drawgrid= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `lagfun=` into a reusable destination frame.
/// `args` is the option string after `lagfun=` (e.g. `decay=0.95`); empty uses defaults.
pub(crate) unsafe fn lagfun_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_lagfun_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "lagfun", args) }
}

/// Feed one frame into `lagfun=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn lagfun_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_lagfun_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("lagfun requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("lagfun", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "lagfun", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("lagfun graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed lagfun source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive lagfun frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `lagfun=` after the last input frame and emit remaining output.
pub(crate) unsafe fn lagfun_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush lagfun source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed lagfun frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_lagfun_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("lagfun args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "lagfun args may only contain [A-Za-z0-9=.:_-] (FFmpeg lagfun= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `amplify=` into a reusable destination frame.
/// `args` is the option string after `amplify=` (e.g. `radius=2:factor=2`); empty uses defaults.
pub(crate) unsafe fn amplify_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_amplify_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "amplify", args) }
}

/// Feed one frame into `amplify=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn amplify_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_amplify_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("amplify requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("amplify", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "amplify", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("amplify graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed amplify source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive amplify frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `amplify=` after the last input frame and emit remaining output.
pub(crate) unsafe fn amplify_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush amplify source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed amplify frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

pub(crate) fn validate_amplify_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("amplify args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "amplify args may only contain [A-Za-z0-9=.:_-] (FFmpeg amplify= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `bitplanenoise=` into a reusable destination frame.
/// `args` is the option string after `bitplanenoise=` (e.g. `bitplane=1:filter=1`); empty uses defaults.
pub(crate) unsafe fn bitplanenoise_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_bitplanenoise_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "bitplanenoise", args) }
}

pub(crate) fn validate_bitplanenoise_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("bitplanenoise args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "bitplanenoise args may only contain [A-Za-z0-9=.:_-] (FFmpeg bitplanenoise= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `deband=` into a reusable destination frame.
/// `args` is the option string after `deband=` (e.g. `1thr=0.02`); empty uses defaults.
pub(crate) unsafe fn deband_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_deband_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "deband", args) }
}

pub(crate) fn validate_deband_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("deband args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "deband args may only contain [A-Za-z0-9=.:_-] (FFmpeg deband= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `gradfun=` into a reusable destination frame.
/// `args` is the option string after `gradfun=` (e.g. `strength=1.2`); empty uses defaults.
pub(crate) unsafe fn gradfun_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_gradfun_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "gradfun", args) }
}

pub(crate) fn validate_gradfun_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("gradfun args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "gradfun args may only contain [A-Za-z0-9=.:_-] (FFmpeg gradfun= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `lenscorrection=` into a reusable destination frame.
/// `args` is the option string after `lenscorrection=` (e.g. `k1=-0.1`); empty uses defaults.
pub(crate) unsafe fn lenscorrection_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_lenscorrection_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "lenscorrection", args) }
}

pub(crate) fn validate_lenscorrection_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("lenscorrection args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "lenscorrection args may only contain [A-Za-z0-9=.:_-] (FFmpeg lenscorrection= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `pixelize=` into a reusable destination frame.
/// `args` is the option string after `pixelize=` (e.g. `width=8:height=8`); empty uses defaults.
pub(crate) unsafe fn pixelize_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_pixelize_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "pixelize", args) }
}

pub(crate) fn validate_pixelize_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("pixelize args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "pixelize args may only contain [A-Za-z0-9=.:_-] (FFmpeg pixelize= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `removegrain=` into a reusable destination frame.
/// `args` is the option string after `removegrain=` (e.g. `m0=1`); empty uses defaults.
pub(crate) unsafe fn removegrain_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_removegrain_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "removegrain", args) }
}

pub(crate) fn validate_removegrain_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("removegrain args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "removegrain args may only contain [A-Za-z0-9=.:_-] (FFmpeg removegrain= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `yaepblur=` into a reusable destination frame.
/// `args` is the option string after `yaepblur=` (e.g. `r=3`); empty uses defaults.
pub(crate) unsafe fn yaepblur_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_yaepblur_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "yaepblur", args) }
}

pub(crate) fn validate_yaepblur_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("yaepblur args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "yaepblur args may only contain [A-Za-z0-9=.:_-] (FFmpeg yaepblur= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `vibrance=` into a reusable destination frame.
/// `args` is the option string after `vibrance=` (e.g. `intensity=0.3:rbal=1`); empty uses defaults.
pub(crate) unsafe fn vibrance_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_vibrance_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "vibrance", args) }
}

pub(crate) fn validate_vibrance_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("vibrance args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "vibrance args may only contain [A-Za-z0-9=.:_-] (FFmpeg vibrance= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `dilation=` into a reusable destination frame.
/// `args` is the option string after `dilation=` (e.g. `threshold0=10`); empty uses defaults.
pub(crate) unsafe fn dilation_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_dilation_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "dilation", args) }
}

pub(crate) fn validate_dilation_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("dilation args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "dilation args may only contain [A-Za-z0-9=.:_-] (FFmpeg dilation= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `erosion=` into a reusable destination frame.
/// `args` is the option string after `erosion=` (e.g. `threshold0=10`); empty uses defaults.
pub(crate) unsafe fn erosion_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_erosion_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "erosion", args) }
}

pub(crate) fn validate_erosion_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("erosion args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "erosion args may only contain [A-Za-z0-9=.:_-] (FFmpeg erosion= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorize=` into a reusable destination frame.
/// `args` is the option string after `colorize=` (e.g. `hue=120:saturation=0.5`); empty uses defaults.
pub(crate) unsafe fn colorize_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorize_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorize", args) }
}

pub(crate) fn validate_colorize_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorize args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorize args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorize= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `exposure=` into a reusable destination frame.
/// `args` is the option string after `exposure=` (e.g. `exposure=0.5`); empty uses defaults.
pub(crate) unsafe fn exposure_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_exposure_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "exposure", args) }
}

pub(crate) fn validate_exposure_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("exposure args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "exposure args may only contain [A-Za-z0-9=.:_-] (FFmpeg exposure= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `chromashift=` into a reusable destination frame.
/// `args` is the option string after `chromashift=` (e.g. `cbh=4`); empty uses defaults.
pub(crate) unsafe fn chromashift_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_chromashift_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "chromashift", args) }
}

pub(crate) fn validate_chromashift_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("chromashift args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "chromashift args may only contain [A-Za-z0-9=.:_-] (FFmpeg chromashift= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorcontrast=` into a reusable destination frame.
/// `args` is the option string after `colorcontrast=` (e.g. `rc=0.1:gm=0.1:by=0.1`); empty uses defaults.
pub(crate) unsafe fn colorcontrast_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorcontrast_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorcontrast", args) }
}

pub(crate) fn validate_colorcontrast_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorcontrast args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorcontrast args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorcontrast= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorcorrect=` into a reusable destination frame.
/// `args` is the option string after `colorcorrect=` (e.g. `rl=0.1:bl=-0.1`); empty uses defaults.
pub(crate) unsafe fn colorcorrect_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorcorrect_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorcorrect", args) }
}

pub(crate) fn validate_colorcorrect_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorcorrect args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorcorrect args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorcorrect= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `histeq=` into a reusable destination frame.
/// `args` is the option string after `histeq=` (e.g. `strength=0.2`); empty uses defaults.
pub(crate) unsafe fn histeq_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_histeq_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "histeq", args) }
}

pub(crate) fn validate_histeq_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("histeq args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "histeq args may only contain [A-Za-z0-9=.:_-] (FFmpeg histeq= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `shuffleplanes=` into a reusable destination frame.
/// `args` is the option string after `shuffleplanes=` (e.g. `map0=0:map1=2:map2=1`); empty uses defaults.
pub(crate) unsafe fn shuffleplanes_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_shuffleplanes_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "shuffleplanes", args) }
}

pub(crate) fn validate_shuffleplanes_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("shuffleplanes args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "shuffleplanes args may only contain [A-Za-z0-9=.:_-] (FFmpeg shuffleplanes= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `lutyuv=` into a reusable destination frame.
/// `args` is the option string after `lutyuv=` (e.g. `y=val*0.8`); empty uses defaults.
pub(crate) unsafe fn lutyuv_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_lutyuv_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "lutyuv", args) }
}

pub(crate) fn validate_lutyuv_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("lutyuv args must be 0..=128 bytes without NUL".into());
    }
    if !args.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'=' | b':' | b'-' | b'_' | b'.' | b'*' | b'+' | b'(' | b')' | b'/'
            )
    }) {
        return Err(
            "lutyuv args may only contain [A-Za-z0-9=.:_*+()/-] (FFmpeg lutyuv= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorhold=` into a reusable destination frame.
/// `args` is the option string after `colorhold=` (e.g. `similarity=0.2:blend=0.1`); empty uses defaults.
pub(crate) unsafe fn colorhold_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorhold_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "colorhold", args) }
}

pub(crate) fn validate_colorhold_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorhold args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorhold args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorhold= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `fade=` into a reusable destination frame.
/// `args` is the option string after `fade=` (e.g. `t=in:s=0:n=4`); empty uses defaults.
pub(crate) unsafe fn fade_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_fade_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "fade", args) }
}

pub(crate) fn validate_fade_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("fade args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("fade args may only contain [A-Za-z0-9=.:_-] (FFmpeg fade= options)".into());
    }
    Ok(())
}

/// Apply FFmpeg-compatible `perspective=` into a reusable destination frame.
/// `args` is the option string after `perspective=` (e.g. `sense=destination:x0=0:y0=10:x1=W:y1=0:x2=0:y2=H:x3=W:y3=H-10`); empty uses defaults.
pub(crate) unsafe fn perspective_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_perspective_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "perspective", args) }
}

pub(crate) fn validate_perspective_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("perspective args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "perspective args may only contain [A-Za-z0-9=.:_-] (FFmpeg perspective= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `lumakey=` into a reusable destination frame.
/// `args` is the option string after `lumakey=` (e.g. `threshold=0.1:tolerance=0.1:softness=0.1`); empty uses defaults.
pub(crate) unsafe fn lumakey_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_lumakey_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "lumakey", args) }
}

pub(crate) fn validate_lumakey_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("lumakey args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "lumakey args may only contain [A-Za-z0-9=.:_-] (FFmpeg lumakey= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `chromakey=` into a reusable destination frame.
/// `args` is the option string after `chromakey=` (e.g. `similarity=0.3:blend=0.1`); empty uses defaults.
pub(crate) unsafe fn chromakey_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_chromakey_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "chromakey", args) }
}

pub(crate) fn validate_chromakey_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("chromakey args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "chromakey args may only contain [A-Za-z0-9=.:_-] (FFmpeg chromakey= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `colorkey=` into a reusable destination frame.
/// `args` is the option string after `colorkey=` (e.g. `color=black:similarity=0.1:blend=0.1`); empty uses defaults.
/// Output is converted to `yuva420p` so FFV1 encode keeps alpha (matches FFmpeg `colorkey=,format=yuva420p`).
pub(crate) unsafe fn colorkey_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_colorkey_args(args)?;
    let chain = if args.is_empty() {
        "colorkey,format=yuva420p".into()
    } else {
        format!("colorkey={args},format=yuva420p")
    };
    unsafe { apply_parsed_video_filter(graph, dst, src, "colorkey", &chain) }
}

pub(crate) fn validate_colorkey_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("colorkey args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "colorkey args may only contain [A-Za-z0-9=.:_-] (FFmpeg colorkey= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `despill=` into a reusable destination frame.
/// `args` is the option string after `despill=` (e.g. `type=green:mix=0.5`); empty uses defaults.
pub(crate) unsafe fn despill_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_despill_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "despill", args) }
}

pub(crate) fn validate_despill_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("despill args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "despill args may only contain [A-Za-z0-9=.:_-] (FFmpeg despill= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `selectivecolor=` into a reusable destination frame.
/// `args` is the option string after `selectivecolor=` (e.g. `reds=0.2 0 0 0`); empty uses defaults.
pub(crate) unsafe fn selectivecolor_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_selectivecolor_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "selectivecolor", args) }
}

pub(crate) fn validate_selectivecolor_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("selectivecolor args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.' | b' '))
        || args
            .bytes()
            .any(|b| matches!(b, b';' | b'\'' | b'"' | b'`' | b'|' | b'&' | b'$'))
    {
        return Err(
            "selectivecolor args may only contain [A-Za-z0-9=.:_ -] (FFmpeg selectivecolor= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `stereo3d=` into a reusable destination frame.
/// `args` is the option string after `stereo3d=` (e.g. `sbsl:abl`); empty uses defaults.
pub(crate) unsafe fn stereo3d_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_stereo3d_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "stereo3d", args) }
}

pub(crate) fn validate_stereo3d_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("stereo3d args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "stereo3d args may only contain [A-Za-z0-9=.:_-] (FFmpeg stereo3d= options)".into(),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stereo3DFormat {
    SideBySideLr,
    SideBySideRl,
    SideBySide2Lr,
    SideBySide2Rl,
    AboveBelowLr,
    AboveBelowRl,
    AboveBelow2Lr,
    AboveBelow2Rl,
    InterleaveRowsLr,
    InterleaveRowsRl,
    InterleaveColsLr,
    InterleaveColsRl,
    AlternatingLr,
    AlternatingRl,
    MonoLeft,
    MonoRight,
    Anaglyph,
    CheckerboardLr,
    CheckerboardRl,
    Hdmi,
}

fn parse_stereo3d_format(name: &str) -> Result<Stereo3DFormat> {
    match name {
        "sbsl" => Ok(Stereo3DFormat::SideBySideLr),
        "sbsr" => Ok(Stereo3DFormat::SideBySideRl),
        "sbs2l" => Ok(Stereo3DFormat::SideBySide2Lr),
        "sbs2r" => Ok(Stereo3DFormat::SideBySide2Rl),
        "abl" | "tbl" => Ok(Stereo3DFormat::AboveBelowLr),
        "abr" | "tbr" => Ok(Stereo3DFormat::AboveBelowRl),
        "ab2l" | "tb2l" => Ok(Stereo3DFormat::AboveBelow2Lr),
        "ab2r" | "tb2r" => Ok(Stereo3DFormat::AboveBelow2Rl),
        "irl" => Ok(Stereo3DFormat::InterleaveRowsLr),
        "irr" => Ok(Stereo3DFormat::InterleaveRowsRl),
        "icl" => Ok(Stereo3DFormat::InterleaveColsLr),
        "icr" => Ok(Stereo3DFormat::InterleaveColsRl),
        "al" => Ok(Stereo3DFormat::AlternatingLr),
        "ar" => Ok(Stereo3DFormat::AlternatingRl),
        "ml" => Ok(Stereo3DFormat::MonoLeft),
        "mr" => Ok(Stereo3DFormat::MonoRight),
        "arcg" | "arch" | "arcc" | "arcd" | "arbg" | "argg" | "agmg" | "agmh" | "agmc" | "agmd"
        | "aybg" | "aybh" | "aybc" | "aybd" => Ok(Stereo3DFormat::Anaglyph),
        "chl" => Ok(Stereo3DFormat::CheckerboardLr),
        "chr" => Ok(Stereo3DFormat::CheckerboardRl),
        "hdmi" => Ok(Stereo3DFormat::Hdmi),
        _ => Err(format!("unsupported stereo3d format: {name}").into()),
    }
}

fn parse_stereo3d_formats(args: &str) -> Result<(Stereo3DFormat, Stereo3DFormat)> {
    if args.is_empty() {
        return Ok((Stereo3DFormat::SideBySideLr, Stereo3DFormat::Anaglyph));
    }
    if args.contains('=') {
        let mut in_fmt = None;
        let mut out_fmt = None;
        for part in args.split(':') {
            let Some((key, value)) = part.split_once('=') else {
                return Err("stereo3d args must use in= and out= pairs".into());
            };
            match key {
                "in" => in_fmt = Some(parse_stereo3d_format(value)?),
                "out" => out_fmt = Some(parse_stereo3d_format(value)?),
                _ => return Err(format!("unsupported stereo3d option: {key}").into()),
            }
        }
        return Ok((
            in_fmt.ok_or("stereo3d args missing in=")?,
            out_fmt.ok_or("stereo3d args missing out=")?,
        ));
    }
    let Some((in_name, out_name)) = args.split_once(':') else {
        return Err("stereo3d args must be in:out or in=...:out=...".into());
    };
    Ok((
        parse_stereo3d_format(in_name)?,
        parse_stereo3d_format(out_name)?,
    ))
}

/// Output size after `stereo3d=` (matches FFmpeg vf_stereo3d config_output geometry).
pub(crate) fn stereo3d_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_stereo3d_args(args)?;
    let (in_fmt, out_fmt) = parse_stereo3d_formats(args)?;
    if matches!(
        in_fmt,
        Stereo3DFormat::AlternatingLr | Stereo3DFormat::AlternatingRl
    ) || matches!(
        out_fmt,
        Stereo3DFormat::AlternatingLr | Stereo3DFormat::AlternatingRl
    ) {
        return Err("stereo3d alternating-frame modes change frame count".into());
    }
    let (mut eye_w, mut eye_h) = (width, height);
    match in_fmt {
        Stereo3DFormat::SideBySideLr | Stereo3DFormat::SideBySideRl => {
            eye_w = width / 2;
        }
        Stereo3DFormat::SideBySide2Lr | Stereo3DFormat::SideBySide2Rl => {}
        Stereo3DFormat::AboveBelowLr | Stereo3DFormat::AboveBelowRl => {
            eye_h = height / 2;
        }
        Stereo3DFormat::AboveBelow2Lr | Stereo3DFormat::AboveBelow2Rl => {}
        Stereo3DFormat::InterleaveColsLr | Stereo3DFormat::InterleaveColsRl => {
            eye_w = width / 2;
        }
        Stereo3DFormat::InterleaveRowsLr | Stereo3DFormat::InterleaveRowsRl => {
            if !matches!(
                out_fmt,
                Stereo3DFormat::CheckerboardLr | Stereo3DFormat::CheckerboardRl
            ) {
                eye_h = height / 2;
            }
        }
        Stereo3DFormat::MonoLeft
        | Stereo3DFormat::MonoRight
        | Stereo3DFormat::Anaglyph
        | Stereo3DFormat::CheckerboardLr
        | Stereo3DFormat::CheckerboardRl
        | Stereo3DFormat::Hdmi => {}
        Stereo3DFormat::AlternatingLr | Stereo3DFormat::AlternatingRl => unreachable!(),
    }
    let (mut out_w, mut out_h) = (eye_w, eye_h);
    match out_fmt {
        Stereo3DFormat::SideBySideLr | Stereo3DFormat::SideBySideRl => {
            out_w = eye_w * 2;
        }
        Stereo3DFormat::SideBySide2Lr | Stereo3DFormat::SideBySide2Rl => {}
        Stereo3DFormat::AboveBelowLr | Stereo3DFormat::AboveBelowRl => {
            out_h = eye_h * 2;
        }
        Stereo3DFormat::AboveBelow2Lr | Stereo3DFormat::AboveBelow2Rl => {}
        Stereo3DFormat::InterleaveRowsLr | Stereo3DFormat::InterleaveRowsRl => {
            out_h = eye_h * 2;
        }
        Stereo3DFormat::CheckerboardLr
        | Stereo3DFormat::CheckerboardRl
        | Stereo3DFormat::InterleaveColsLr
        | Stereo3DFormat::InterleaveColsRl => {
            out_w = eye_w * 2;
        }
        Stereo3DFormat::Hdmi => {
            if eye_h != 720 && eye_h != 1080 {
                return Err("stereo3d hdmi output requires 720 or 1080 eye height".into());
            }
            let blanks = eye_h / 24;
            out_h = eye_h * 2 + blanks;
        }
        Stereo3DFormat::Anaglyph | Stereo3DFormat::MonoLeft | Stereo3DFormat::MonoRight => {}
        Stereo3DFormat::AlternatingLr | Stereo3DFormat::AlternatingRl => unreachable!(),
    }
    Ok((out_w, out_h))
}

/// Apply FFmpeg-compatible `field=` into a reusable destination frame.
/// `args` is the option string after `field=` (e.g. `bottom`); empty uses defaults.
pub(crate) unsafe fn field_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_field_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "field", args) }
}

pub(crate) fn validate_field_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("field args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("field args may only contain [A-Za-z0-9=.:_-] (FFmpeg field= options)".into());
    }
    if !args.is_empty()
        && args != "top"
        && args != "bottom"
        && args != "0"
        && args != "1"
        && args != "type=top"
        && args != "type=bottom"
    {
        return Err(format!("unsupported field option: {args}").into());
    }
    Ok(())
}

/// Output size after `field=` (height halved; width unchanged).
pub(crate) fn field_output_size(width: u32, height: u32, args: &str) -> Result<(u32, u32)> {
    validate_field_args(args)?;
    let out_h = height / 2;
    if out_h == 0 {
        return Err("field requires height >= 2".into());
    }
    Ok((width, out_h))
}

/// Feed one frame into `fade=`; returns `false` while the filter buffers (EAGAIN).
pub(crate) unsafe fn fade_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<bool> {
    validate_fade_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("fade requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("fade", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "fade", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("fade graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed fade source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive fade frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Flush `fade=` after the last input frame and emit remaining output.
pub(crate) unsafe fn fade_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush fade source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed fade frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Apply `fade=` one-in/one-out; switches to push mode when the first frame buffers.
pub(crate) unsafe fn fade_apply_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    push_mode: &mut bool,
) -> Result<bool> {
    if *push_mode {
        return unsafe { fade_push_frame(graph, dst, src, args) };
    }
    validate_fade_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("fade requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("fade", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "fade", args, s, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("fade graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed fade source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            *push_mode = true;
            return Ok(false);
        }
        check(code, "receive fade frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Apply FFmpeg-compatible `pseudocolor=` into a reusable destination frame.
/// `args` is the option string after `pseudocolor=` (e.g. `preset=magma`); empty uses defaults.
pub(crate) unsafe fn pseudocolor_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_pseudocolor_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "pseudocolor", args) }
}

pub(crate) fn validate_pseudocolor_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("pseudocolor args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "pseudocolor args may only contain [A-Za-z0-9=.:_-] (FFmpeg pseudocolor= options)"
                .into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `nlmeans=` into a reusable destination frame.
/// `args` is the option string after `nlmeans=` (e.g. `s=1.0`); empty uses defaults.
pub(crate) unsafe fn nlmeans_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_nlmeans_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "nlmeans", args) }
}

pub(crate) fn validate_nlmeans_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("nlmeans args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "nlmeans args may only contain [A-Za-z0-9=.:_-] (FFmpeg nlmeans= options)".into(),
        );
    }
    Ok(())
}

/// Apply FFmpeg-compatible `bm3d=` into a reusable destination frame.
/// `args` is the option string after `bm3d=` (e.g. `sigma=3`); empty uses defaults.
pub(crate) unsafe fn bm3d_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_bm3d_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "bm3d", args) }
}

pub(crate) fn validate_bm3d_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("bm3d args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err("bm3d args may only contain [A-Za-z0-9=.:_-] (FFmpeg bm3d= options)".into());
    }
    Ok(())
}

/// Apply per-frame `dctdnoiz=` (fair-pairs FFmpeg `-vf dctdnoiz=ARGS`).
pub(crate) unsafe fn dctdnoiz_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_dctdnoiz_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "dctdnoiz", args) }
}

pub(crate) fn validate_dctdnoiz_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("dctdnoiz args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "dctdnoiz args may only contain [A-Za-z0-9=.:_-] (FFmpeg dctdnoiz= options)".into(),
        );
    }
    Ok(())
}

/// Apply per-frame `fftdnoiz=` (fair-pairs FFmpeg `-vf fftdnoiz=ARGS`).
pub(crate) unsafe fn fftdnoiz_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_fftdnoiz_args(args)?;
    unsafe { apply_video_filter(graph, dst, src, "fftdnoiz", args) }
}

pub(crate) fn validate_fftdnoiz_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("fftdnoiz args must be 0..=128 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'-' | b'_' | b'.'))
    {
        return Err(
            "fftdnoiz args may only contain [A-Za-z0-9=.:_-] (FFmpeg fftdnoiz= options)".into(),
        );
    }
    Ok(())
}

/// Validate FFmpeg `fps=` rate string (`25`, `12`, `30000/1001`).
pub(crate) fn validate_fps_args(args: &str) -> Result<()> {
    if args.is_empty() || args.len() > 32 || args.contains('\0') {
        return Err("fps args must be 1..=32 bytes without NUL".into());
    }
    if !args
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'/' | b'.'))
    {
        return Err("fps args may only contain [0-9/.] (FFmpeg fps= rate)".into());
    }
    if args.starts_with('/') || args.ends_with('/') || args.contains("//") {
        return Err("fps args must be a rate like 25 or 30000/1001".into());
    }
    Ok(())
}

fn open_fps_graph(graph: &mut Option<FilterGraph>, src: *mut AVFrame, args: &str) -> Result<()> {
    validate_fps_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("fps requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("fps", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "fps", args, s, time_base, sar,
            )?);
        }
    }
    Ok(())
}

/// Feed one frame into `fps=` and emit each produced frame (0..N).
pub(crate) unsafe fn fps_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    open_fps_graph(graph, src, args)?;
    unsafe {
        let active = graph.as_mut().ok_or("fps graph missing")?;
        check(av_buffersrc_write_frame(active.src, src), "feed fps source")?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive fps frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `fps=` after the last input frame and emit remaining output.
pub(crate) unsafe fn fps_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush fps source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed fps frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Validate FFmpeg `minterpolate=` option string (e.g. `mi_mode=blend:fps=50`).
pub(crate) fn validate_minterpolate_args(args: &str) -> Result<()> {
    if args.len() > 128 || args.contains('\0') {
        return Err("minterpolate args must be 0..=128 bytes without NUL".into());
    }
    if !args.is_empty()
        && !args
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b':' | b'.' | b'_' | b'-'))
    {
        return Err(
            "minterpolate args may only contain [A-Za-z0-9=.:_-] (FFmpeg minterpolate= options)"
                .into(),
        );
    }
    Ok(())
}

fn open_minterpolate_graph(
    graph: &mut Option<FilterGraph>,
    src: *mut AVFrame,
    args: &str,
) -> Result<()> {
    validate_minterpolate_args(args)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("minterpolate requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches("minterpolate", args, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_from_frame(
                "minterpolate",
                args,
                s,
                time_base,
                sar,
            )?);
        }
    }
    Ok(())
}

/// Feed one frame into `minterpolate=` and emit each produced frame (0..N).
pub(crate) unsafe fn minterpolate_push_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    args: &str,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    open_minterpolate_graph(graph, src, args)?;
    unsafe {
        let active = graph.as_mut().ok_or("minterpolate graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed minterpolate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive minterpolate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Flush `minterpolate=` after the last input frame and emit remaining output.
pub(crate) unsafe fn minterpolate_flush(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    let Some(active) = graph.as_mut() else {
        return Ok(());
    };
    unsafe {
        check(
            av_buffersrc_write_frame(active.src, ptr::null()),
            "flush minterpolate source",
        )?;
        loop {
            av_frame_unref(dst);
            let code = av_buffersink_get_frame(active.sink, dst);
            if code == -libc::EAGAIN || code == EOF {
                return Ok(());
            }
            check(code, "receive flushed minterpolate frame")?;
            let d = &mut *dst;
            d.pict_type = 0;
            d.quality = 0;
            emit(dst)?;
        }
    }
}

/// Feed one frame through optional `minterpolate=` then optional `fps=`.
pub(crate) unsafe fn temporal_push_frame(
    minterpolate_graph: &mut Option<FilterGraph>,
    minterpolate_dst: *mut AVFrame,
    minterpolate: Option<&str>,
    fps_graph: &mut Option<FilterGraph>,
    fps_dst: *mut AVFrame,
    fps: Option<&str>,
    src: *mut AVFrame,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if let Some(args) = minterpolate {
        minterpolate_push_frame(minterpolate_graph, minterpolate_dst, src, args, |frame| {
            if let Some(fps_args) = fps {
                fps_push_frame(fps_graph, fps_dst, frame, fps_args, |out| emit(out))?;
            } else {
                emit(frame)?;
            }
            Ok(())
        })?;
    } else if let Some(args) = fps {
        fps_push_frame(fps_graph, fps_dst, src, args, |out| emit(out))?;
    } else {
        emit(src)?;
    }
    Ok(())
}

/// Flush optional `minterpolate=` then optional `fps=` after the last input frame.
pub(crate) unsafe fn temporal_flush_frames(
    minterpolate_graph: &mut Option<FilterGraph>,
    minterpolate_dst: *mut AVFrame,
    minterpolate: Option<&str>,
    fps_graph: &mut Option<FilterGraph>,
    fps_dst: *mut AVFrame,
    fps: Option<&str>,
    mut emit: impl FnMut(*mut AVFrame) -> Result<()>,
) -> Result<()> {
    if minterpolate.is_some() {
        minterpolate_flush(minterpolate_graph, minterpolate_dst, |frame| {
            if let Some(fps_args) = fps {
                fps_push_frame(fps_graph, fps_dst, frame, fps_args, |out| emit(out))?;
            } else {
                emit(frame)?;
            }
            Ok(())
        })?;
    }
    if fps.is_some() {
        fps_flush(fps_graph, fps_dst, |out| emit(out))?;
    }
    Ok(())
}

/// Apply FFmpeg-compatible `transpose=` into a reusable destination frame.
pub(crate) unsafe fn transpose_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    mode: TransposeMode,
) -> Result<()> {
    unsafe { apply_video_filter(graph, dst, src, "transpose", mode.as_str()) }
}

/// Apply FFmpeg-compatible `pad=W:H:X:Y:black` into a reusable destination frame.
pub(crate) unsafe fn pad_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    pad: PadRect,
) -> Result<()> {
    let args = format!("{}:{}:{}:{}:black", pad.width, pad.height, pad.x, pad.y);
    unsafe { apply_video_filter(graph, dst, src, "pad", &args) }
}

/// Apply FFmpeg-compatible `rotate=` into a reusable destination frame.
pub(crate) unsafe fn rotate_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    angle: RotateAngle,
) -> Result<()> {
    if angle.degrees.abs() < 1e-12 {
        unsafe {
            av_frame_unref(dst);
            check(av_frame_ref(dst, src), "reference identity rotate frame")?;
        }
        return Ok(());
    }
    unsafe {
        let s = &*src;
        let (ow, oh) = angle.size(s.width as u32, s.height as u32);
        let args = format!("a={}*PI/180:ow={ow}:oh={oh}:c=black", angle.degrees);
        apply_video_filter(graph, dst, src, "rotate", &args)
    }
}

/// Normalized absolute subtitle path for libass (`av_opt_set` / FFmpeg CLI escape).
pub(crate) fn subtitles_filename(path: &Path) -> Result<String> {
    if !path.is_file() {
        return Err("subtitle file not found".into());
    }
    let abs = path
        .canonicalize()
        .map_err(|err| format!("subtitle path: {err}"))?;
    let utf = abs.to_str().ok_or("subtitle path must be UTF-8")?;
    let utf = utf
        .strip_prefix(r"\\?\")
        .or_else(|| utf.strip_prefix("//?/"))
        .unwrap_or(utf);
    Ok(utf.replace('\\', "/"))
}

/// Escape absolute path for FFmpeg CLI `-vf subtitles=...` fair-pair oracles.
pub fn subtitles_cli_vf(path: &Path) -> Result<String> {
    let normalized = subtitles_filename(path)?;
    let mut escaped = String::from("'");
    for ch in normalized.chars() {
        if matches!(ch, '\\' | ':' | '\'') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('\'');
    Ok(format!("subtitles={escaped}"))
}

/// Escape absolute path for FFmpeg CLI `movie=...` fair-pair oracles.
pub fn movie_cli_path(path: &Path) -> Result<String> {
    let normalized = subtitles_filename(path)?;
    let mut escaped = String::from("'");
    for ch in normalized.chars() {
        if matches!(ch, '\\' | ':' | '\'') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('\'');
    Ok(escaped)
}

/// FFmpeg-equivalent `-vf` for overlay via `movie=` (fair-pairs dedicated overlay command).
pub fn overlay_cli_vf(path: &Path, x: i32, y: i32) -> Result<String> {
    let movie = movie_cli_path(path)?;
    Ok(format!("movie={movie}[ov];[in][ov]overlay={x}:{y}"))
}

/// Composite an external video via libavfilter `movie=` + `overlay=x:y`.
pub(crate) unsafe fn overlay_frame(
    graph: &mut Option<OverlayGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    path: &Path,
    x: i32,
    y: i32,
) -> Result<()> {
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("overlay requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let filename = subtitles_filename(path)?;
        let needs_new = match graph.as_ref() {
            Some(existing) => {
                !existing.matches(&filename, x, y, s.width, s.height, s.format, time_base)
            }
            None => true,
        };
        if needs_new {
            *graph = Some(OverlayGraph::open(
                &filename, x, y, s.width, s.height, s.format, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("overlay graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed overlay main source",
        )?;
        av_frame_unref(dst);
        check(
            av_buffersink_get_frame(active.sink, dst),
            "receive overlay frame",
        )?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(())
    }
}

pub fn validate_xfade_transition(name: &str) -> Result<()> {
    const ALLOWED: &[&str] = &[
        "fade",
        "fadeblack",
        "fadewhite",
        "wipeleft",
        "wiperight",
        "wipeup",
        "wipedown",
        "slideleft",
        "slideright",
        "slideup",
        "slidedown",
    ];
    if ALLOWED.contains(&name) {
        Ok(())
    } else {
        Err(format!(
            "xfade transition must be one of: {}",
            ALLOWED.join(", ")
        ))
    }
}

pub(crate) fn us_to_filter_secs(us: i64) -> Result<String> {
    if us < 0 {
        return Err("xfade duration/offset must be >= 0".into());
    }
    let whole = us / 1_000_000;
    let frac = us % 1_000_000;
    if frac == 0 {
        Ok(whole.to_string())
    } else {
        Ok(format!("{whole}.{frac:06}")
            .trim_end_matches('0')
            .to_string())
    }
}

/// FFmpeg `-filter_complex` fair-pair string for dual-input xfade + format lock.
pub fn xfade_cli_vf(
    _path: &Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    _fps_num: i32,
    _fps_den: i32,
) -> Result<String> {
    validate_xfade_transition(transition)?;
    let duration = us_to_filter_secs(duration_us)?;
    let offset = us_to_filter_secs(offset_us)?;
    Ok(format!(
        "[0:v][1:v]xfade=transition={transition}:duration={duration}:offset={offset},format=yuv420p"
    ))
}

/// Composite a second video via libavfilter `movie=` + `fps=` + `xfade=`.
pub(crate) unsafe fn xfade_frame(
    graph: &mut Option<XfadeGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    path: &Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    fps_num: i32,
    fps_den: i32,
) -> Result<bool> {
    validate_xfade_transition(transition)?;
    let duration = us_to_filter_secs(duration_us)?;
    let offset = us_to_filter_secs(offset_us)?;
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("xfade requires a valid frame geometry".into());
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let filename = subtitles_filename(path)?;
        let fps_num = if fps_num > 0 { fps_num } else { 25 };
        let fps_den = if fps_den > 0 { fps_den } else { 1 };
        let filter_tb = AVRational {
            num: fps_den,
            den: fps_num,
        };
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(
                &filename, transition, &duration, &offset, s.width, s.height, s.format, filter_tb,
            ),
            None => true,
        };
        if needs_new {
            *graph = Some(XfadeGraph::open(
                &filename, transition, &duration, &offset, s.width, s.height, s.format, sar,
                fps_num, fps_den,
            )?);
        }
        let active = graph.as_mut().ok_or("xfade graph missing")?;
        // Rescale decoder PTS into the CFR filter time base.
        let src_tb = if s.time_base.num > 0 && s.time_base.den > 0 {
            s.time_base
        } else {
            filter_tb
        };
        if s.pts != NOPTS && (src_tb.num != filter_tb.num || src_tb.den != filter_tb.den) {
            (*src).pts = super::rescale_owned(s.pts, src_tb, filter_tb)?;
        }
        (*src).time_base = filter_tb;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed xfade main source",
        )?;
        av_frame_unref(dst);
        let code = av_buffersink_get_frame(active.sink, dst);
        if code == -libc::EAGAIN || code == EOF {
            return Ok(false);
        }
        check(code, "receive xfade frame")?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = (*src).pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(true)
    }
}

/// Burn external text subtitles via libavfilter `subtitles=` (libass).

pub(crate) unsafe fn subtitles_frame(
    graph: &mut Option<FilterGraph>,
    dst: *mut AVFrame,
    src: *mut AVFrame,
    path: &Path,
) -> Result<()> {
    unsafe {
        let s = &*src;
        if s.width <= 0 || s.height <= 0 {
            return Err("subtitles requires a valid frame geometry".into());
        }
        let mut time_base = s.time_base;
        if time_base.num <= 0 || time_base.den <= 0 {
            time_base = AVRational { num: 1, den: 25 };
        }
        let sar = if s.sample_aspect_ratio.num > 0 && s.sample_aspect_ratio.den > 0 {
            s.sample_aspect_ratio
        } else {
            AVRational { num: 1, den: 1 }
        };
        let filename = subtitles_filename(path)?;
        let needs_new = match graph.as_ref() {
            Some(existing) => !existing.matches(
                "subtitles",
                &filename,
                s.width,
                s.height,
                s.format,
                time_base,
            ),
            None => true,
        };
        if needs_new {
            *graph = Some(FilterGraph::open_subtitles(
                &filename, s.width, s.height, s.format, time_base, sar,
            )?);
        }
        let active = graph.as_mut().ok_or("subtitles graph missing")?;
        check(
            av_buffersrc_write_frame(active.src, src),
            "feed subtitles source",
        )?;
        av_frame_unref(dst);
        check(
            av_buffersink_get_frame(active.sink, dst),
            "receive subtitles frame",
        )?;
        let d = &mut *dst;
        d.pict_type = 0;
        d.quality = 0;
        if d.pts == NOPTS {
            d.pts = s.pts;
        }
        if d.duration == 0 {
            d.duration = s.duration;
        }
        Ok(())
    }
}

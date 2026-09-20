#[cfg(feature = "media")]
use std::path::PathBuf;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "media")]
    {
        run_native(args)
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = args;
        Err("media commands require cargo build --release --features media and FFmpeg development libraries".into())
    }
}
#[cfg(feature = "media")]
fn emit_json(quiet: bool, json: String) {
    if !quiet {
        println!("{json}");
    }
}

#[cfg(feature = "media")]
fn play_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut audio = true;
    let mut rate = 1.0f32;
    let mut muted = false;
    let mut fullscreen = false;
    let mut on_top = false;
    let mut audio_track = 0u32;
    let mut subtitle_track = 0i32;
    let mut subtitle_track_set = false;
    let mut subtitles: Option<PathBuf> = None;
    let mut audio_device: Option<String> = None;
    let mut start_us: Option<i64> = None;
    let mut stop_us: Option<i64> = None;
    let mut spherical = false;
    let mut spherical_projection = fvid_media::SphericalProjection::Equirect;
    let mut yaw_deg_milli = 0i32;
    let mut pitch_deg_milli = 0i32;
    let mut roll_deg_milli = 0i32;
    let mut fov_deg_milli = fvid_media::FOV_DEFAULT_MILLI;
    let mut hdr_tonemap = fvid_media::HdrTonemap::Off;
    let mut hdr_nits = fvid_media::HDR_NITS_DEFAULT;
    let mut hdr_maxcll = 0u32;
    let mut hdr_maxfall = 0u32;
    let mut spherical_stereo = fvid_media::SphericalStereoLayout::Mono;
    let mut hdr_mastering_min_milli = 0u32;
    let mut hdr_mastering_max_nits = 0u32;
    let mut stereo3d = fvid_media::PlayStereo3D::Off;
    let mut quit_at_end = false;
    let mut start_paused = false;
    let mut network_cache_ms = fvid_media::NETWORK_CACHE_DEFAULT_MS;
    let mut snapshot_dir: Option<PathBuf> = None;
    let mut inputs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--no-audio" => audio = false,
            "--mute" => muted = true,
            "--fullscreen" => fullscreen = true,
            "--on-top" => on_top = true,
            "--rate" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --rate requires a number")?;
                rate = value
                    .parse()
                    .map_err(|_| format!("invalid playback rate {value}"))?;
            }
            "--start-time" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --start-time requires mm:ss or seconds")?;
                start_us = fvid_media::initial_seek_us(value, -1)
                    .or_else(|| value.parse::<i64>().ok().map(|s| s.saturating_mul(1_000_000)))
                    .ok_or_else(|| format!("invalid start time {value}"))
                    .map(Some)?;
            }
            "--stop-time" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --stop-time requires mm:ss or seconds")?;
                stop_us = fvid_media::initial_stop_us(value, -1)
                    .or_else(|| value.parse::<i64>().ok().map(|s| s.saturating_mul(1_000_000)))
                    .ok_or_else(|| format!("invalid stop time {value}"))
                    .map(Some)?;
            }
            "--audio-track" => {
                index += 1;
                let value = args.get(index).ok_or("play --audio-track requires an index")?;
                audio_track = value
                    .parse()
                    .map_err(|_| format!("invalid audio track {value}"))?;
            }
            "--subtitle-track" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --subtitle-track requires an index")?;
                subtitle_track = value
                    .parse()
                    .map_err(|_| format!("invalid subtitle track {value}"))?;
                subtitle_track_set = true;
            }
            "--no-subtitles" => {
                subtitle_track = -1;
                subtitle_track_set = true;
            }
            "--subtitles" | "--subs" => {
                index += 1;
                let value = args.get(index).ok_or("play --subtitles requires a file")?;
                subtitles = Some(PathBuf::from(value));
                if !subtitle_track_set {
                    subtitle_track = i32::MAX;
                }
            }
            "--audio-device" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --audio-device requires a name")?;
                audio_device = Some(value.clone());
            }
            "--list-audio-devices" => {
                for name in fvid_media::audio_output_devices() {
                    println!("{name}");
                }
                return Ok(());
            }
            "--spherical" | "--360" => spherical = true,
            "--spherical-projection" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --spherical-projection requires equirect|dual-fisheye|cubemap|little-planet")?;
                spherical_projection = fvid_media::parse_spherical_projection(value)?;
                spherical = true;
            }
            "--spherical-stereo" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --spherical-stereo requires mono|tb|sbs")?;
                spherical_stereo = fvid_media::parse_spherical_stereo(value)?;
                spherical = true;
            }
            "--yaw" => {
                index += 1;
                let value = args.get(index).ok_or("play --yaw requires degrees")?;
                yaw_deg_milli = fvid_media::parse_degrees_milli(value)?;
            }
            "--pitch" => {
                index += 1;
                let value = args.get(index).ok_or("play --pitch requires degrees")?;
                pitch_deg_milli = fvid_media::parse_degrees_milli(value)?;
            }
            "--roll" => {
                index += 1;
                let value = args.get(index).ok_or("play --roll requires degrees")?;
                roll_deg_milli = fvid_media::parse_degrees_milli(value)?;
            }
            "--fov" => {
                index += 1;
                let value = args.get(index).ok_or("play --fov requires degrees")?;
                fov_deg_milli = fvid_media::parse_degrees_milli(value)?;
            }
            "--hdr-tonemap" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --hdr-tonemap requires off|clip|reinhard|hable")?;
                hdr_tonemap = fvid_media::parse_hdr_tonemap(value)?;
            }
            "--hdr-nits" => {
                index += 1;
                let value = args.get(index).ok_or("play --hdr-nits requires nits")?;
                hdr_nits = value
                    .parse()
                    .map_err(|_| format!("invalid hdr nits {value}"))?;
            }
            "--hdr-maxcll" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --hdr-maxcll requires MaxCLL,MaxFALL")?;
                let (cll, fall) = fvid_media::parse_hdr_maxcll_maxfall(value)?;
                hdr_maxcll = cll;
                hdr_maxfall = fall;
            }
            "--hdr-mastering" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --hdr-mastering requires min,max nits")?;
                let (min_m, max_n) = fvid_media::parse_hdr_mastering_nits(value)?;
                hdr_mastering_min_milli = min_m;
                hdr_mastering_max_nits = max_n;
            }
            "--play-stereo3d" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --play-stereo3d requires off|sbsl|abl|mono-left|mono-right")?;
                stereo3d = fvid_media::parse_play_stereo3d(value)?;
            }
            "--play-and-exit" | "--quit-at-end" => quit_at_end = true,
            "--start-paused" => start_paused = true,
            "--network-caching" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --network-caching requires milliseconds")?;
                network_cache_ms = value
                    .parse()
                    .map_err(|_| format!("invalid network caching {value}"))?;
            }
            "--snapshot-path" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or("play --snapshot-path requires a directory")?;
                snapshot_dir = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "fvid media play INPUT... [--no-audio] [--mute] [--fullscreen] [--on-top] [--rate N] [--start-time TIME] [--stop-time TIME] [--audio-track N] [--subtitle-track N] [--no-subtitles] [--subtitles FILE] [--audio-device NAME] [--list-audio-devices] [--spherical] [--spherical-projection equirect|dual-fisheye|cubemap|little-planet] [--spherical-stereo mono|tb|sbs] [--yaw DEG] [--pitch DEG] [--roll DEG] [--fov DEG] [--hdr-tonemap off|clip|reinhard|hable] [--hdr-nits N] [--hdr-maxcll MaxCLL,MaxFALL] [--hdr-mastering min,max] [--play-stereo3d off|sbsl|abl|mono-left|mono-right] [--play-and-exit] [--start-paused] [--network-caching MS] [--snapshot-path DIR]\nINPUT is a local file or http/https/rtsp/rtmp/udp URL. --start-time/--stop-time are mm:ss, hh:mm:ss, or seconds. Space pauses. Left/right seek 10s. Up/down volume. M mutes. B cycles audio. V cycles subtitles. L sets A-B loop. T always on top. F fullscreen. [ ] speed. . steps one frame. S saves a bitmap. Esc or Q quits. Drop files, or use Open / Open URL, to replace the playlist. Su"b file loads SRT/ASS. The window stays open after the file ends and continues with the next playlist item. Display is capped at 1920x1080. Rate is clamped to 0.25..4. --spherical enables 360° view; --spherical-projection selects equirect/dual-fisheye/cubemap/little-planet; Ctrl+3 toggles; Ctrl+Shift+3 cycles projection; Shift+arrows roll. --hdr-tonemap selects display tonemap (auto Hable on PQ/HLG); --hdr-nits sets display peak; --hdr-maxcll sets MaxCLL,MaxFALL. --play-stereo3d selects packed 3D view. --play-and-exit closes when the playlist stops. --start-paused opens paused. --network-caching sets demux cache ms. --snapshot-path sets snapshot directory."
                );
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}").into());
            }
            other => inputs.push(PathBuf::from(other)),
        }
        index += 1;
    }
    if inputs.is_empty() {
        return Err("play requires an input file".into());
    }
    let stats = fvid_media::play_paths(
        &inputs,
        fvid_media::PlayOptions {
            audio,
            rate,
            muted,
            fullscreen,
            on_top,
            audio_track,
            subtitle_track,
            subtitles,
            audio_device,
            start_us,
            stop_us,
            spherical,
            spherical_projection,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
            hdr_tonemap,
            hdr_nits,
            hdr_maxcll,
            hdr_maxfall,
            spherical_stereo,
            hdr_mastering_min_milli,
            hdr_mastering_max_nits,
            stereo3d,
            quit_at_end,
            start_paused,
            network_cache_ms,
            snapshot_dir,
            ..Default::default()
        },
    )?;
    emit_json(false, serde_json::to_string_pretty(&stats)?);
    Ok(())
}

#[cfg(feature = "media")]
fn run_native(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let help = "fvid media play INPUT... [--no-audio] [--mute] [--fullscreen] [--rate N] [--audio-track N] [--subtitle-track N] [--no-subtitles] [--subtitles FILE] [--audio-device NAME] [--list-audio-devices] (local file or http/https/rtsp/rtmp/udp URL) | probe INPUT | capabilities | plan [remux|transcode-lossless|trim|trim-pcm|concat|overlay|xfade|burn-subtitles|loudness|loudnorm|mix-audio|merge-audio|decode-audio] INPUT... [flags] | remux INPUT OUTPUT [--streams 0,1] [--metadata KEY=VALUE] [--metadata-delete KEY] [--stream-metadata INDEX:KEY=VALUE] [--stream-metadata-delete INDEX:KEY] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | convert-subtitles INPUT OUTPUT.mkv [--codec ass] [--streams INDEX] | burn-subtitles INPUT OUTPUT.mkv --subs FILE.srt | overlay MAIN OVERLAY OUTPUT.mkv [--overlay-x N] [--overlay-y N] | xfade MAIN OTHER OUTPUT.mkv --xfade-duration SECONDS [--transition NAME] [--xfade-offset SECONDS] | decode INPUT [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad WIDTH:HEIGHT:X:Y] [--scale WIDTH:HEIGHT] [--epx ARGS] [--pix-fmt NAME] [--colorspace ARGS] [--zscale ARGS] [--tonemap ARGS] [--yadif ARGS] [--bwdif ARGS] [--w3fdif ARGS] [--tblend ARGS] [--tmix ARGS] [--hqdn3d ARGS] [--gblur ARGS] [--eq ARGS] [--unsharp ARGS] [--hue ARGS] [--avgblur ARGS] [--boxblur ARGS] [--negate 0|1] [--edgedetect ARGS] [--sobel ARGS] [--prewitt ARGS] [--roberts ARGS] [--kirsch ARGS] [--scharr ARGS] [--atadenoise ARGS] [--owdenoise ARGS] [--vaguedenoiser ARGS] [--nlmeans ARGS] [--bm3d ARGS] [--dctdnoiz ARGS] [--fftdnoiz ARGS] [--smartblur ARGS] [--sab ARGS] [--bilateral ARGS] [--cas ARGS] [--vignette ARGS] [--curves ARGS] [--colorbalance ARGS] [--colorlevels ARGS] [--colorchannelmixer ARGS] [--deflicker ARGS] [--photosensitivity ARGS] [--monochrome ARGS] [--grayworld 0|ARGS] [--drawbox ARGS] [--drawgrid ARGS] [--lagfun ARGS] [--amplify ARGS] [--bitplanenoise ARGS] [--deband ARGS] [--gradfun ARGS] [--lenscorrection ARGS] [--pixelize ARGS] [--removegrain ARGS] [--yaepblur ARGS] [--vibrance ARGS] [--dilation ARGS] [--erosion ARGS] [--colorize ARGS] [--exposure ARGS] [--chromashift ARGS] [--colorcontrast ARGS] [--colorcorrect ARGS] [--histeq ARGS] [--shuffleplanes ARGS] [--lutyuv ARGS] [--colorhold ARGS] [--fade ARGS] [--perspective ARGS] [--lumakey ARGS] [--chromakey ARGS] [--colorkey ARGS] [--despill ARGS] [--selectivecolor ARGS] [--stereo3d ARGS] [--field ARGS] [--hqx ARGS] [--xbr ARGS] [--il ARGS] [--super2xsai ARGS] [--kerndeint ARGS] [--phase ARGS] [--estdif ARGS] [--tinterlace ARGS] [--separatefields ARGS] [--weave ARGS] [--doubleweave ARGS] [--framepack ARGS] [--telecine ARGS] [--pullup ARGS] [--decimate ARGS] [--mpdecimate ARGS] [--framestep ARGS] [--tile ARGS] [--untile ARGS] [--shuffleframes ARGS] [--reverse ARGS] [--loop ARGS] [--thumbnail ARGS] [--freezedetect ARGS] [--setpts ARGS] [--pseudocolor ARGS] [--minterpolate ARGS] [--fps RATE] [--subs FILE.srt] [--from SECONDS --to SECONDS] [--device N] | loudness INPUT [--streams INDEX] | loudnorm INPUT OUTPUT.wav [--loudnorm-args ARGS] [--dual-pass] [--streams INDEX] | decode-audio INPUT OUTPUT.wav [--streams INDEX] [--rate HZ] [--channels N] [--volume GAIN] [--from SECONDS --to SECONDS] | trim-pcm INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] | trim INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | mix-audio OUTPUT a.m4a b.m4a [c.m4a...] [--weights W,...] [--duration shortest] [--normalize|--no-normalize] | merge-audio OUTPUT a.m4a b.m4a | concat OUTPUT INPUT INPUT... [--streams 0] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | transcode INPUT OUTPUT --encoder NAME [--encoder-option KEY=VALUE] [--from SECONDS --to SECONDS [--seek]] | transcode-lossless INPUT OUTPUT.mkv [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad WIDTH:HEIGHT:X:Y] [--scale WIDTH:HEIGHT] [--epx ARGS] [--pix-fmt NAME] [--colorspace ARGS] [--zscale ARGS] [--tonemap ARGS] [--yadif ARGS] [--bwdif ARGS] [--w3fdif ARGS] [--tblend ARGS] [--tmix ARGS] [--hqdn3d ARGS] [--gblur ARGS] [--eq ARGS] [--unsharp ARGS] [--hue ARGS] [--avgblur ARGS] [--boxblur ARGS] [--negate 0|1] [--edgedetect ARGS] [--sobel ARGS] [--prewitt ARGS] [--roberts ARGS] [--kirsch ARGS] [--scharr ARGS] [--atadenoise ARGS] [--owdenoise ARGS] [--vaguedenoiser ARGS] [--nlmeans ARGS] [--bm3d ARGS] [--dctdnoiz ARGS] [--fftdnoiz ARGS] [--smartblur ARGS] [--sab ARGS] [--bilateral ARGS] [--cas ARGS] [--vignette ARGS] [--curves ARGS] [--colorbalance ARGS] [--colorlevels ARGS] [--colorchannelmixer ARGS] [--deflicker ARGS] [--photosensitivity ARGS] [--monochrome ARGS] [--grayworld 0|ARGS] [--drawbox ARGS] [--drawgrid ARGS] [--lagfun ARGS] [--amplify ARGS] [--bitplanenoise ARGS] [--deband ARGS] [--gradfun ARGS] [--lenscorrection ARGS] [--pixelize ARGS] [--removegrain ARGS] [--yaepblur ARGS] [--vibrance ARGS] [--dilation ARGS] [--erosion ARGS] [--colorize ARGS] [--exposure ARGS] [--chromashift ARGS] [--colorcontrast ARGS] [--colorcorrect ARGS] [--histeq ARGS] [--shuffleplanes ARGS] [--lutyuv ARGS] [--colorhold ARGS] [--fade ARGS] [--perspective ARGS] [--lumakey ARGS] [--chromakey ARGS] [--colorkey ARGS] [--despill ARGS] [--selectivecolor ARGS] [--stereo3d ARGS] [--field ARGS] [--hqx ARGS] [--xbr ARGS] [--il ARGS] [--super2xsai ARGS] [--kerndeint ARGS] [--phase ARGS] [--estdif ARGS] [--tinterlace ARGS] [--separatefields ARGS] [--weave ARGS] [--doubleweave ARGS] [--framepack ARGS] [--telecine ARGS] [--pullup ARGS] [--decimate ARGS] [--mpdecimate ARGS] [--framestep ARGS] [--tile ARGS] [--untile ARGS] [--shuffleframes ARGS] [--reverse ARGS] [--loop ARGS] [--thumbnail ARGS] [--freezedetect ARGS] [--setpts ARGS] [--pseudocolor ARGS] [--minterpolate ARGS] [--fps RATE] [--from SECONDS --to SECONDS [--seek]] [--streams 0,1] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | crop-lossless INPUT OUTPUT.mkv --crop X:Y:WIDTH:HEIGHT [--streams 0,1] | hw-filter INPUT OUTPUT.mp4 [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--from SECONDS --to SECONDS] [--device N]";
    let Some(command) = args.first() else {
        return Err(help.into());
    };
    if command == "--help" || command == "-h" {
        println!("{help}");
        return Ok(());
    }
    if command == "play" {
        return play_command(&args[1..]);
    }
    if command == "capabilities" {
        if args.len() != 1 {
            return Err(help.into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&fvid_media::capabilities())?
        );
        return Ok(());
    }
    if command == "probe" {
        if args.len() != 2 {
            return Err(help.into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&fvid_media::probe(&PathBuf::from(&args[1]))?)?
        );
        return Ok(());
    }
    if command == "plan" {
        // Reuse the shared option parse below by falling through with a rewritten
        // command tag after collecting paths/flags — handled after the parse loop.
    }
    let mut paths = Vec::new();
    let mut from = None;
    let mut to = None;
    let mut crop = None;
    let mut scale = None;
    let mut transpose = None;
    let mut rotate = None;
    let mut pad = None;
    let mut burn_subs = None;
    let mut overlay = None;
    let mut overlay_x = 0i32;
    let mut overlay_y = 0i32;
    let mut pix_fmt = None;
    let mut colorspace = None;
    let mut zscale = None;
    let mut tonemap = None;
    let mut yadif = None;
    let mut bwdif = None;
    let mut w3fdif = None;
    let mut tblend = None;
    let mut tmix = None;
    let mut hqdn3d = None;
    let mut gblur = None;
    let mut eq = None;
    let mut unsharp = None;
    let mut hue = None;
    let mut avgblur = None;
    let mut boxblur = None;
    let mut negate = None;
    let mut edgedetect = None;
    let mut sobel = None;
    let mut prewitt = None;
    let mut roberts = None;
    let mut kirsch = None;
    let mut scharr = None;
    let mut atadenoise = None;
    let mut owdenoise = None;
    let mut vaguedenoiser = None;
    let mut nlmeans = None;
    let mut bm3d = None;
    let mut dctdnoiz = None;
    let mut fftdnoiz = None;
    let mut smartblur = None;
    let mut sab = None;
    let mut bilateral = None;
    let mut cas = None;
    let mut epx = None;
    let mut vignette = None;
    let mut curves = None;
    let mut colorbalance = None;
    let mut colorlevels = None;
    let mut colorchannelmixer = None;
    let mut deflicker = None;
    let mut photosensitivity = None;
    let mut monochrome = None;
    let mut grayworld = None;
    let mut drawbox = None;
    let mut drawgrid = None;
    let mut lagfun = None;
    let mut amplify = None;
    let mut bitplanenoise = None;
    let mut deband = None;
    let mut gradfun = None;
    let mut lenscorrection = None;
    let mut pixelize = None;
    let mut removegrain = None;
    let mut yaepblur = None;
    let mut vibrance = None;
    let mut dilation = None;
    let mut erosion = None;
    let mut colorize = None;
    let mut exposure = None;
    let mut chromashift = None;
    let mut colorcontrast = None;
    let mut colorcorrect = None;
    let mut histeq = None;
    let mut shuffleplanes = None;
    let mut lutyuv = None;
    let mut colorhold = None;
    let mut fade = None;
    let mut perspective = None;
    let mut lumakey = None;
    let mut chromakey = None;
    let mut colorkey = None;
    let mut despill = None;
    let mut selectivecolor = None;
    let mut stereo3d = None;
    let mut field = None;
    let mut hqx = None;
    let mut xbr = None;
    let mut il = None;
    let mut super2xsai = None;
    let mut kerndeint = None;
    let mut phase = None;
    let mut estdif = None;
    let mut tinterlace = None;
    let mut separatefields = None;
    let mut weave = None;
    let mut doubleweave = None;
    let mut framepack = None;
    let mut telecine = None;
    let mut pullup = None;
    let mut decimate = None;
    let mut mpdecimate = None;
    let mut framestep = None;
    let mut tile = None;
    let mut untile = None;
    let mut shuffleframes = None;
    let mut reverse = None;
    let mut vloop = None;
    let mut thumbnail = None;
    let mut freezedetect = None;
    let mut setpts = None;
    let mut select = None;
    let mut pseudocolor = None;
    let mut minterpolate = None;
    let mut fps = None;
    let mut xfade_transition = None;
    let mut xfade_duration = None;
    let mut xfade_offset = None;
    let mut loudnorm_args = None;
    let mut loudnorm_dual = false;
    let mut mix_duration = fvid_media::MixDuration::Shortest;
    let mut mix_normalize = true;
    let mut mix_weights = Vec::new();
    let mut sample_rate = None;
    let mut channels = None;
    let mut volume = None;
    let mut vertical_flip = false;
    let mut horizontal_flip = false;
    let mut seek = false;
    let mut encoder = None;
    let mut encoder_options = Vec::new();
    let mut subtitle_codec = fvid_media::SubtitleCodec::Ass;
    let mut device = 0usize;
    let mut device_set = false;
    let mut quiet = false;
    let mut progress = false;
    let mut options = fvid_media::CopyOptions::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" => progress = true,
            "--encoder" => {
                i += 1;
                encoder = Some(args.get(i).ok_or("missing encoder name")?.clone());
            }
            "--codec" => {
                i += 1;
                subtitle_codec =
                    fvid_media::SubtitleCodec::parse(args.get(i).ok_or("missing subtitle codec")?)?;
            }
            "--encoder-option" => {
                i += 1;
                let (key, value) = args
                    .get(i)
                    .ok_or("missing encoder option")?
                    .split_once('=')
                    .ok_or("encoder option must be KEY=VALUE")?;
                encoder_options.push((key.to_owned(), value.to_owned()));
            }
            "--device" => {
                i += 1;
                device = args.get(i).ok_or("missing device")?.parse()?;
                device_set = true;
            }
            "--seek" => seek = true,
            "--hflip" => horizontal_flip = true,
            "--vflip" => vertical_flip = true,
            "--crop" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing crop")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<usize>, _>>()?;
                if values.len() != 4 {
                    return Err("crop must be X:Y:WIDTH:HEIGHT".into());
                }
                crop = Some(fvid_media::CropRect {
                    x: values[0],
                    y: values[1],
                    width: values[2],
                    height: values[3],
                });
            }
            "--scale" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing scale")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<u32>, _>>()?;
                if values.len() != 2 {
                    return Err("scale must be WIDTH:HEIGHT".into());
                }
                scale = Some(fvid_media::ScaleSize {
                    width: values[0],
                    height: values[1],
                });
            }
            "--transpose" => {
                i += 1;
                transpose = Some(fvid_media::TransposeMode::parse(
                    args.get(i).ok_or("missing transpose mode")?,
                )?);
            }
            "--rotate" => {
                i += 1;
                rotate = Some(fvid_media::RotateAngle::parse(
                    args.get(i).ok_or("missing rotate degrees")?,
                )?);
            }
            "--pad" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing pad")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<u32>, _>>()?;
                if values.len() != 4 {
                    return Err("pad must be WIDTH:HEIGHT:X:Y".into());
                }
                pad = Some(fvid_media::PadRect {
                    width: values[0],
                    height: values[1],
                    x: values[2],
                    y: values[3],
                });
            }
            "--subs" => {
                i += 1;
                burn_subs = Some(std::path::PathBuf::from(
                    args.get(i).ok_or("missing subtitle file")?,
                ));
            }
            "--overlay" => {
                i += 1;
                overlay = Some(std::path::PathBuf::from(
                    args.get(i).ok_or("missing overlay video")?,
                ));
            }
            "--overlay-x" => {
                i += 1;
                overlay_x = args
                    .get(i)
                    .ok_or("missing overlay-x")?
                    .parse()
                    .map_err(|_| "overlay-x must be an integer")?;
            }
            "--overlay-y" => {
                i += 1;
                overlay_y = args
                    .get(i)
                    .ok_or("missing overlay-y")?
                    .parse()
                    .map_err(|_| "overlay-y must be an integer")?;
            }
            "--pix-fmt" => {
                i += 1;
                pix_fmt = Some(args.get(i).ok_or("missing pixel format")?.clone());
            }
            "--colorspace" => {
                i += 1;
                colorspace = Some(args.get(i).ok_or("missing colorspace args")?.clone());
            }
            "--zscale" => {
                i += 1;
                zscale = Some(args.get(i).ok_or("missing zscale args")?.clone());
            }
            "--tonemap" => {
                i += 1;
                tonemap = Some(args.get(i).ok_or("missing tonemap args")?.clone());
            }
            "--yadif" => {
                i += 1;
                yadif = Some(args.get(i).ok_or("missing yadif args")?.clone());
            }
            "--bwdif" => {
                i += 1;
                bwdif = Some(args.get(i).ok_or("missing bwdif args")?.clone());
            }
            "--w3fdif" => {
                i += 1;
                w3fdif = Some(args.get(i).ok_or("missing w3fdif args")?.clone());
            }
            "--tblend" => {
                i += 1;
                tblend = Some(args.get(i).ok_or("missing tblend args")?.clone());
            }
            "--tmix" => {
                i += 1;
                tmix = Some(args.get(i).ok_or("missing tmix args")?.clone());
            }
            "--hqdn3d" => {
                i += 1;
                hqdn3d = Some(args.get(i).ok_or("missing hqdn3d args")?.clone());
            }
            "--gblur" => {
                i += 1;
                gblur = Some(args.get(i).ok_or("missing gblur args")?.clone());
            }
            "--eq" => {
                i += 1;
                eq = Some(args.get(i).ok_or("missing eq args")?.clone());
            }
            "--unsharp" => {
                i += 1;
                unsharp = Some(args.get(i).ok_or("missing unsharp args")?.clone());
            }
            "--hue" => {
                i += 1;
                hue = Some(args.get(i).ok_or("missing hue args")?.clone());
            }
            "--avgblur" => {
                i += 1;
                avgblur = Some(args.get(i).ok_or("missing avgblur args")?.clone());
            }
            "--boxblur" => {
                i += 1;
                boxblur = Some(args.get(i).ok_or("missing boxblur args")?.clone());
            }
            "--negate" => {
                i += 1;
                negate = Some(args.get(i).ok_or("missing negate args (0|1)")?.clone());
            }
            "--edgedetect" => {
                i += 1;
                edgedetect = Some(args.get(i).ok_or("missing edgedetect args")?.clone());
            }
            "--sobel" => {
                i += 1;
                sobel = Some(args.get(i).ok_or("missing sobel args")?.clone());
            }
            "--prewitt" => {
                i += 1;
                prewitt = Some(args.get(i).ok_or("missing prewitt args")?.clone());
            }
            "--roberts" => {
                i += 1;
                roberts = Some(args.get(i).ok_or("missing roberts args")?.clone());
            }
            "--kirsch" => {
                i += 1;
                kirsch = Some(args.get(i).ok_or("missing kirsch args")?.clone());
            }
            "--scharr" => {
                i += 1;
                scharr = Some(args.get(i).ok_or("missing scharr args")?.clone());
            }
            "--atadenoise" => {
                i += 1;
                atadenoise = Some(args.get(i).ok_or("missing atadenoise args")?.clone());
            }
            "--owdenoise" => {
                i += 1;
                owdenoise = Some(args.get(i).ok_or("missing owdenoise args")?.clone());
            }
            "--vaguedenoiser" => {
                i += 1;
                vaguedenoiser = Some(args.get(i).ok_or("missing vaguedenoiser args")?.clone());
            }
            "--nlmeans" => {
                i += 1;
                nlmeans = Some(args.get(i).ok_or("missing nlmeans args")?.clone());
            }
            "--bm3d" => {
                i += 1;
                bm3d = Some(args.get(i).ok_or("missing bm3d args")?.clone());
            }
            "--dctdnoiz" => {
                i += 1;
                dctdnoiz = Some(args.get(i).ok_or("missing dctdnoiz args")?.clone());
            }
            "--fftdnoiz" => {
                i += 1;
                fftdnoiz = Some(args.get(i).ok_or("missing fftdnoiz args")?.clone());
            }
            "--smartblur" => {
                i += 1;
                smartblur = Some(args.get(i).ok_or("missing smartblur args")?.clone());
            }
            "--sab" => {
                i += 1;
                sab = Some(args.get(i).ok_or("missing sab args")?.clone());
            }
            "--bilateral" => {
                i += 1;
                bilateral = Some(args.get(i).ok_or("missing bilateral args")?.clone());
            }
            "--cas" => {
                i += 1;
                cas = Some(args.get(i).ok_or("missing cas args")?.clone());
            }
            "--epx" => {
                i += 1;
                epx = Some(args.get(i).ok_or("missing epx args")?.clone());
            }
            "--vignette" => {
                i += 1;
                vignette = Some(args.get(i).ok_or("missing vignette args")?.clone());
            }
            "--curves" => {
                i += 1;
                curves = Some(args.get(i).ok_or("missing curves args")?.clone());
            }
            "--colorbalance" => {
                i += 1;
                colorbalance = Some(args.get(i).ok_or("missing colorbalance args")?.clone());
            }
            "--colorlevels" => {
                i += 1;
                colorlevels = Some(args.get(i).ok_or("missing colorlevels args")?.clone());
            }
            "--colorchannelmixer" => {
                i += 1;
                colorchannelmixer =
                    Some(args.get(i).ok_or("missing colorchannelmixer args")?.clone());
            }
            "--deflicker" => {
                i += 1;
                deflicker = Some(args.get(i).ok_or("missing deflicker args")?.clone());
            }
            "--photosensitivity" => {
                i += 1;
                photosensitivity =
                    Some(args.get(i).ok_or("missing photosensitivity args")?.clone());
            }
            "--monochrome" => {
                i += 1;
                monochrome = Some(args.get(i).ok_or("missing monochrome args")?.clone());
            }
            "--grayworld" => {
                i += 1;
                grayworld = Some(
                    args.get(i)
                        .ok_or("missing grayworld args (0 for defaults)")?
                        .clone(),
                );
            }
            "--drawbox" => {
                i += 1;
                drawbox = Some(args.get(i).ok_or("missing drawbox args")?.clone());
            }
            "--drawgrid" => {
                i += 1;
                drawgrid = Some(args.get(i).ok_or("missing drawgrid args")?.clone());
            }
            "--lagfun" => {
                i += 1;
                lagfun = Some(args.get(i).ok_or("missing lagfun args")?.clone());
            }
            "--amplify" => {
                i += 1;
                amplify = Some(args.get(i).ok_or("missing amplify args")?.clone());
            }
            "--bitplanenoise" => {
                i += 1;
                bitplanenoise = Some(args.get(i).ok_or("missing bitplanenoise args")?.clone());
            }
            "--deband" => {
                i += 1;
                deband = Some(args.get(i).ok_or("missing deband args")?.clone());
            }
            "--gradfun" => {
                i += 1;
                gradfun = Some(args.get(i).ok_or("missing gradfun args")?.clone());
            }
            "--lenscorrection" => {
                i += 1;
                lenscorrection = Some(args.get(i).ok_or("missing lenscorrection args")?.clone());
            }
            "--pixelize" => {
                i += 1;
                pixelize = Some(args.get(i).ok_or("missing pixelize args")?.clone());
            }
            "--removegrain" => {
                i += 1;
                removegrain = Some(args.get(i).ok_or("missing removegrain args")?.clone());
            }
            "--yaepblur" => {
                i += 1;
                yaepblur = Some(args.get(i).ok_or("missing yaepblur args")?.clone());
            }
            "--vibrance" => {
                i += 1;
                vibrance = Some(args.get(i).ok_or("missing vibrance args")?.clone());
            }
            "--dilation" => {
                i += 1;
                dilation = Some(args.get(i).ok_or("missing dilation args")?.clone());
            }
            "--erosion" => {
                i += 1;
                erosion = Some(args.get(i).ok_or("missing erosion args")?.clone());
            }
            "--colorize" => {
                i += 1;
                colorize = Some(args.get(i).ok_or("missing colorize args")?.clone());
            }
            "--exposure" => {
                i += 1;
                exposure = Some(args.get(i).ok_or("missing exposure args")?.clone());
            }
            "--chromashift" => {
                i += 1;
                chromashift = Some(args.get(i).ok_or("missing chromashift args")?.clone());
            }
            "--colorcontrast" => {
                i += 1;
                colorcontrast = Some(args.get(i).ok_or("missing colorcontrast args")?.clone());
            }
            "--colorcorrect" => {
                i += 1;
                colorcorrect = Some(args.get(i).ok_or("missing colorcorrect args")?.clone());
            }
            "--histeq" => {
                i += 1;
                histeq = Some(args.get(i).ok_or("missing histeq args")?.clone());
            }
            "--shuffleplanes" => {
                i += 1;
                shuffleplanes = Some(args.get(i).ok_or("missing shuffleplanes args")?.clone());
            }
            "--lutyuv" => {
                i += 1;
                lutyuv = Some(args.get(i).ok_or("missing lutyuv args")?.clone());
            }
            "--colorhold" => {
                i += 1;
                colorhold = Some(args.get(i).ok_or("missing colorhold args")?.clone());
            }
            "--fade" => {
                i += 1;
                fade = Some(args.get(i).ok_or("missing fade args")?.clone());
            }
            "--perspective" => {
                i += 1;
                perspective = Some(args.get(i).ok_or("missing perspective args")?.clone());
            }
            "--lumakey" => {
                i += 1;
                lumakey = Some(args.get(i).ok_or("missing lumakey args")?.clone());
            }
            "--chromakey" => {
                i += 1;
                chromakey = Some(args.get(i).ok_or("missing chromakey args")?.clone());
            }
            "--colorkey" => {
                i += 1;
                colorkey = Some(args.get(i).ok_or("missing colorkey args")?.clone());
            }
            "--despill" => {
                i += 1;
                despill = Some(args.get(i).ok_or("missing despill args")?.clone());
            }
            "--selectivecolor" => {
                i += 1;
                selectivecolor = Some(args.get(i).ok_or("missing selectivecolor args")?.clone());
            }
            "--stereo3d" => {
                i += 1;
                stereo3d = Some(args.get(i).ok_or("missing stereo3d args")?.clone());
            }
            "--field" => {
                i += 1;
                field = Some(args.get(i).ok_or("missing field args")?.clone());
            }
            "--hqx" => {
                i += 1;
                hqx = Some(args.get(i).ok_or("missing hqx args")?.clone());
            }
            "--xbr" => {
                i += 1;
                xbr = Some(args.get(i).ok_or("missing xbr args")?.clone());
            }
            "--il" => {
                i += 1;
                il = Some(args.get(i).ok_or("missing il args")?.clone());
            }
            "--super2xsai" => {
                i += 1;
                super2xsai = Some(args.get(i).ok_or("missing super2xsai args")?.clone());
            }
            "--kerndeint" => {
                i += 1;
                kerndeint = Some(args.get(i).ok_or("missing kerndeint args")?.clone());
            }
            "--phase" => {
                i += 1;
                phase = Some(args.get(i).ok_or("missing phase args")?.clone());
            }
            "--estdif" => {
                i += 1;
                estdif = Some(args.get(i).ok_or("missing estdif args")?.clone());
            }
            "--tinterlace" => {
                i += 1;
                tinterlace = Some(args.get(i).ok_or("missing tinterlace args")?.clone());
            }
            "--separatefields" => {
                i += 1;
                separatefields = Some(args.get(i).ok_or("missing separatefields args")?.clone());
            }
            "--weave" => {
                i += 1;
                weave = Some(args.get(i).ok_or("missing weave args")?.clone());
            }
            "--doubleweave" => {
                i += 1;
                doubleweave = Some(args.get(i).ok_or("missing doubleweave args")?.clone());
            }
            "--framepack" => {
                i += 1;
                framepack = Some(args.get(i).ok_or("missing framepack args")?.clone());
            }
            "--telecine" => {
                i += 1;
                telecine = Some(args.get(i).ok_or("missing telecine args")?.clone());
            }
            "--pullup" => {
                i += 1;
                pullup = Some(args.get(i).ok_or("missing pullup args")?.clone());
            }
            "--decimate" => {
                i += 1;
                decimate = Some(args.get(i).ok_or("missing decimate args")?.clone());
            }
            "--mpdecimate" => {
                i += 1;
                mpdecimate = Some(args.get(i).ok_or("missing mpdecimate args")?.clone());
            }
            "--framestep" => {
                i += 1;
                framestep = Some(args.get(i).ok_or("missing framestep args")?.clone());
            }
            "--tile" => {
                i += 1;
                tile = Some(args.get(i).ok_or("missing tile args")?.clone());
            }
            "--untile" => {
                i += 1;
                untile = Some(args.get(i).ok_or("missing untile args")?.clone());
            }
            "--shuffleframes" => {
                i += 1;
                shuffleframes = Some(args.get(i).ok_or("missing shuffleframes args")?.clone());
            }
            "--reverse" => {
                i += 1;
                reverse = Some(args.get(i).ok_or("missing reverse args")?.clone());
            }
            "--loop" => {
                i += 1;
                vloop = Some(args.get(i).ok_or("missing loop args")?.clone());
            }
            "--thumbnail" => {
                i += 1;
                thumbnail = Some(args.get(i).ok_or("missing thumbnail args")?.clone());
            }
            "--freezedetect" => {
                i += 1;
                freezedetect = Some(args.get(i).ok_or("missing freezedetect args")?.clone());
            }
            "--setpts" => {
                i += 1;
                setpts = Some(args.get(i).ok_or("missing setpts args")?.clone());
            }
            "--select" => {
                i += 1;
                select = Some(args.get(i).ok_or("missing select args")?.clone());
            }
            "--pseudocolor" => {
                i += 1;
                pseudocolor = Some(args.get(i).ok_or("missing pseudocolor args")?.clone());
            }
            "--minterpolate" => {
                i += 1;
                minterpolate = Some(args.get(i).ok_or("missing minterpolate args")?.clone());
            }
            "--fps" => {
                i += 1;
                fps = Some(args.get(i).ok_or("missing fps rate")?.clone());
            }
            "--transition" => {
                i += 1;
                xfade_transition = Some(args.get(i).ok_or("missing xfade transition")?.clone());
            }
            "--xfade-duration" => {
                i += 1;
                xfade_duration = Some(fvid_media::parse_time(
                    args.get(i).ok_or("missing xfade duration")?,
                )?);
            }
            "--xfade-offset" => {
                i += 1;
                xfade_offset = Some(fvid_media::parse_time(
                    args.get(i).ok_or("missing xfade offset")?,
                )?);
            }
            "--loudnorm-args" => {
                i += 1;
                loudnorm_args = Some(args.get(i).ok_or("missing loudnorm args")?.clone());
            }
            "--dual-pass" => loudnorm_dual = true,
            "--duration" => {
                i += 1;
                mix_duration =
                    fvid_media::MixDuration::parse(args.get(i).ok_or("missing mix duration")?)?;
            }
            "--normalize" => mix_normalize = true,
            "--no-normalize" => mix_normalize = false,
            "--weights" => {
                i += 1;
                mix_weights = args
                    .get(i)
                    .ok_or("missing mix weights")?
                    .split(|c| c == ',' || c == ' ')
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        part.parse::<f32>()
                            .map_err(|_| "mix weight must be a number")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if mix_weights.is_empty() {
                    return Err("mix weights must not be empty".into());
                }
            }
            "--rate" => {
                i += 1;
                sample_rate = Some(
                    args.get(i)
                        .ok_or("missing sample rate")?
                        .parse::<i32>()
                        .map_err(|_| "sample rate must be an integer")?,
                );
            }
            "--channels" => {
                i += 1;
                channels = Some(
                    args.get(i)
                        .ok_or("missing channel count")?
                        .parse::<i32>()
                        .map_err(|_| "channels must be an integer")?,
                );
            }
            "--volume" => {
                i += 1;
                volume = Some(
                    args.get(i)
                        .ok_or("missing volume")?
                        .parse::<f64>()
                        .map_err(|_| "volume must be a linear gain number")?,
                );
            }
            "--streams" => {
                i += 1;
                options.streams = args
                    .get(i)
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--max-packets" => {
                i += 1;
                options.max_packets = Some(
                    args.get(i)
                        .ok_or("missing max-packets")?
                        .parse()
                        .map_err(|_| "invalid max-packets")?,
                );
            }
            "--max-memory-mib" => {
                i += 1;
                options.max_controlled_bytes = Some(fvid_media::parse_max_memory_mib(
                    args.get(i).ok_or("missing max-memory-mib")?,
                )?);
            }
            "--max-rss-mib" => {
                i += 1;
                options.max_rss_bytes = Some(fvid_media::parse_max_rss_mib(
                    args.get(i).ok_or("missing max-rss-mib")?,
                )?);
            }
            "--metadata" => {
                i += 1;
                let (key, value) = args
                    .get(i)
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                i += 1;
                options
                    .metadata_delete
                    .push(args.get(i).ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                i += 1;
                let raw = args.get(i).ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                i += 1;
                let raw = args.get(i).ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--from" => {
                i += 1;
                from = Some(fvid_media::parse_time(args.get(i).ok_or("missing start")?)?);
            }
            "--to" => {
                i += 1;
                to = Some(fvid_media::parse_time(args.get(i).ok_or("missing end")?)?);
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown media option: {value}").into());
            }
            value => paths.push(PathBuf::from(value)),
        }
        i += 1;
    }
    if progress {
        options.progress = Some(fvid_media::ProgressHook::new(|event| {
            eprintln!(
                "{}",
                serde_json::json!({
                    "packets": event.packets,
                    "payload_bytes": event.payload_bytes,
                    "done": event.done,
                })
            );
        }));
    }
    let metadata_mutation = !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty();
    if command == "plan" {
        if metadata_mutation || encoder.is_some() || !encoder_options.is_empty() {
            return Err("plan does not take metadata mutation or encoder selection".into());
        }
        let mut plan_paths = paths;
        let sub = if plan_paths
            .first()
            .and_then(|p| p.to_str())
            .is_some_and(|s| {
                matches!(
                    s,
                    "trim"
                        | "trim-pcm"
                        | "concat"
                        | "remux"
                        | "transcode-lossless"
                        | "overlay"
                        | "xfade"
                        | "burn-subtitles"
                        | "loudness"
                        | "loudnorm"
                        | "mix-audio"
                        | "merge-audio"
                        | "decode-audio"
                )
            }) {
            Some(plan_paths.remove(0))
        } else {
            None
        };
        let transform = fvid_media::LosslessTransform {
            crop,
            vertical_flip,
            horizontal_flip,
            scale,
            epx: epx.clone(),
            transpose,
            rotate,
            pad,
            burn_subs,
            overlay: overlay.map(|path| fvid_media::OverlaySpec {
                path,
                x: overlay_x,
                y: overlay_y,
            }),
            xfade: None,
            yadif: yadif.clone(),
            bwdif: bwdif.clone(),
            w3fdif: w3fdif.clone(),
            tblend: tblend.clone(),
            tmix: tmix.clone(),
            hqdn3d: hqdn3d.clone(),
            gblur: gblur.clone(),
            eq: eq.clone(),
            unsharp: unsharp.clone(),
            hue: hue.clone(),
            avgblur: avgblur.clone(),
            boxblur: boxblur.clone(),
            negate: negate.clone(),
            edgedetect: edgedetect.clone(),
            sobel: sobel.clone(),
            prewitt: prewitt.clone(),
            roberts: roberts.clone(),
            kirsch: kirsch.clone(),
            scharr: scharr.clone(),
            atadenoise: atadenoise.clone(),
            owdenoise: owdenoise.clone(),
            vaguedenoiser: vaguedenoiser.clone(),
            nlmeans: nlmeans.clone(),
            bm3d: bm3d.clone(),
            dctdnoiz: dctdnoiz.clone(),
            fftdnoiz: fftdnoiz.clone(),
            smartblur: smartblur.clone(),
            sab: sab.clone(),
            bilateral: bilateral.clone(),
            cas: cas.clone(),
            vignette: vignette.clone(),
            curves: curves.clone(),
            colorbalance: colorbalance.clone(),
            colorlevels: colorlevels.clone(),
            colorchannelmixer: colorchannelmixer.clone(),
            deflicker: deflicker.clone(),
            photosensitivity: photosensitivity.clone(),
            monochrome: monochrome.clone(),
            grayworld: grayworld.clone(),
            drawbox: drawbox.clone(),
            drawgrid: drawgrid.clone(),
            lagfun: lagfun.clone(),
            amplify: amplify.clone(),
            bitplanenoise: bitplanenoise.clone(),
            deband: deband.clone(),
            gradfun: gradfun.clone(),
            lenscorrection: lenscorrection.clone(),
            pixelize: pixelize.clone(),
            removegrain: removegrain.clone(),
            yaepblur: yaepblur.clone(),
            vibrance: vibrance.clone(),
            dilation: dilation.clone(),
            erosion: erosion.clone(),
            colorize: colorize.clone(),
            exposure: exposure.clone(),
            chromashift: chromashift.clone(),
            colorcontrast: colorcontrast.clone(),
            colorcorrect: colorcorrect.clone(),
            histeq: histeq.clone(),
            shuffleplanes: shuffleplanes.clone(),
            lutyuv: lutyuv.clone(),
            colorhold: colorhold.clone(),
            fade: fade.clone(),
            perspective: perspective.clone(),
            lumakey: lumakey.clone(),
            chromakey: chromakey.clone(),
            colorkey: colorkey.clone(),
            despill: despill.clone(),
            selectivecolor: selectivecolor.clone(),
            stereo3d: stereo3d.clone(),
            field: field.clone(),
            hqx: hqx.clone(),
            xbr: xbr.clone(),
            il: il.clone(),
            super2xsai: super2xsai.clone(),
            kerndeint: kerndeint.clone(),
            phase: phase.clone(),
            estdif: estdif.clone(),
            tinterlace: tinterlace.clone(),
            separatefields: separatefields.clone(),
            weave: weave.clone(),
            doubleweave: doubleweave.clone(),
            framepack: framepack.clone(),
            telecine: telecine.clone(),
            pullup: pullup.clone(),
            decimate: decimate.clone(),
            mpdecimate: mpdecimate.clone(),
            framestep: framestep.clone(),
            tile: tile.clone(),
            untile: untile.clone(),
            shuffleframes: shuffleframes.clone(),
            reverse: reverse.clone(),
            r#loop: vloop.clone(),
            thumbnail: thumbnail.clone(),
            freezedetect: freezedetect.clone(),
            pseudocolor: pseudocolor.clone(),
            minterpolate: minterpolate.clone(),
            fps: fps.clone(),
            colorspace: colorspace.clone(),
            zscale: zscale.clone(),
            tonemap: tonemap.clone(),
            pix_fmt: pix_fmt.clone(),
            interval: match (from, to) {
                (None, None) => None,
                (Some(start), Some(end)) => Some((start, end)),
                _ => return Err("plan interval requires both --from and --to".into()),
            },
            seek,
        };
        let geometry = transform.crop.is_some()
            || transform.vertical_flip
            || transform.horizontal_flip
            || transform.scale.is_some()
            || transform.transpose.is_some()
            || transform.rotate.is_some()
            || transform.pad.is_some()
            || transform.burn_subs.is_some()
            || transform.overlay.is_some()
            || transform.yadif.is_some()
            || transform.bwdif.is_some()
            || transform.w3fdif.is_some()
            || transform.tblend.is_some()
            || transform.tmix.is_some()
            || transform.hqdn3d.is_some()
            || transform.gblur.is_some()
            || transform.eq.is_some()
            || transform.unsharp.is_some()
            || transform.hue.is_some()
            || transform.avgblur.is_some()
            || transform.boxblur.is_some()
            || transform.negate.is_some()
            || transform.edgedetect.is_some()
            || transform.sobel.is_some()
            || transform.prewitt.is_some()
            || transform.roberts.is_some()
            || transform.kirsch.is_some()
            || transform.scharr.is_some()
            || transform.atadenoise.is_some()
            || transform.owdenoise.is_some()
            || transform.vaguedenoiser.is_some()
            || transform.nlmeans.is_some()
            || transform.bm3d.is_some()
            || transform.dctdnoiz.is_some()
            || transform.fftdnoiz.is_some()
            || transform.smartblur.is_some()
            || transform.sab.is_some()
            || transform.bilateral.is_some()
            || transform.cas.is_some()
            || transform.epx.is_some()
            || transform.vignette.is_some()
            || transform.curves.is_some()
            || transform.colorbalance.is_some()
            || transform.colorlevels.is_some()
            || transform.colorchannelmixer.is_some()
            || transform.deflicker.is_some()
            || transform.photosensitivity.is_some()
            || transform.monochrome.is_some()
            || transform.grayworld.is_some()
            || transform.drawbox.is_some()
            || transform.drawgrid.is_some()
            || transform.lagfun.is_some()
            || transform.amplify.is_some()
            || transform.bitplanenoise.is_some()
            || transform.deband.is_some()
            || transform.gradfun.is_some()
            || transform.lenscorrection.is_some()
            || transform.pixelize.is_some()
            || transform.removegrain.is_some()
            || transform.yaepblur.is_some()
            || transform.vibrance.is_some()
            || transform.dilation.is_some()
            || transform.erosion.is_some()
            || transform.colorize.is_some()
            || transform.exposure.is_some()
            || transform.chromashift.is_some()
            || transform.colorcontrast.is_some()
            || transform.colorcorrect.is_some()
            || transform.histeq.is_some()
            || transform.shuffleplanes.is_some()
            || transform.lutyuv.is_some()
            || transform.colorhold.is_some()
            || transform.fade.is_some()
            || transform.perspective.is_some()
            || transform.lumakey.is_some()
            || transform.chromakey.is_some()
            || transform.colorkey.is_some()
            || transform.despill.is_some()
            || transform.selectivecolor.is_some()
            || transform.stereo3d.is_some()
            || transform.field.is_some()
            || transform.hqx.is_some()
            || transform.xbr.is_some()
            || transform.il.is_some()
            || transform.super2xsai.is_some()
            || transform.kerndeint.is_some()
            || transform.phase.is_some()
            || transform.estdif.is_some()
            || transform.tinterlace.is_some()
            || transform.separatefields.is_some()
            || transform.weave.is_some()
            || transform.doubleweave.is_some()
            || transform.framepack.is_some()
            || transform.telecine.is_some()
            || transform.pullup.is_some()
            || transform.decimate.is_some()
            || transform.mpdecimate.is_some()
            || transform.framestep.is_some()
            || transform.tile.is_some()
            || transform.untile.is_some()
            || transform.shuffleframes.is_some()
            || transform.reverse.is_some()
            || transform.r#loop.is_some()
            || transform.thumbnail.is_some()
            || transform.freezedetect.is_some()
            || setpts.is_some()
            || select.is_some()
            || transform.pseudocolor.is_some()
            || transform.minterpolate.is_some()
            || transform.fps.is_some()
            || transform.colorspace.is_some()
            || transform.zscale.is_some()
            || transform.tonemap.is_some()
            || transform.pix_fmt.is_some()
            || transform.seek;
        let plan = match sub.as_ref().and_then(|p| p.to_str()) {
            Some("trim") => {
                if plan_paths.len() != 1 || geometry {
                    return Err(
                        "plan trim requires INPUT --from SECONDS --to SECONDS without geometry flags"
                            .into(),
                    );
                }
                let (start, end) = match (from, to) {
                    (Some(start), Some(end)) => (start, end),
                    _ => return Err("plan trim requires both --from and --to".into()),
                };
                fvid_media::plan_trim(&plan_paths[0], start, end, &options)?
            }
            Some("trim-pcm") => {
                if plan_paths.len() != 1 || geometry {
                    return Err(
                        "plan trim-pcm requires INPUT --from SECONDS --to SECONDS [--streams INDEX] without geometry flags"
                            .into(),
                    );
                }
                let (start, end) = match (from, to) {
                    (Some(start), Some(end)) => (start, end),
                    _ => return Err("plan trim-pcm requires both --from and --to".into()),
                };
                fvid_media::plan_trim_pcm(&plan_paths[0], start, end, &options)?
            }
            Some("concat") => {
                if plan_paths.len() < 2 || from.is_some() || to.is_some() || geometry {
                    return Err(
                        "plan concat requires INPUT INPUT... without --from/--to/geometry flags"
                            .into(),
                    );
                }
                fvid_media::plan_concat(&plan_paths, &options)?
            }
            Some("remux") => {
                if plan_paths.len() != 1 || from.is_some() || to.is_some() || geometry {
                    return Err("plan remux requires a single INPUT without transform flags".into());
                }
                fvid_media::plan_remux(&plan_paths[0], &options)?
            }
            Some("transcode-lossless") => {
                if plan_paths.len() != 1 {
                    return Err("plan transcode-lossless requires a single INPUT".into());
                }
                fvid_media::plan_transcode_lossless(&plan_paths[0], &transform, &options, None)?
            }
            Some("overlay") => {
                if plan_paths.len() != 1 || transform.overlay.is_none() {
                    return Err(
                        "plan overlay requires MAIN --overlay FILE [--overlay-x N] [--overlay-y N]"
                            .into(),
                    );
                }
                let spec = transform.overlay.as_ref().unwrap();
                fvid_media::plan_overlay(&plan_paths[0], &spec.path, spec.x, spec.y, &options)?
            }
            Some("xfade") => {
                if plan_paths.len() != 2 {
                    return Err(
                        "plan xfade requires MAIN OTHER --xfade-duration SECONDS [--transition NAME] [--xfade-offset SECONDS]"
                            .into(),
                    );
                }
                let duration = xfade_duration.ok_or("plan xfade requires --xfade-duration")?;
                let transition = xfade_transition.as_deref().unwrap_or("fade");
                let offset = xfade_offset.unwrap_or(0);
                fvid_media::plan_xfade(
                    &plan_paths[0],
                    &plan_paths[1],
                    transition,
                    duration,
                    offset,
                    &options,
                )?
            }
            Some("burn-subtitles") => {
                if plan_paths.len() != 1 || transform.burn_subs.is_none() {
                    return Err("plan burn-subtitles requires INPUT --subs FILE.srt".into());
                }
                fvid_media::plan_burn_subtitles(
                    &plan_paths[0],
                    transform.burn_subs.as_ref().unwrap(),
                    &options,
                )?
            }
            Some("loudness") => {
                if plan_paths.len() != 1 || geometry {
                    return Err("plan loudness requires INPUT without video transform flags".into());
                }
                fvid_media::plan_loudness(&plan_paths[0], &options)?
            }
            Some("loudnorm") => {
                if plan_paths.len() != 1 || geometry {
                    return Err(
                        "plan loudnorm requires INPUT [--loudnorm-args ARGS] [--dual-pass] without video transform flags"
                            .into(),
                    );
                }
                fvid_media::plan_loudnorm(
                    &plan_paths[0],
                    loudnorm_args.as_deref(),
                    loudnorm_dual,
                    &options,
                )?
            }
            Some("mix-audio") => {
                if plan_paths.len() < 2 || geometry {
                    return Err(
                        "plan mix-audio requires INPUT INPUT... [--weights W,...] [--duration shortest] [--normalize|--no-normalize]"
                            .into(),
                    );
                }
                fvid_media::plan_mix_audio(
                    &plan_paths,
                    &fvid_media::MixAudioOptions {
                        duration: mix_duration,
                        normalize: mix_normalize,
                        weights: mix_weights,
                    },
                )?
            }
            Some("merge-audio") => {
                if plan_paths.len() != 2 || geometry {
                    return Err("plan merge-audio requires INPUT INPUT".into());
                }
                fvid_media::plan_merge_audio(&plan_paths)?
            }
            Some("decode-audio") => {
                if plan_paths.len() != 1 || geometry {
                    return Err(
                        "plan decode-audio requires INPUT [--rate HZ] [--channels N] [--volume GAIN] [--from SECONDS --to SECONDS] [--streams INDEX] without video transform flags"
                            .into(),
                    );
                }
                let interval = match (from, to) {
                    (None, None) => None,
                    (Some(start), Some(end)) => Some((start, end)),
                    _ => {
                        return Err(
                            "plan decode-audio interval requires both --from and --to".into()
                        );
                    }
                };
                fvid_media::plan_decode_audio(
                    &plan_paths[0],
                    &fvid_media::AudioDecodeTransform {
                        interval,
                        sample_rate,
                        channels,
                        volume,
                    },
                    &options,
                )?
            }
            None => {
                if plan_paths.len() != 1 {
                    return Err(
                        "plan requires INPUT, or: plan trim|trim-pcm|concat|remux|transcode-lossless|overlay|xfade|burn-subtitles|loudness|loudnorm|mix-audio|merge-audio|decode-audio ..."
                            .into(),
                    );
                }
                let idle = !geometry && transform.interval.is_none();
                if idle {
                    fvid_media::plan_remux(&plan_paths[0], &options)?
                } else {
                    fvid_media::plan_transcode_lossless(&plan_paths[0], &transform, &options, None)?
                }
            }
            Some(_) => unreachable!(),
        };
        emit_json(quiet, serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    if metadata_mutation && command != "remux" {
        return Err("metadata mutation requires remux".into());
    }
    if command != "transcode" && (encoder.is_some() || !encoder_options.is_empty()) {
        return Err("encoder selection requires explicit transcode command".into());
    }
    if command == "hw-filter" && paths.len() == 2 {
        #[cfg(feature = "media-cuda")]
        {
            let interval = match (from, to) {
                (None, None) => None,
                (Some(start), Some(end)) => Some((start, end)),
                _ => return Err("hw-filter interval requires both --from and --to".into()),
            };
            let stats = fvid_media::hw_filter(
                &paths[0],
                &paths[1],
                &fvid_media::HwFilterOptions {
                    crop,
                    horizontal_flip,
                    vertical_flip,
                    device,
                    host_bounce: false,
                    interval,
                    ..Default::default()
                },
            )?;
            emit_json(quiet, serde_json::to_string_pretty(&stats)?);
            return Ok(());
        }
        #[cfg(not(feature = "media-cuda"))]
        {
            let _ = device;
            return Err(
                "hw-filter requires cargo build --release --features media-cuda (CUDA NVDEC/NVENC)"
                    .into(),
            );
        }
    }
    if command == "decode" && paths.len() == 1 {
        if device_set
            && (crop.is_some()
                || scale.is_some()
                || transpose.is_some()
                || rotate.is_some()
                || pad.is_some()
                || burn_subs.is_some()
                || pix_fmt.is_some()
                || horizontal_flip
                || vertical_flip
                || from.is_some())
        {
            return Err(
                "decode --device does not take --crop/--hflip/--vflip/--transpose/--rotate/--pad/--scale/--subs/--pix-fmt/--from yet".into(),
            );
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("decode interval requires both --from and --to".into()),
        };
        let stats = if device_set {
            #[cfg(feature = "media-cuda")]
            {
                fvid_media::decode_video_cuda(&paths[0], device)?
            }
            #[cfg(not(feature = "media-cuda"))]
            {
                let _ = device;
                return Err("decode --device requires cargo build --features media-cuda".into());
            }
        } else {
            fvid_media::decode_video_transformed(
                &paths[0],
                fvid_media::DecodeTransform {
                    crop,
                    vertical_flip,
                    horizontal_flip,
                    scale,
                    epx: epx.clone(),
                    transpose,
                    rotate,
                    pad,
                    burn_subs: burn_subs.clone(),
                    overlay: overlay.map(|path| fvid_media::OverlaySpec {
                        path,
                        x: overlay_x,
                        y: overlay_y,
                    }),
                    yadif: yadif.clone(),
                    bwdif: bwdif.clone(),
                    w3fdif: w3fdif.clone(),
                    tblend: tblend.clone(),
                    tmix: tmix.clone(),
                    hqdn3d: hqdn3d.clone(),
                    gblur: gblur.clone(),
                    eq: eq.clone(),
                    unsharp: unsharp.clone(),
                    hue: hue.clone(),
                    avgblur: avgblur.clone(),
                    boxblur: boxblur.clone(),
                    negate: negate.clone(),
                    edgedetect: edgedetect.clone(),
                    sobel: sobel.clone(),
                    prewitt: prewitt.clone(),
                    roberts: roberts.clone(),
                    kirsch: kirsch.clone(),
                    scharr: scharr.clone(),
                    atadenoise: atadenoise.clone(),
                    owdenoise: owdenoise.clone(),
                    vaguedenoiser: vaguedenoiser.clone(),
                    nlmeans: nlmeans.clone(),
                    bm3d: bm3d.clone(),
                    dctdnoiz: dctdnoiz.clone(),
                    fftdnoiz: fftdnoiz.clone(),
                    smartblur: smartblur.clone(),
                    sab: sab.clone(),
                    bilateral: bilateral.clone(),
                    cas: cas.clone(),
                    vignette: vignette.clone(),
                    curves: curves.clone(),
                    colorbalance: colorbalance.clone(),
                    colorlevels: colorlevels.clone(),
                    colorchannelmixer: colorchannelmixer.clone(),
                    deflicker: deflicker.clone(),
                    photosensitivity: photosensitivity.clone(),
                    monochrome: monochrome.clone(),
                    grayworld: grayworld.clone(),
                    drawbox: drawbox.clone(),
                    drawgrid: drawgrid.clone(),
                    lagfun: lagfun.clone(),
                    amplify: amplify.clone(),
                    bitplanenoise: bitplanenoise.clone(),
                    deband: deband.clone(),
                    gradfun: gradfun.clone(),
                    lenscorrection: lenscorrection.clone(),
                    pixelize: pixelize.clone(),
                    removegrain: removegrain.clone(),
                    yaepblur: yaepblur.clone(),
                    vibrance: vibrance.clone(),
                    dilation: dilation.clone(),
                    erosion: erosion.clone(),
                    colorize: colorize.clone(),
                    exposure: exposure.clone(),
                    chromashift: chromashift.clone(),
                    colorcontrast: colorcontrast.clone(),
                    colorcorrect: colorcorrect.clone(),
                    histeq: histeq.clone(),
                    shuffleplanes: shuffleplanes.clone(),
                    lutyuv: lutyuv.clone(),
                    colorhold: colorhold.clone(),
                    fade: fade.clone(),
                    perspective: perspective.clone(),
                    lumakey: lumakey.clone(),
                    chromakey: chromakey.clone(),
                    colorkey: colorkey.clone(),
                    despill: despill.clone(),
                    selectivecolor: selectivecolor.clone(),
                    stereo3d: stereo3d.clone(),
                    field: field.clone(),
                    hqx: hqx.clone(),
                    xbr: xbr.clone(),
                    il: il.clone(),
                    super2xsai: super2xsai.clone(),
                    kerndeint: kerndeint.clone(),
                    phase: phase.clone(),
                    estdif: estdif.clone(),
                    tinterlace: tinterlace.clone(),
                    separatefields: separatefields.clone(),
                    weave: weave.clone(),
                    doubleweave: doubleweave.clone(),
                    framepack: framepack.clone(),
                    telecine: telecine.clone(),
                    pullup: pullup.clone(),
                    decimate: decimate.clone(),
                    mpdecimate: mpdecimate.clone(),
                    framestep: framestep.clone(),
                    tile: tile.clone(),
                    untile: untile.clone(),
                    shuffleframes: shuffleframes.clone(),
                    reverse: reverse.clone(),
                    r#loop: vloop.clone(),
                    thumbnail: thumbnail.clone(),
                    freezedetect: freezedetect.clone(),
                    pseudocolor: pseudocolor.clone(),
                    minterpolate: minterpolate.clone(),
                    fps: fps.clone(),
                    colorspace: colorspace.clone(),
                    zscale: zscale.clone(),
                    tonemap: tonemap.clone(),
                    pix_fmt: pix_fmt.clone(),
                    interval,
                },
            )?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if (command == "crop-lossless" || command == "transcode-lossless" || command == "transcode")
        && paths.len() == 2
    {
        if command == "crop-lossless" && crop.is_none() {
            return Err("--crop required".into());
        }
        if command == "crop-lossless" && scale.is_some() {
            return Err("crop-lossless does not take --scale; use transcode-lossless".into());
        }
        if command == "crop-lossless" && transpose.is_some() {
            return Err("crop-lossless does not take --transpose; use transcode-lossless".into());
        }
        if command == "crop-lossless" && rotate.is_some() {
            return Err("crop-lossless does not take --rotate; use transcode-lossless".into());
        }
        if command == "crop-lossless" && pad.is_some() {
            return Err("crop-lossless does not take --pad; use transcode-lossless".into());
        }
        if command == "crop-lossless" && pix_fmt.is_some() {
            return Err("crop-lossless does not take --pix-fmt; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorspace.is_some() {
            return Err("crop-lossless does not take --colorspace; use transcode-lossless".into());
        }
        if command == "crop-lossless" && zscale.is_some() {
            return Err("crop-lossless does not take --zscale; use transcode-lossless".into());
        }
        if command == "crop-lossless" && tonemap.is_some() {
            return Err("crop-lossless does not take --tonemap; use transcode-lossless".into());
        }
        if command == "crop-lossless" && yadif.is_some() {
            return Err("crop-lossless does not take --yadif; use transcode-lossless".into());
        }
        if command == "crop-lossless" && bwdif.is_some() {
            return Err("crop-lossless does not take --bwdif; use transcode-lossless".into());
        }
        if command == "crop-lossless" && w3fdif.is_some() {
            return Err("crop-lossless does not take --w3fdif; use transcode-lossless".into());
        }
        if command == "crop-lossless" && tblend.is_some() {
            return Err("crop-lossless does not take --tblend; use transcode-lossless".into());
        }
        if command == "crop-lossless" && tmix.is_some() {
            return Err("crop-lossless does not take --tmix; use transcode-lossless".into());
        }
        if command == "crop-lossless" && hqdn3d.is_some() {
            return Err("crop-lossless does not take --hqdn3d; use transcode-lossless".into());
        }
        if command == "crop-lossless" && fps.is_some() {
            return Err("crop-lossless does not take --fps; use transcode-lossless".into());
        }
        if command == "crop-lossless" && gblur.is_some() {
            return Err("crop-lossless does not take --gblur; use transcode-lossless".into());
        }
        if command == "crop-lossless" && eq.is_some() {
            return Err("crop-lossless does not take --eq; use transcode-lossless".into());
        }
        if command == "crop-lossless" && unsharp.is_some() {
            return Err("crop-lossless does not take --unsharp; use transcode-lossless".into());
        }
        if command == "crop-lossless" && hue.is_some() {
            return Err("crop-lossless does not take --hue; use transcode-lossless".into());
        }
        if command == "crop-lossless" && avgblur.is_some() {
            return Err("crop-lossless does not take --avgblur; use transcode-lossless".into());
        }
        if command == "crop-lossless" && boxblur.is_some() {
            return Err("crop-lossless does not take --boxblur; use transcode-lossless".into());
        }
        if command == "crop-lossless" && negate.is_some() {
            return Err("crop-lossless does not take --negate; use transcode-lossless".into());
        }
        if command == "crop-lossless" && edgedetect.is_some() {
            return Err("crop-lossless does not take --edgedetect; use transcode-lossless".into());
        }
        if command == "crop-lossless" && sobel.is_some() {
            return Err("crop-lossless does not take --sobel; use transcode-lossless".into());
        }
        if command == "crop-lossless" && prewitt.is_some() {
            return Err("crop-lossless does not take --prewitt; use transcode-lossless".into());
        }
        if command == "crop-lossless" && roberts.is_some() {
            return Err("crop-lossless does not take --roberts; use transcode-lossless".into());
        }
        if command == "crop-lossless" && kirsch.is_some() {
            return Err("crop-lossless does not take --kirsch; use transcode-lossless".into());
        }
        if command == "crop-lossless" && scharr.is_some() {
            return Err("crop-lossless does not take --scharr; use transcode-lossless".into());
        }
        if command == "crop-lossless" && atadenoise.is_some() {
            return Err("crop-lossless does not take --atadenoise; use transcode-lossless".into());
        }
        if command == "crop-lossless" && owdenoise.is_some() {
            return Err("crop-lossless does not take --owdenoise; use transcode-lossless".into());
        }
        if command == "crop-lossless" && vaguedenoiser.is_some() {
            return Err(
                "crop-lossless does not take --vaguedenoiser; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && nlmeans.is_some() {
            return Err("crop-lossless does not take --nlmeans; use transcode-lossless".into());
        }
        if command == "crop-lossless" && bm3d.is_some() {
            return Err("crop-lossless does not take --bm3d; use transcode-lossless".into());
        }
        if command == "crop-lossless" && dctdnoiz.is_some() {
            return Err("crop-lossless does not take --dctdnoiz; use transcode-lossless".into());
        }
        if command == "crop-lossless" && fftdnoiz.is_some() {
            return Err("crop-lossless does not take --fftdnoiz; use transcode-lossless".into());
        }
        if command == "crop-lossless" && smartblur.is_some() {
            return Err("crop-lossless does not take --smartblur; use transcode-lossless".into());
        }
        if command == "crop-lossless" && sab.is_some() {
            return Err("crop-lossless does not take --sab; use transcode-lossless".into());
        }
        if command == "crop-lossless" && bilateral.is_some() {
            return Err("crop-lossless does not take --bilateral; use transcode-lossless".into());
        }
        if command == "crop-lossless" && cas.is_some() {
            return Err("crop-lossless does not take --cas; use transcode-lossless".into());
        }
        if command == "crop-lossless" && epx.is_some() {
            return Err("crop-lossless does not take --epx; use transcode-lossless".into());
        }
        if command == "crop-lossless" && vignette.is_some() {
            return Err("crop-lossless does not take --vignette; use transcode-lossless".into());
        }
        if command == "crop-lossless" && curves.is_some() {
            return Err("crop-lossless does not take --curves; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorbalance.is_some() {
            return Err(
                "crop-lossless does not take --colorbalance; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && colorlevels.is_some() {
            return Err("crop-lossless does not take --colorlevels; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorchannelmixer.is_some() {
            return Err(
                "crop-lossless does not take --colorchannelmixer; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && deflicker.is_some() {
            return Err("crop-lossless does not take --deflicker; use transcode-lossless".into());
        }
        if command == "crop-lossless" && photosensitivity.is_some() {
            return Err(
                "crop-lossless does not take --photosensitivity; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && monochrome.is_some() {
            return Err("crop-lossless does not take --monochrome; use transcode-lossless".into());
        }
        if command == "crop-lossless" && grayworld.is_some() {
            return Err("crop-lossless does not take --grayworld; use transcode-lossless".into());
        }
        if command == "crop-lossless" && drawbox.is_some() {
            return Err("crop-lossless does not take --drawbox; use transcode-lossless".into());
        }
        if command == "crop-lossless" && drawgrid.is_some() {
            return Err("crop-lossless does not take --drawgrid; use transcode-lossless".into());
        }
        if command == "crop-lossless" && lagfun.is_some() {
            return Err("crop-lossless does not take --lagfun; use transcode-lossless".into());
        }
        if command == "crop-lossless" && amplify.is_some() {
            return Err("crop-lossless does not take --amplify; use transcode-lossless".into());
        }
        if command == "crop-lossless" && bitplanenoise.is_some() {
            return Err(
                "crop-lossless does not take --bitplanenoise; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && deband.is_some() {
            return Err("crop-lossless does not take --deband; use transcode-lossless".into());
        }
        if command == "crop-lossless" && gradfun.is_some() {
            return Err("crop-lossless does not take --gradfun; use transcode-lossless".into());
        }
        if command == "crop-lossless" && lenscorrection.is_some() {
            return Err(
                "crop-lossless does not take --lenscorrection; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && pixelize.is_some() {
            return Err("crop-lossless does not take --pixelize; use transcode-lossless".into());
        }
        if command == "crop-lossless" && removegrain.is_some() {
            return Err("crop-lossless does not take --removegrain; use transcode-lossless".into());
        }
        if command == "crop-lossless" && yaepblur.is_some() {
            return Err("crop-lossless does not take --yaepblur; use transcode-lossless".into());
        }
        if command == "crop-lossless" && vibrance.is_some() {
            return Err("crop-lossless does not take --vibrance; use transcode-lossless".into());
        }
        if command == "crop-lossless" && dilation.is_some() {
            return Err("crop-lossless does not take --dilation; use transcode-lossless".into());
        }
        if command == "crop-lossless" && erosion.is_some() {
            return Err("crop-lossless does not take --erosion; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorize.is_some() {
            return Err("crop-lossless does not take --colorize; use transcode-lossless".into());
        }
        if command == "crop-lossless" && exposure.is_some() {
            return Err("crop-lossless does not take --exposure; use transcode-lossless".into());
        }
        if command == "crop-lossless" && chromashift.is_some() {
            return Err("crop-lossless does not take --chromashift; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorcontrast.is_some() {
            return Err(
                "crop-lossless does not take --colorcontrast; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && colorcorrect.is_some() {
            return Err(
                "crop-lossless does not take --colorcorrect; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && histeq.is_some() {
            return Err("crop-lossless does not take --histeq; use transcode-lossless".into());
        }
        if command == "crop-lossless" && shuffleplanes.is_some() {
            return Err(
                "crop-lossless does not take --shuffleplanes; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && lutyuv.is_some() {
            return Err("crop-lossless does not take --lutyuv; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorhold.is_some() {
            return Err("crop-lossless does not take --colorhold; use transcode-lossless".into());
        }
        if command == "crop-lossless" && fade.is_some() {
            return Err("crop-lossless does not take --fade; use transcode-lossless".into());
        }
        if command == "crop-lossless" && perspective.is_some() {
            return Err("crop-lossless does not take --perspective; use transcode-lossless".into());
        }
        if command == "crop-lossless" && lumakey.is_some() {
            return Err("crop-lossless does not take --lumakey; use transcode-lossless".into());
        }
        if command == "crop-lossless" && chromakey.is_some() {
            return Err("crop-lossless does not take --chromakey; use transcode-lossless".into());
        }
        if command == "crop-lossless" && colorkey.is_some() {
            return Err("crop-lossless does not take --colorkey; use transcode-lossless".into());
        }
        if command == "crop-lossless" && despill.is_some() {
            return Err("crop-lossless does not take --despill; use transcode-lossless".into());
        }
        if command == "crop-lossless" && selectivecolor.is_some() {
            return Err(
                "crop-lossless does not take --selectivecolor; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && stereo3d.is_some() {
            return Err("crop-lossless does not take --stereo3d; use transcode-lossless".into());
        }
        if command == "crop-lossless" && field.is_some() {
            return Err("crop-lossless does not take --field; use transcode-lossless".into());
        }
        if command == "crop-lossless" && hqx.is_some() {
            return Err("crop-lossless does not take --hqx; use transcode-lossless".into());
        }
        if command == "crop-lossless" && xbr.is_some() {
            return Err("crop-lossless does not take --xbr; use transcode-lossless".into());
        }
        if command == "crop-lossless" && il.is_some() {
            return Err("crop-lossless does not take --il; use transcode-lossless".into());
        }
        if command == "crop-lossless" && super2xsai.is_some() {
            return Err("crop-lossless does not take --super2xsai; use transcode-lossless".into());
        }
        if command == "crop-lossless" && kerndeint.is_some() {
            return Err("crop-lossless does not take --kerndeint; use transcode-lossless".into());
        }
        if command == "crop-lossless" && phase.is_some() {
            return Err("crop-lossless does not take --phase; use transcode-lossless".into());
        }
        if command == "crop-lossless" && estdif.is_some() {
            return Err("crop-lossless does not take --estdif; use transcode-lossless".into());
        }
        if command == "crop-lossless" && tinterlace.is_some() {
            return Err("crop-lossless does not take --tinterlace; use transcode-lossless".into());
        }
        if command == "crop-lossless" && separatefields.is_some() {
            return Err("crop-lossless does not take --separatefields; use transcode-lossless".into());
        }
        if command == "crop-lossless" && weave.is_some() {
            return Err("crop-lossless does not take --weave; use transcode-lossless".into());
        }
        if command == "crop-lossless" && doubleweave.is_some() {
            return Err("crop-lossless does not take --doubleweave; use transcode-lossless".into());
        }
        if command == "crop-lossless" && framepack.is_some() {
            return Err("crop-lossless does not take --framepack; use transcode-lossless".into());
        }
        if command == "crop-lossless" && telecine.is_some() {
            return Err("crop-lossless does not take --telecine; use transcode-lossless".into());
        }
        if command == "crop-lossless" && pullup.is_some() {
            return Err("crop-lossless does not take --pullup; use transcode-lossless".into());
        }
        if command == "crop-lossless" && decimate.is_some() {
            return Err("crop-lossless does not take --decimate; use transcode-lossless".into());
        }
        if command == "crop-lossless" && mpdecimate.is_some() {
            return Err("crop-lossless does not take --mpdecimate; use transcode-lossless".into());
        }
        if command == "crop-lossless" && framestep.is_some() {
            return Err("crop-lossless does not take --framestep; use transcode-lossless".into());
        }
        if command == "crop-lossless" && tile.is_some() {
            return Err("crop-lossless does not take --tile; use transcode-lossless".into());
        }
        if command == "crop-lossless" && untile.is_some() {
            return Err("crop-lossless does not take --untile; use transcode-lossless".into());
        }
        if command == "crop-lossless" && shuffleframes.is_some() {
            return Err("crop-lossless does not take --shuffleframes; use transcode-lossless".into());
        }
        if command == "crop-lossless" && reverse.is_some() {
            return Err("crop-lossless does not take --reverse; use transcode-lossless".into());
        }
        if command == "crop-lossless" && vloop.is_some() {
            return Err("crop-lossless does not take --loop; use transcode-lossless".into());
        }
        if command == "crop-lossless" && thumbnail.is_some() {
            return Err("crop-lossless does not take --thumbnail; use transcode-lossless".into());
        }
        if command == "crop-lossless" && freezedetect.is_some() {
            return Err("crop-lossless does not take --freezedetect; use transcode-lossless".into());
        }
        if command == "crop-lossless" && setpts.is_some() {
            return Err("crop-lossless does not take --setpts; use transcode-lossless".into());
        }
        if command == "crop-lossless" && select.is_some() {
            return Err("crop-lossless does not take --select; use transcode-lossless".into());
        }
        if command == "crop-lossless" && pseudocolor.is_some() {
            return Err("crop-lossless does not take --pseudocolor; use transcode-lossless".into());
        }
        if command == "crop-lossless" && minterpolate.is_some() {
            return Err(
                "crop-lossless does not take --minterpolate; use transcode-lossless".into(),
            );
        }
        if command == "crop-lossless" && overlay.is_some() {
            return Err(
                "crop-lossless does not take --overlay; use overlay or transcode-lossless".into(),
            );
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("lossless interval requires both --from and --to".into()),
        };
        let transform = fvid_media::LosslessTransform {
            crop,
            vertical_flip,
            horizontal_flip,
            scale,
            epx,
            transpose,
            rotate,
            pad,
            burn_subs: None,
            overlay: overlay.map(|path| fvid_media::OverlaySpec {
                path,
                x: overlay_x,
                y: overlay_y,
            }),
            xfade: None,
            yadif,
            bwdif,
            w3fdif,
            tblend,
            tmix,
            hqdn3d,
            gblur,
            eq,
            unsharp,
            hue,
            avgblur,
            boxblur,
            negate,
            edgedetect,
            sobel,
            prewitt,
            roberts,
            kirsch,
            scharr,
            atadenoise,
            owdenoise,
            vaguedenoiser,
            nlmeans,
            bm3d,
            dctdnoiz,
            fftdnoiz,
            smartblur,
            sab,
            bilateral,
            cas,
            vignette,
            curves,
            colorbalance,
            colorlevels,
            colorchannelmixer,
            deflicker,
            photosensitivity,
            monochrome,
            grayworld,
            drawbox,
            drawgrid,
            lagfun,
            amplify,
            bitplanenoise,
            deband,
            gradfun,
            lenscorrection,
            pixelize,
            removegrain,
            yaepblur,
            vibrance,
            dilation,
            erosion,
            colorize,
            exposure,
            chromashift,
            colorcontrast,
            colorcorrect,
            histeq,
            shuffleplanes,
            lutyuv,
            colorhold,
            fade,
            perspective,
            lumakey,
            chromakey,
            colorkey,
            despill,
            selectivecolor,
            stereo3d,
            field,
            hqx,
            xbr,
            il,
            super2xsai,
            kerndeint,
            phase,
            estdif,
            tinterlace,
            separatefields,
            weave,
            doubleweave,
            framepack,
            telecine,
            pullup,
            decimate,
            mpdecimate,
            framestep,
            tile,
            untile,
            shuffleframes,
            reverse,
            r#loop: vloop,
            thumbnail,
            freezedetect,
            pseudocolor,
            minterpolate,
            fps,
            colorspace,
            zscale,
            tonemap,
            pix_fmt,
            interval,
            seek,
        };
        let stats = if command == "transcode" {
            fvid_media::transcode(
                &paths[0],
                &paths[1],
                transform,
                &options,
                &fvid_media::EncoderSettings {
                    name: encoder
                        .ok_or("transcode requires --encoder; quality is defined by its options")?,
                    options: encoder_options,
                },
            )?
        } else {
            fvid_media::transcode_lossless(&paths[0], &paths[1], transform, &options)?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if crop.is_some()
        || scale.is_some()
        || transpose.is_some()
        || rotate.is_some()
        || pad.is_some()
        || pix_fmt.is_some()
        || colorspace.is_some()
        || zscale.is_some()
        || tonemap.is_some()
        || yadif.is_some()
        || bwdif.is_some()
        || w3fdif.is_some()
        || tblend.is_some()
        || tmix.is_some()
        || hqdn3d.is_some()
        || gblur.is_some()
        || eq.is_some()
        || unsharp.is_some()
        || hue.is_some()
        || avgblur.is_some()
        || boxblur.is_some()
        || negate.is_some()
        || edgedetect.is_some()
        || sobel.is_some()
        || prewitt.is_some()
        || roberts.is_some()
        || kirsch.is_some()
        || scharr.is_some()
        || atadenoise.is_some()
        || owdenoise.is_some()
        || vaguedenoiser.is_some()
        || nlmeans.is_some()
        || bm3d.is_some()
        || dctdnoiz.is_some()
        || fftdnoiz.is_some()
        || smartblur.is_some()
        || sab.is_some()
        || bilateral.is_some()
        || cas.is_some()
        || epx.is_some()
        || vignette.is_some()
        || curves.is_some()
        || colorbalance.is_some()
        || colorlevels.is_some()
        || colorchannelmixer.is_some()
        || deflicker.is_some()
        || photosensitivity.is_some()
        || monochrome.is_some()
        || grayworld.is_some()
        || drawbox.is_some()
        || drawgrid.is_some()
        || lagfun.is_some()
        || amplify.is_some()
        || bitplanenoise.is_some()
        || deband.is_some()
        || gradfun.is_some()
        || lenscorrection.is_some()
        || pixelize.is_some()
        || removegrain.is_some()
        || yaepblur.is_some()
        || vibrance.is_some()
        || dilation.is_some()
        || erosion.is_some()
        || colorize.is_some()
        || exposure.is_some()
        || chromashift.is_some()
        || colorcontrast.is_some()
        || colorcorrect.is_some()
        || histeq.is_some()
        || shuffleplanes.is_some()
        || lutyuv.is_some()
        || colorhold.is_some()
        || fade.is_some()
        || perspective.is_some()
        || lumakey.is_some()
        || chromakey.is_some()
        || colorkey.is_some()
        || despill.is_some()
        || selectivecolor.is_some()
        || stereo3d.is_some()
        || field.is_some()
        || hqx.is_some()
        || xbr.is_some()
        || il.is_some()
        || super2xsai.is_some()
        || kerndeint.is_some()
        || phase.is_some()
        || estdif.is_some()
        || tinterlace.is_some()
        || separatefields.is_some()
        || weave.is_some()
        || doubleweave.is_some()
        || framepack.is_some()
        || telecine.is_some()
        || pullup.is_some()
        || decimate.is_some()
        || mpdecimate.is_some()
        || framestep.is_some()
        || tile.is_some()
        || untile.is_some()
        || shuffleframes.is_some()
        || reverse.is_some()
        || vloop.is_some()
        || thumbnail.is_some()
        || freezedetect.is_some()
        || setpts.is_some()
        || select.is_some()
        || pseudocolor.is_some()
        || minterpolate.is_some()
        || fps.is_some()
        || vertical_flip
        || horizontal_flip
        || seek
    {
        return Err(
            "--crop/--hflip/--vflip/--transpose/--rotate/--pad/--scale/--epx/--pix-fmt/--colorspace/--zscale/--tonemap/--yadif/--bwdif/--w3fdif/--tblend/--tmix/--hqdn3d/--gblur/--eq/--unsharp/--hue/--avgblur/--boxblur/--negate/--edgedetect/--sobel/--prewitt/--roberts/--kirsch/--scharr/--atadenoise/--owdenoise/--vaguedenoiser/--nlmeans/--bm3d/--dctdnoiz/--fftdnoiz/--smartblur/--sab/--bilateral/--cas/--vignette/--curves/--colorbalance/--colorlevels/--colorchannelmixer/--deflicker/--photosensitivity/--monochrome/--grayworld/--drawbox/--drawgrid/--lagfun/--amplify/--bitplanenoise/--deband/--gradfun/--lenscorrection/--pixelize/--removegrain/--yaepblur/--vibrance/--dilation/--erosion/--colorize/--exposure/--chromashift/--colorcontrast/--colorcorrect/--histeq/--shuffleplanes/--lutyuv/--colorhold/--fade/--perspective/--lumakey/--chromakey/--colorkey/--despill/--selectivecolor/--stereo3d/--field/--hqx/--xbr/--il/--super2xsai/--kerndeint/--phase/--estdif/--tinterlace/--separatefields/--weave/--doubleweave/--framepack/--telecine/--pullup/--decimate/--mpdecimate/--framestep/--shuffleframes/--reverse/--loop/--thumbnail/--freezedetect/--setpts/--pseudocolor/--minterpolate/--fps/--seek require decode, crop-lossless, transcode-lossless, transcode, or hw-filter"
                .into(),
        );
    }
    if command == "decode-audio" && paths.len() == 2 {
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("decode-audio interval requires both --from and --to".into()),
        };
        let stats = fvid_media::decode_audio_transformed(
            &paths[0],
            &paths[1],
            fvid_media::AudioDecodeTransform {
                interval,
                sample_rate,
                channels,
                volume,
            },
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if sample_rate.is_some() || channels.is_some() || volume.is_some() {
        return Err("--rate/--channels/--volume require decode-audio".into());
    }
    if command == "loudness" && paths.len() == 1 {
        let stats = fvid_media::measure_loudness(&paths[0], &options)?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "loudnorm" && paths.len() == 2 {
        if crop.is_some()
            || scale.is_some()
            || transpose.is_some()
            || rotate.is_some()
            || pad.is_some()
            || burn_subs.is_some()
            || overlay.is_some()
            || colorspace.is_some()
            || zscale.is_some()
            || tonemap.is_some()
            || yadif.is_some()
            || bwdif.is_some()
            || w3fdif.is_some()
            || tblend.is_some()
            || tmix.is_some()
            || hqdn3d.is_some()
            || gblur.is_some()
            || eq.is_some()
            || unsharp.is_some()
            || hue.is_some()
            || avgblur.is_some()
            || boxblur.is_some()
            || negate.is_some()
            || edgedetect.is_some()
            || sobel.is_some()
            || prewitt.is_some()
            || roberts.is_some()
            || kirsch.is_some()
            || scharr.is_some()
            || atadenoise.is_some()
            || owdenoise.is_some()
            || vaguedenoiser.is_some()
            || nlmeans.is_some()
            || bm3d.is_some()
            || dctdnoiz.is_some()
            || fftdnoiz.is_some()
            || smartblur.is_some()
            || sab.is_some()
            || bilateral.is_some()
            || cas.is_some()
            || epx.is_some()
            || vignette.is_some()
            || curves.is_some()
            || colorbalance.is_some()
            || colorlevels.is_some()
            || colorchannelmixer.is_some()
            || deflicker.is_some()
            || photosensitivity.is_some()
            || monochrome.is_some()
            || grayworld.is_some()
            || drawbox.is_some()
            || drawgrid.is_some()
            || lagfun.is_some()
            || amplify.is_some()
            || bitplanenoise.is_some()
            || deband.is_some()
            || gradfun.is_some()
            || lenscorrection.is_some()
            || pixelize.is_some()
            || removegrain.is_some()
            || yaepblur.is_some()
            || vibrance.is_some()
            || dilation.is_some()
            || erosion.is_some()
            || colorize.is_some()
            || exposure.is_some()
            || chromashift.is_some()
            || colorcontrast.is_some()
            || colorcorrect.is_some()
            || histeq.is_some()
            || shuffleplanes.is_some()
            || lutyuv.is_some()
            || colorhold.is_some()
            || fade.is_some()
            || perspective.is_some()
            || lumakey.is_some()
            || chromakey.is_some()
            || colorkey.is_some()
            || despill.is_some()
            || selectivecolor.is_some()
            || stereo3d.is_some()
            || field.is_some()
            || hqx.is_some()
            || xbr.is_some()
            || il.is_some()
            || super2xsai.is_some()
            || kerndeint.is_some()
            || phase.is_some()
            || estdif.is_some()
            || tinterlace.is_some()
            || pseudocolor.is_some()
            || minterpolate.is_some()
            || fps.is_some()
            || pix_fmt.is_some()
            || from.is_some()
            || to.is_some()
            || volume.is_some()
            || sample_rate.is_some()
            || channels.is_some()
        {
            return Err(
                "loudnorm takes INPUT OUTPUT [--loudnorm-args ARGS] [--dual-pass] [--streams INDEX] only".into(),
            );
        }
        let stats = if loudnorm_dual {
            fvid_media::apply_loudnorm_dual(
                &paths[0],
                &paths[1],
                loudnorm_args.as_deref(),
                &options,
            )?
        } else {
            fvid_media::apply_loudnorm(&paths[0], &paths[1], loudnorm_args.as_deref(), &options)?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if loudnorm_args.is_some() || loudnorm_dual {
        return Err("--loudnorm-args/--dual-pass require loudnorm".into());
    }
    if command == "convert-subtitles" && paths.len() == 2 {
        let stats = fvid_media::convert_subtitles(
            &paths[0],
            &paths[1],
            &fvid_media::SubtitleConvertOptions {
                streams: options.streams.clone(),
                codec: subtitle_codec,
            },
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "burn-subtitles" && paths.len() == 2 {
        let subs = burn_subs
            .as_deref()
            .ok_or("burn-subtitles requires --subs FILE")?;
        let stats = fvid_media::burn_subtitles(&paths[0], &paths[1], subs, &options)?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "xfade" && paths.len() == 3 {
        let transition = xfade_transition.as_deref().unwrap_or("fade");
        let duration = xfade_duration.ok_or("--xfade-duration required")?;
        let offset = xfade_offset.unwrap_or(0);
        let stats = fvid_media::xfade_video(
            &paths[0], &paths[1], &paths[2], transition, duration, offset, &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "xfade" {
        return Err("xfade requires MAIN OTHER OUTPUT.mkv --xfade-duration SECONDS".into());
    }

    if xfade_transition.is_some() || xfade_duration.is_some() || xfade_offset.is_some() {
        return Err("--transition/--xfade-duration/--xfade-offset require xfade".into());
    }
    if command == "overlay" && paths.len() == 3 {
        let stats = fvid_media::overlay_video(
            &paths[0], &paths[1], &paths[2], overlay_x, overlay_y, &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if burn_subs.is_some() {
        return Err("--subs requires decode or burn-subtitles".into());
    }
    if overlay.is_some() {
        return Err("--overlay requires decode, overlay, transcode-lossless, or transcode".into());
    }
    if command == "trim-pcm" && paths.len() == 2 {
        let stats = fvid_media::trim_pcm(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "mix-audio" && paths.len() >= 3 {
        let stats = fvid_media::mix_audio(
            &paths[1..],
            &paths[0],
            &fvid_media::MixAudioOptions {
                duration: mix_duration,
                normalize: mix_normalize,
                weights: mix_weights,
            },
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "merge-audio" && paths.len() == 3 {
        let stats = fvid_media::merge_audio(&paths[1..], &paths[0])?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    let stats = match command.as_str() {
        "remux" if paths.len() == 2 && from.is_none() && to.is_none() => {
            fvid_media::remux(&paths[0], &paths[1], &options)?
        }
        "trim" if paths.len() == 2 => fvid_media::trim(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?,
        "concat" if paths.len() >= 3 && from.is_none() && to.is_none() => {
            fvid_media::concat(&paths[1..], &paths[0], &options)?
        }
        _ => return Err(help.into()),
    };
    emit_json(quiet, serde_json::to_string_pretty(&stats)?);
    Ok(())
}

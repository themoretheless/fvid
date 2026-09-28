use fvid::{Crop, ExecutionOptions, Transform};
mod media_cli;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};

const HELP: &str = "fvid INPUT.y4m OUTPUT.y4m [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--memory-mib N] [--backend cpu|auto|metal|vulkan|dx12|gl|cuda] [--device N]\nUse - for stdin/stdout. Output files must not exist. Only progressive 8-bit planar YUV 420/422/444 is supported.\nCrop is applied before reflections. Default backend: cpu. Frame/staging-buffer budget: 256 MiB. Use --list-devices to inspect GPU adapters. --then starts the next resident GPU stage (requires an explicit GPU backend); coordinates are relative to the previous stage output.\nOpen the FVid player: fvid play [OPTIONS] [INPUT...] where INPUT is Y4M, MP4/AVC or WebM/Matroska VP9/AV1 video, or an .m3u/.pls list of them; several inputs queue up (requires --features player). Options apply to the first input: --start-time and --stop-time take seconds or HH:MM:SS(.mmm), --rate a speed from 0.25 to 4. The rest hold for the whole session: --start-paused leaves each item waiting for the play key, --no-audio keeps it silent, and --audio-track N and --subtitle-track N start every item on the Nth track it lists, counted from 1. --volume N sets the level in VLC's 0-200 percent, --mute starts silent, --loop and --repeat decide what follows the last item, --audio-delay and --subtitle-delay shift those clocks in milliseconds, and --sub-file FILE reads subtitles from the named file before the one guessed from the video's name. --zoom F magnifies the picture by F, within the 0.25 to 10 VLC allows, and --crop RATIO cuts it to a shape VLC lists (16:9, or 185:100 for its cinema ratios; none leaves it whole), and --aspect RATIO forces the shape that picture is drawn at (default keeps the file's own shape, fill takes the window's). --brightness, --gamma, --saturation, --contrast and --hue start the session with VLC's picture settings on, at the given values and VLC's slider bounds (brightness and contrast 0 to 2, saturation 0 to 3, gamma 0.01 to 10, hue -180 to 180 degrees). --log CURVE reads the coded values as camera log rather than the curve the file states (slog1, slog2, slog3, clog, clog2, clog3, vlog, logc, logc4), --gamut NAME reads those bytes in the working gamut it names rather than the one the file or the curve gives (rec709, rec2020, dci-p3, s-gamut3.cine, awg4 and the rest by their labels), --display PANEL grades for the screen it names (sdr[:nits] for a desktop panel, whose white is 100 cd/m² unless told, pq[:nits] or hlg[:nits] for BT.2100 codes over BT.2020 at 1 000 by default, hdr being another spelling of pq), --tonemap CURVE compresses highlights with one of linear, gamma, clip, reinhard, hable or mobius, and --lut FILE applies a .cube, .3dl, .spi1d or .spi3d look after them, --grid N bakes the conversion on the cube edge it names (2 to 128 nodes, 33 by default), and --interp MODE joins the nodes of that cube and of the look's grid by nearest, trilinear or tetrahedral (tetrahedral by default); these hold for the session and each item is graded from the colour signal it states for itself, while BT.2100 material is compressed for the screen even when none of them is named. Space pauses, F toggles fullscreen, arrows seek (Shift 10 s, Alt 1 min, Ctrl/Cmd 5 min), Ctrl/Cmd+Up/Down or the wheel set volume, M mutes, = and - step the rate, \\ returns to 1x, E steps one frame (Shift back), T toggles subtitles, Ctrl/Cmd+T asks for a file to caption the item on screen with, G/H delay them, Alt+Up/Down and Alt+=/- move and size them, N/P walk the list, R cycles repeat, L marks the A-B loop points (press again to clear), J/K shift the audio delay, A (Shift+A back) walks the file's audio tracks, B (Shift+B back) its subtitle tracks, `[` and `]` its chapters, Shift+S writes the picture on screen out as a PNG beside the file, z and Z step it along VLC's zoom menu (drag a magnified picture to see the part outside the window), c and C walk VLC's crop shapes (the walk comes back to the whole picture), v and V walk VLC's aspect-ratio shapes (that walk returns the file's own shape), i opens the panel saying what the item is and closes it again, Ctrl/Cmd+E opens and closes VLC's dialog of picture settings (brightness, gamma, saturation, contrast and tone over the shown picture), Esc quits. A .srt/.vtt/.ass/.ssa/.smi file named after the video beside it is loaded automatically. Native codecs and audio; general AVC tools are still in development.";
struct Temporary(PathBuf);
fn process_selected<R: io::BufRead, W: Write>(
    reader: R,
    writer: W,
    transforms: &[Transform],
    memory: usize,
    options: ExecutionOptions,
) -> fvid::Result<fvid::Stats> {
    if transforms.len() == 1 {
        return fvid::process_with_options(reader, writer, transforms[0], memory, options);
    }
    #[cfg(any(feature = "gpu", feature = "cuda"))]
    {
        let (stats, transfers) =
            fvid::process_gpu_chain(reader, writer, transforms, memory, options)?;
        eprintln!(
            "uploads={} downloads={} upload_bytes={} download_bytes={} filter_passes={}",
            transfers.uploads,
            transfers.downloads,
            transfers.upload_bytes,
            transfers.download_bytes,
            transfers.filter_passes
        );
        Ok(stats)
    }
    #[cfg(not(any(feature = "gpu", feature = "cuda")))]
    Err(fvid::Error::Invalid(
        "--then requires a GPU-enabled build and an explicit GPU backend".into(),
    ))
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "mcp") {
        #[cfg(feature = "mcp")]
        return fvid::mcp::run(&args[1..]);
        #[cfg(not(feature = "mcp"))]
        return Err("MCP requires cargo build --release --features mcp".into());
    }
    if args.first().is_some_and(|a| a == "media") {
        return media_cli::run(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "play") {
        #[cfg(feature = "player")]
        {
            return fvid::player::run(args[1..].to_vec());
        }
        #[cfg(not(feature = "player"))]
        return Err(
            "player requires cargo build --features player; native playback supports Y4M, MP4/AVC and WebM/Matroska VP9/AV1 subsets".into(),
        );
    }
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--list-devices") {
        println!("backend=cpu device=0 name=CPU status=available");
        for entry in fvid::backend::devices() {
            if let Some(reason) = entry.unavailable_reason {
                println!(
                    "backend={} status=unavailable reason={:?}",
                    entry.backend, reason
                );
            }
            for d in entry.devices {
                println!(
                    "backend={} device={} name={:?} type={} status=available",
                    d.backend, d.ordinal, d.name, d.device_type
                );
            }
        }
        return Ok(());
    }
    if args.len() < 2 {
        return Err(HELP.into());
    }
    let mut transform = Transform::default();
    let mut transforms = Vec::new();
    let mut options = ExecutionOptions::default();
    let mut backend_explicit = false;
    let mut memory = 256usize * 1024 * 1024;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--then" => {
                if transforms.len() >= 255 {
                    return Err("at most 256 GPU stages are supported".into());
                }
                transforms.push(std::mem::take(&mut transform));
            }
            "--hflip" => transform.horizontal = true,
            "--vflip" => transform.vertical = true,
            "--crop" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing crop")?
                    .split(':')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                if values.len() != 4 {
                    return Err("crop must be X:Y:WIDTH:HEIGHT".into());
                }
                transform.crop = Some(Crop {
                    x: values[0],
                    y: values[1],
                    width: values[2],
                    height: values[3],
                });
            }
            "--backend" => {
                i += 1;
                options.backend = args.get(i).ok_or("missing backend")?.parse()?;
                backend_explicit = true;
            }
            "--device" => {
                i += 1;
                options.device = args.get(i).ok_or("missing device ordinal")?.parse()?;
            }
            "--memory-mib" => {
                i += 1;
                memory = args
                    .get(i)
                    .ok_or("missing memory budget")?
                    .parse::<usize>()?
                    .checked_mul(1024 * 1024)
                    .ok_or("memory budget overflow")?;
            }
            other => return Err(format!("unknown option: {other}").into()),
        }
        i += 1;
    }
    transforms.push(transform);
    let input: Box<dyn io::Read> = if args[0] == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(&args[0])?)
    };
    // Larger than 64 KiB reduces syscall chatter on 1080p Y4M; keep modest so
    // the working set stays cache-friendly versus multi-MiB buffers.
    const IO_BUF: usize = 256 * 1024;
    let input = BufReader::with_capacity(IO_BUF, input);
    let stats = if args[1] == "-" {
        process_selected(
            input,
            BufWriter::with_capacity(IO_BUF, io::stdout().lock()),
            &transforms,
            memory,
            options,
        )?
    } else {
        let output = Path::new(&args[1]);
        if output.symlink_metadata().is_ok() {
            return Err("output already exists".into());
        }
        let directory = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut staged = None;
        for index in 0..100 {
            let path = directory.join(format!(".fvid-{}-{index}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    staged = Some((Temporary(path), file));
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let (temp, file) = staged.ok_or("cannot create temporary output")?;
        let mut writer = BufWriter::with_capacity(IO_BUF, file);
        let stats = process_selected(input, &mut writer, &transforms, memory, options)?;
        writer.flush()?;
        // Prefer hard link; rename when hard links are unavailable (see publish).
        fvid::publish::publish_file(&temp.0, output)?;
        stats
    };
    if backend_explicit || io::stderr().is_terminal() {
        eprintln!(
            "frames={} input_bytes={} output_bytes={} backend={} device={:?} controlled_memory_bytes={}",
            stats.frames,
            stats.input_bytes,
            stats.output_bytes,
            stats.backend,
            stats.device_name,
            stats.controlled_memory_bytes
        );
    }
    Ok(())
}
fn main() {
    #[cfg(feature = "airbug")]
    let _airbug = match fvid::airbug_runtime::install() {
        Ok(runtime) => Some(runtime),
        Err(e) => {
            eprintln!("fvid: airbug init failed: {e}");
            None
        }
    };
    if let Err(e) = run() {
        #[cfg(feature = "airbug")]
        fvid::airbug_runtime::capture_error(e.as_ref());
        eprintln!("fvid: {e}");
        std::process::exit(1);
    }
}

#!/usr/bin/env python3
"""Explicit benchmark preparation: fetch the reference samples K-Lite's decode-only codecs need (no local encoder exists).

Files land in benchmarks/data/klite/ (gitignored) and their sha256 is pinned in
benchmarks/klite-samples.json, so validate_klite_coverage.py fails loudly when a
sample changes under us instead of silently decoding something else.
"""
import argparse, hashlib, json, pathlib, re, subprocess, sys, urllib.parse
from common import ROOT

BASE = "https://fate-suite.ffmpeg.org"
SAMPLE_BASE = "https://samples.ffmpeg.org"
DEST = ROOT / "benchmarks" / "data" / "klite"
PIN = ROOT / "benchmarks" / "klite-samples.json"
# a proof fixture should stay small enough to fetch on every CI run
MAX_SAMPLE = 8 * 1024 ** 2
# why a candidate was rejected; --auto --verbose prints it
VERBOSE = False

# coverage row -> reference sample; a bare path is relative to the FATE mirror, a "://"
# source is fetched verbatim. Rows without an entry stay `needs-sample` in the gate.
SAMPLES = {
    "vc1": "vc1/SA00040.vc1",
    "lagarith": "lagarith/lagarith-red.avi",
    "vvc": "vvc/wpp-single-slice-pic.vvc",
    "realvideo": "real/rv30.rm",
    "vorbis": "ogg-vorbis/chained-meta.ogg",
    "opus": "ogg-opus/chained-meta.ogg",
    "eac3": "eac3/csi_miami_5.1_256_spx_small.eac3",
    "ape": "lossless-audio/luckynight-mac380-c4000.ape",
    "shorten": "lossless-audio/luckynight-partial.shn",
    "atrac3": "atrac3/mc_sich_at3_066_small.wav",
    "amr": "amrnb/4.75k.amr",
    "rm": "real/spygames-2MB.rmvb",
    "ogv": "ogg/bear.ogv",
    "theora": "ogg/bear.ogv",
    "speex": "https://archive.org/download/fabulas_esopo_01_librivox_speex/fabula_01_005_esopo.spx",
    "ralf": "https://samples.ffmpeg.org/real/AC-ralf/Wonderful_RA10_Lossless_706K_30s.rm",
}


def landed(pin):
    """A pin is only satisfied when its file is actually here at the pinned size."""
    if not pin:
        return False
    path = DEST / pin["file"]
    return path.exists() and path.stat().st_size == pin["bytes"]


def main(argv=None):
    global VERBOSE
    parser = argparse.ArgumentParser()
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--verbose", action="store_true",
                        help="print why each rejected candidate was rejected")
    parser.add_argument("--auto", action="store_true",
                        help="hunt the FATE mirror for the rows listed in AUTO that have no pin yet")
    parser.add_argument("--add", nargs=2, action="append", metavar=("ROW", "SOURCE"), default=[],
                        help="pin a sample the FATE mirror cannot provide, from a URL or a local path")
    args = parser.parse_args(argv)
    VERBOSE = args.verbose
    DEST.mkdir(parents=True, exist_ok=True)
    pins = json.loads(PIN.read_text()) if PIN.exists() and not args.force else {}
    for row, remote in SAMPLES.items():
        path = DEST / f"{row}--{pathlib.PurePosixPath(remote).name}"
        url = remote if "://" in remote else f"{BASE}/{remote}"
        if path.exists() and not args.force:
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
        else:
            result = subprocess.run(["curl", "-sfL", "--max-time", "120", "-o", str(path), url],
                                    capture_output=True)
            if result.returncode or not path.exists() or path.stat().st_size == 0:
                print(f"MISS  {row:<12} {url}  {result.stderr.decode()[:80]}")
                path.unlink(missing_ok=True)
                continue
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if pins.get(row, {}).get("sha256") not in (None, digest):
            print(f"DRIFT {row:<12} pinned {pins[row]['sha256'][:12]} != {digest[:12]}")
        pins[row] = {"url": url, "file": path.name, "sha256": digest, "bytes": path.stat().st_size}
        print(f"OK    {row:<12} {path.name}  {pins[row]['bytes']} bytes")
    for row, source in args.add:
        path, url = resolve_add(row, source)
        if path is None:
            print(f"MISS  {row:<12} {source}")
            continue
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if pins.get(row, {}).get("sha256") not in (None, digest):
            print(f"DRIFT {row:<12} pinned {pins[row]['sha256'][:12]} != {digest[:12]}")
        pins[row] = {"url": url, "file": path.name, "sha256": digest, "bytes": path.stat().st_size}
        print(f"OK    {row:<12} {path.name}  {pins[row]['bytes']} bytes")
    if args.auto:
        def land(row, path, url, probed, tag, miss):
            if path is None:
                print(f"{tag}MISS {row:<12} {miss}")
                return
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            pins[row] = {"url": url, "file": path.name, "sha256": digest, "codec": probed,
                         "bytes": path.stat().st_size}
            print(f"{tag}    {row:<12} {path.name}  {pins[row]['bytes']} bytes")
        for row, (accepted, stream, directories) in AUTO.items():
            if landed(pins.get(row)):
                continue
            land(row, *auto_probe(row, accepted, stream, directories), "AUTO",
                 ",".join(directories))
        # an intact take from the sample archive beats a damaged FATE fixture, so these rows
        # are hunted even when a pin already exists -- but a row already pointing at its
        # external take keeps it
        for row, (accepted, stream, fragments) in EXTERNAL.items():
            if landed(pins.get(row)):
                print(f"EXT    {row:<12} kept {pins[row]['file']}")
                continue
            land(row, *external_probe(row, accepted, stream, fragments), "EXT ",
                 ",".join(fragments))
            if row in CUTS and row in pins:
                pins[row]["cut_frames"] = CUTS[row]
    for row in sorted(set(pins) - (set(SAMPLES) | set(AUTO) | set(EXTERNAL))):
        print(f"ORPHAN {row:<12} no rule asks for this pin; drop it or add the row back")
    PIN.write_text(json.dumps(pins, indent=2, sort_keys=True) + "\n")
    print(f"{len(pins)} samples pinned -> {PIN.relative_to(ROOT)}")
    return 0 if all(row in pins for row in SAMPLES) else 1


def resolve_add(row, source):
    """Land a user-supplied sample under benchmarks/data/klite/ and return (path, url)."""
    name = pathlib.PurePosixPath(source.split("?")[0]).name or "sample"
    path = DEST / f"{row}--{name}"
    if source.startswith(("http://", "https://")):
        path.parent.mkdir(parents=True, exist_ok=True)
        result = subprocess.run(["curl", "-sfL", "--max-time", "300", "-o", str(path), source],
                                capture_output=True)
        if result.returncode or not path.exists() or path.stat().st_size == 0:
            path.unlink(missing_ok=True)
            return None, None
        return path, source
    local = pathlib.Path(source).expanduser()
    if not local.is_file():
        return None, None
    path.parent.mkdir(parents=True, exist_ok=True)
    if local.resolve() != path.resolve():
        path.write_bytes(local.read_bytes())
    return path, f"local:{local}"


# Coverage rows whose fixture no local encoder can produce. FATE carries most of them, but
# under a directory named after the container family rather than the codec, so the pin is
# hunted rather than written down: row -> (codec names ffprobe may report, stream, directories).
# `--auto` takes the smallest probe-matching file from the first directory that yields one.
AUTO = {
    "vp3": (("vp3",), "v", ("vp3", "quicktime")),
    "vp5": (("vp5",), "v", ("vp5", "vp6")),
    "vp6": (("vp6", "vp6a", "vp6f"), "v", ("vp6", "flash-vp6", "ea-vp6")),
    "vp7": (("vp7",), "v", ("vp7", "vp6")),
    "theora": (("theora",), "v", ("ogg",)),
    "svq3": (("svq3",), "v", ("svq3", "quicktime")),
    "dirac": (("dirac",), "v", ("dirac",)),
    "bink": (("bink",), "v", ("bink",)),
    "smacker": (("smacker",), "v", ("smacker",)),
    "flic": (("flic",), "v", ("fli",)),
    "fraps": (("fraps",), "v", ("fraps",)),
    "vmnc": (("vmnc",), "v", ("VMnc",)),
    "loco": (("loco",), "v", ("loco",)),
    "mszh": (("mszh",), "v", ("lcl",)),
    "tscc": (("tscc",), "v", ("tscc",)),
    "tscc2": (("tscc2",), "v", ("tscc",)),
    "camstudio": (("cscd",), "v", ("CSCD",)),
    "qpeg": (("qpeg",), "v", ("qpeg",)),
    "8bps": (("8bps",), "v", ("8bps",)),
    "truemotion1": (("truemotion1",), "v", ("duck",)),
    "truemotion2": (("truemotion2", "truemotion2rt"), "v", ("truemotion2", "tmv", "vcr1", "duck")),
    "vcr1": (("vcr1",), "v", ("vcr1",)),
    "wnv1": (("wnv1",), "v", ("wnv1",)),
    "ultimotion": (("ulti",), "v", ("ulti",)),
    "vixl": (("vixl",), "v", ("vixl",)),
    "indeo2": (("indeo2",), "v", ("rt21", "iv32")),
    "indeo3": (("indeo3",), "v", ("iv32", "iv8")),
    "indeo4": (("indeo4",), "v", ("iv41",)),
    "indeo5": (("indeo5",), "v", ("iv50",)),
    "rv40": (("rv40",), "v", ("real",)),
    "hap": (("hap",), "v", ("hap",)),
    "mdec": (("mdec",), "v", ("mdec", "psx-str")),
    "4xm": (("4xm",), "v", ("4xm",)),
    "canopushq": (("hq_hqa", "hqx"), "v", ("canopus",)),
    "mirillis": (("fic",), "v", ("fic",)),
    "icod": (("aic",), "v", ("quicktime", "mov")),
    "g2m": (("g2m",), "v", ("g2m", "asf")),
    "bink": (("binkvideo",), "v", ("bink",)),
    "smacker": (("smackvideo", "smackaud"), "v", ("smacker",)),
    "flic": (("flic",), "v", ("fli",)),
    "msmpeg4v1": (("msmpeg4v1", "mpeg4"), "v", ("msmpeg4v1",)),
    "mp1": (("mp1", "mp2", "mp3"), "a", ("mp3-conformance", "mpegaudio")),
    "musepack": (("musepack7", "musepack8"), "a", ("musepack",)),
    "tak": (("tak",), "a", ("lossless-audio", "tak")),
    "wmapro": (("wmapro",), "a", ("wmapro",)),
    "wmalossless": (("wmalossless",), "a", ("wmapro", "xwma")),
    "cook": (("cook",), "a", ("real", "realaudio")),
    "sipr": (("sipr",), "a", ("sipr", "real")),
    "atrac1": (("atrac1",), "a", ("atrac1",)),
    "atrac3p": (("atrac3p",), "a", ("atrac3p", "oma")),
    "gsm_ms": (("gsm_ms",), "a", ("gsm",)),
    "truespeech": (("truespeech",), "a", ("truespeech",)),
    "imc": (("imc",), "a", ("imc", "wmapro")),
    "als": (("mp4als", "als"), "a", ("lossless-audio",)),
    "wtv": (("wmv3", "vc1", "mpeg2video", "mp2", "aac", "msmpeg4v3"), "v", ("wtv",)),
    "dss": (("dss_sp",), "a", ("dss",)),
    "adpcm_ct": (("adpcm_ct",), "a", ("creative", "vqf", "sol", "iv8")),
    "ra_144": (("14_4", "ra_144", "28_8"), "a", ("realaudio", "real")),
    "qdm2": (("qdm2",), "a", ("mov", "quicktime")),
    "mace": (("mace3", "mace6"), "a", ("mov", "quicktime")),
    "dsd": (("dsmc", "dsd_msbf", "dsd_lsbf"), "a", ("pcm-dvd", "pcm-dvda", "dst")),
    "g729": (("g729",), "a", ("g729", "realaudio", "sipr")),
    "wmalossless": (("wmalossless",), "a", ("wmapro", "xwma", "wmavoice")),
    "nclc": (("notchlc",), "v", ("notchlc", "mov", "vvc")),
    "g729": (("g729",), "a", ("act", "g728", "g723_1")),
    "s302m": (("s302m",), "a", ("w64", "dolby_e", "pcm-dvd")),
    "cyuv": (("cyuv",), "v", ("cyuv",)),
    "aasc": (("aasc",), "v", ("aasc",)),
    # the only PVA take the FATE mirror carries is a 1 MB truncation; its video never reads
    # clean anywhere, so the row is proven through the MP2 track (see CONTAINERS in the gate)
    "pva": (("mp2",), "a", ("pva",)),
    # image codecs with no encoder in this build: the still itself is the fixture, so the take
    # has to come from the mirror. A directory that holds no such file simply reports MISS.
    "webp": (("webp",), "v", ("webp",)),
    "dds": (("dds",), "v", ("dds",)),
    "jpegls": (("jpegls",), "v", ("jpegls",)),
    "brender_pix": (("brender_pix",), "v", ("brenderpix",)),
    "txd": (("txd",), "v", ("txd",)),
    "ptx": (("ptx",), "v", ("ptx",)),
    "xface": (("xface",), "v", ("xface",)),
    "xpm": (("xpm",), "v", ("xpm", "xbm")),
    "pgmyuv": (("pgmyuv",), "v", ("pgmyuv", "pam")),
    "vbn": (("vbn",), "v", ("vbn",)),
    "gem": (("gem",), "v", ("gem",)),
    "vc1image": (("vc1image",), "v", ("vc1",)),
    "wmv3image": (("wmv3image",), "v", ("wmv3image", "vc1")),
}


def listing(url):
    """File names from an Apache index, smallest first."""
    page = subprocess.run(["curl", "-sfL", "--max-time", "60", url], capture_output=True)
    if page.returncode:
        return []
    entries = []
    for row in re.finditer(r'<a href="([^"/][^"]*)">[^<]*</a>\s+[\d-]+\s[\d:]+\s+([\d.]+)([KM]?)',
                           page.stdout.decode(errors="replace")):
        name, digits, suffix = row.groups()
        if name.startswith("?"):
            continue
        size = float(digits) * {"": 1, "K": 1024, "M": 1024 ** 2}[suffix]
        entries.append((size, name))
    return [name for _, name in sorted(entries)]


DAMAGED = ("partial", "truncat", "corrupt", "bad", "broken", "crash", "segfault")

# a fixture stays small enough to fetch on every run, so oversized canonical samples are
# trimmed without re-encoding rather than dropped. The budget is counted in frames/packets,
# not seconds: archive takes are often truncated at the tail, and a short prefix stays intact
# where a longer one would carry the damaged last frame. FFmpeg cannot re-wrap .dsf, so DSD has
# no clip to pin and its row stays `needs-sample`.
CUTS = {"nclc": 5, "icod": 5, "aasc": 20}

# one-off exceptions to the fixture budget, for formats FFmpeg cannot re-wrap at all: `.dsf` has
# a demuxer but no muxer, and a byte prefix trips the demuxer's own size accounting, so the
# only honest take is the whole reference file. `.jv` is the same case one size smaller.
BUDGETS = {"dsd": 320 * 1024 ** 2, "mtv": 24 * 1024 ** 2, "dav": 30 * 1024 ** 2,
           "pva": 12 * 1024 ** 2, "jv": 30 * 1024 ** 2}

# Some formats exist in FATE only as deliberately damaged files, which proves error handling
# rather than the codec, and the video decode path is strict about damage on purpose. The
# project's own sample archive keeps intact takes of them, so hunt there by path fragment and
# accept only a file the FFmpeg CLI also reads end to end without errors.
EXTERNAL = {
    "8bps": (("8bps",), "v", ("V-codecs/8BPS-PlanarRGB",)),
    "indeo4": (("indeo4",), "v", ("testsuite/iv41.avi",)),
    "rv40": (("rv40",), "v", ("real/VC-RV40",)),
    "icod": (("aic",), "v", ("V-codecs/icod",)),
    "nclc": (("notchlc",), "v", ("V-codecs/NotchLC",)),
    "qdm2": (("qdm2",), "a", ("A-codecs/QDM2",)),
    "mace": (("mace3", "mace6"), "a", ("A-codecs/MACE/mac3",)),
    "wmalossless": (("wmalossless",), "a", ("A-codecs/lossless/luckynight.wma",)),
    "metasound": (("metasound",), "a", ("VoxwareMetaSound",)),
    "g729": (("g729",), "a", ("A-codecs/act",)),
    "dsd": (("dsd_lsbf_planar", "dsd_msbf", "dsd_lsbf"), "a", ("2L-125_stereo",)),
    "mtv": (("rawvideo", "mjpeg"), "v", ("mtv/comedian_auto.mtv", "mtv/sfu-desync.mtv")),
    "dav": (("h264", "mpeg4"), "v", ("ticket6144",)),
    # "" in `accepted` means "the point of this fixture is that this build cannot read it",
    # so a failed probe is the pass condition and the clean-read rule is skipped.
    "midi": (("",), "a", ("A-codecs/suite/MIDI/breeze.mid",)),
    "aasc": (("aasc",), "v", ("V-codecs/AASC/AASC.AVI",)),
    "cpng": (("png",), "v", ("V-codecs/PNG1/corepng.avi",)),
    # the archive's only JV take. It stays whole because `.jv` has a demuxer but no muxer and no
    # encoder in this build: nothing can re-wrap a prefix of it.
    "jv": (("jv",), "v", ("game-formats/jv/E_INTRO.JV",)),
    # the archive keeps per-FourCC directories named after the tag, and these two are the only
    # takes of their kind: `V-codecs/I263` is a genuine Intel capture, `V-codecs/ARBC` Gryphon's.
    "i263": (("h263i",), "v", ("V-codecs/I263/i263.avi",)),
    "arbc": (("arbc",), "v", ("V-codecs/ARBC/LOBBY1.AVI", "V-codecs/ARBC/ED.AVI")),
    # GeoVision ships two of its own fourcc tags, and the archive keeps a take of each:
    # `V-codecs/geov.avi` is MPEG-4 Part 2 under `GEOV`, `V-codecs/GAVC` is H.264 under `GAVC`.
    # The camera take is the one the archive still serves whole; the `GEOV` one is old enough
    # that its own headers decode only under concealment.
    "geov": (("h264", "mpeg4"), "v", ("V-codecs/GAVC/GeoVision_camera.avi",)),
}


def index_paths():
    """The sample archive's own file index, cached next to the samples it points at."""
    cache = DEST / ".allsamples.txt"
    if not cache.exists():
        DEST.mkdir(parents=True, exist_ok=True)
        result = subprocess.run(["curl", "-sfL", "--max-time", "180", "-o", str(cache),
                                 f"{SAMPLE_BASE}/allsamples.txt"], capture_output=True)
        if result.returncode:
            cache.unlink(missing_ok=True)
            return []
    return [line[2:] for line in cache.read_text(errors="replace").splitlines()
            if line.startswith("./") and not line.endswith((".txt", ".log"))]


def note(reason):
    if VERBOSE:
        print(f"      reject: {reason}")


def download(url, path, timeout):
    """Fetch `url` to `path`, reporting the failure rather than swallowing it."""
    result = subprocess.run(["curl", "-sfL", "--max-time", str(timeout), "-o", str(path), url],
                            capture_output=True)
    if result.returncode or not path.exists() or path.stat().st_size == 0:
        note(f"{path.name}: curl {result.returncode} {result.stderr.decode(errors='replace')[:80]}")
        path.unlink(missing_ok=True)
        return False
    return True


def probed_codec(path, stream):
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", f"{stream}:0",
                            "-show_entries", "stream=codec_name", "-of", "default=nw=1:nk=1",
                            str(path)], capture_output=True)
    return probe.stdout.decode().strip()


def intact(path, stream):
    """Whether the FFmpeg CLI decodes the row's own stream of the file end to end without error."""
    result = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-xerror", "-i", str(path),
                             "-map", f"0:{stream}:0", "-f", "null", "-"],
                            capture_output=True, timeout=900)
    if result.returncode:
        note(f"{path.name}: -xerror read failed: {result.stderr.decode(errors='replace')[-120:]}")
    return not result.returncode


def external_probe(row, accepted, stream, fragments):
    """Land an intact sample of `row` from the FFmpeg sample archive, probed before kept.

    Rows listed in CUTS are pinned as copy-only prefixes of a larger canonical file: ffmpeg
    stops reading after the frame budget, so a multi-hundred-megabyte original is never fully
    downloaded and the truncated tail of a damaged take is left out. The pin records the source
    URL, the probed codec and the frame budget.
    """
    cut = CUTS.get(row)
    budget = BUDGETS.get(row, MAX_SAMPLE)
    candidates = [path for path in index_paths()
                  if any(fragment.lower() in path.lower() for fragment in fragments)][:8]
    for path in candidates:
        url = f"{SAMPLE_BASE}/{urllib.parse.quote(path)}"
        name = pathlib.PurePosixPath(path).name
        suffix = name.rsplit(".", 1)[-1]
        target = DEST / (f"{row}--{name}" if not cut else f"{row}--{name[: -len(suffix) - 1]}-cut.{suffix}")
        if cut:
            result = subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", url,
                                     "-map", f"0:{stream}:0", f"-frames:{stream}", str(cut),
                                     "-c", "copy", str(target)],
                                    capture_output=True, timeout=900)
            if result.returncode:
                note(f"{name}: copy-cut of {cut} frames failed: "
                     f"{result.stderr.decode(errors='replace')[-120:]}")
                target.unlink(missing_ok=True)
                continue
        elif not download(url, target, max(300, int(budget / 200_000))):
            continue
        if target.stat().st_size > budget:
            note(f"{name}: {target.stat().st_size} bytes over budget {budget}")
            target.unlink(missing_ok=True)
            continue
        probed = probed_codec(target, stream)
        if probed not in accepted:
            note(f"{name}: probed {probed or 'nothing'}, want "
                 f"{'/'.join(a for a in accepted if a) or 'an unreadable file'}")
            target.unlink(missing_ok=True)
            continue
        if "" in accepted or reads_clean(target, stream):
            return target, url, probed
        target.unlink(missing_ok=True)
    return None, None, None


def decoded_frames(path, stream):
    """How many frames the CLI gets out of the file; zero means the stream never demuxes."""
    result = subprocess.run(["ffprobe", "-v", "error", "-select_streams", f"{stream}:0",
                             "-count_frames", "-show_entries", "stream=nb_read_frames",
                             "-of", "default=nw=1:nk=1", str(path)], capture_output=True, timeout=600)
    text = result.stdout.decode().strip()
    return int(text) if text.isdigit() else 0


def reads_clean(path, stream):
    """What makes a candidate sample a real proof rather than an accident.

    Video: the strict decode path needs a file with nothing wrong in it, so a clean `-xerror`
    read is the requirement. Audio: `decode-audio` rides out damage like the FFmpeg CLI does and
    the actual assertion is bit-exact PCM against the oracle on the same file, so a truncated
    take is a legitimate fixture — but a file the CLI cannot pull a single frame out of is not,
    which is what separates FATE's cut WMA Pro sample from its `.act` G.729 takes.
    """
    if stream == "v":
        return intact(path, stream)
    count = decoded_frames(path, stream)
    if not count:
        note(f"{path.name}: the CLI demuxes zero frames")
    return bool(count)


def auto_probe(row, accepted, stream, directories):
    """Take the smallest FATE file a probe identifies as one of `accepted` codec names and that
    survives a clean read.
    """
    for directory in directories:
        names = listing(f"{BASE}/{directory}/")
        for name in sorted(names, key=lambda item: any(tag in item.lower() for tag in DAMAGED))[:40]:
            url = f"{BASE}/{directory}/{urllib.parse.quote(name)}"
            path = DEST / f"{row}--{name}"
            if not download(url, path, 300):
                continue
            if path.stat().st_size > BUDGETS.get(row, MAX_SAMPLE):
                note(f"{name}: {path.stat().st_size} bytes over budget")
                path.unlink(missing_ok=True)
                continue
            probed = probed_codec(path, stream)
            if probed not in accepted:
                note(f"{name}: probed {probed or 'nothing'}, want {'/'.join(a for a in accepted if a)}")
                path.unlink(missing_ok=True)
                continue
            if reads_clean(path, stream):
                return path, url, probed
            path.unlink(missing_ok=True)
    return None, None, None


if __name__ == "__main__":
    sys.exit(main())

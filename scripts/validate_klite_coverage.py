#!/usr/bin/env python3
"""K-Lite Codec Pack coverage probe.

Rows come from the K-Lite 20.0.0 ability table, one row per FFmpeg codec its LAV/ffdshow
backends dispatch to. A row is only "covered" when fvid actually decodes a fixture of that
codec; the FFmpeg CLI is a fixture generator and oracle, never the thing under test.
Rows no local encoder can produce need a pinned reference sample and are reported as
`needs-sample` until one is fetched; rows the linked FFmpeg cannot even read are
`absent-in-build` (K-Lite ships its own filter for those), not fvid failures.
"""
import argparse, hashlib, json, os, pathlib, re, resource, selectors, struct, subprocess, sys, \
       tempfile, time
from common import ROOT

BINARY = ROOT / "target-media" / "release" / "fvid"
FRAMES = 5

# K-Lite 20.0.0 ability table (https://www.codecguide.com/klcp_ability_comparison.htm),
# reduced to one row per FFmpeg decoder that LAV/ffdshow dispatch to: FourCC groups sharing a
# decoder are one row (DivX/Xvid -> mpeg4 plus msmpeg4v1/v2/v3, DVSD..DVHP -> dvvideo,
# DTS/ES/HD/LBR -> dca). name, k-lite component, container, ffmpeg fixture args
# (None => no local encoder, so a pinned reference sample proves the row).
VIDEO = [
    ("h264", "LAV Video / x264vfw", "mp4", ["-c:v", "libx264", "-preset", "ultrafast", "-g", "5"]),
    ("hevc", "LAV Video / x265vfw", "mp4", ["-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "pmode=intra"]),
    ("av1", "LAV Video (AV1)", "mp4", ["-c:v", "libsvtav1", "-svtav1-params", "lp=1:tile-columns=0"]),
    ("vp9", "LAV Video / ffdshow", "webm", ["-c:v", "libvpx-vp9", "-speed", "8", "-b:v", "0"]),
    ("vp8", "ffdshow / WebM", "webm", ["-c:v", "libvpx", "-speed", "8", "-b:v", "0"]),
    ("vp7", "Flash Video VP7 (VP70)", "flv", None),
    ("vp6", "Flash Video VP6 (VP60-62, FLV4)", "flv", None),
    ("vp5", "Flash Video VP5 (VP50)", "flv", None),
    ("vp3", "QuickTime VP3 (VP31)", "mov", None),
    ("mpeg2", "LAV Video (MPEG-2)", "mpg", ["-c:v", "mpeg2video"]),
    ("mpeg1", "LAV Video (MPEG-1)", "mpg", ["-c:v", "mpeg1video"]),
    ("mpeg4p2", "DivX / Xvid / ffdshow (XVID, DX50)", "avi", ["-c:v", "mpeg4", "-q:v", "5"]),
    ("msmpeg4v3", "MS MPEG-4 v3 (MP43, DIV3-6)", "avi", ["-c:v", "msmpeg4"]),
    ("msmpeg4v2", "MS MPEG-4 v2 (MP42, DIV2)", "avi", ["-c:v", "msmpeg4v2"]),
    ("msmpeg4v1", "MS MPEG-4 v1 (MP41, DIV1)", "avi", None),
    ("h263", "ffdshow (H.263)", "avi", ["-s", "176x144", "-c:v", "h263", "-q:v", "7"]),
    ("i263", "Intel H.263 (I263)", "avi", None),
    ("h261", "H.261 (H261, M261)", "avi", ["-s", "176x144", "-c:v", "h261"]),
    ("flv1", "Flash Video H.263 (FLV1)", "flv", ["-c:v", "flv"]),
    ("wmv1", "Windows Media Video 7 (WMV1)", "asf", ["-c:v", "wmv1"]),
    ("wmv2", "Windows Media Video 8 (WMV2)", "asf", ["-c:v", "wmv2"]),
    ("vc1", "LAV Video (VC-1 / WMV3, WVC1, WMVA)", "wmv", None),
    ("theora", "Theora (THEO)", "ogg", None),
    ("svq1", "Sorenson Video 1 (SVQ1)", "mov", ["-c:v", "svq1"]),
    ("svq3", "Sorenson Video 3 (SVQ3)", "mov", None),
    ("dvvideo", "Digital Video (DVSD, DV25/50, DVCPRO)", "mov", ["-c:v", "dvvideo", "-s", "720x576"]),
    ("mjpeg", "ffdshow / Lav (MJPG, AVRN, SP5X)", "avi", ["-c:v", "mjpeg", "-q:v", "4"]),
    ("jpeg2000", "JPEG 2000 / DCinema (MJP2, MJ2C)", "mov", ["-c:v", "jpeg2000"]),
    ("dirac", "BBC Dirac / VC-2 (DRAC, DRCQ)", "mov", None),
    ("prores", "Full/Mega decoding filters", "mov", ["-c:v", "prores_ks", "-profile:v", "3"]),
    ("dnxhd", "Avid DNxHD / DNxHR (AVdn, AVdh)", "mov", ["-s", "1920x1080", "-r", "25", "-c:v", "dnxhd", "-b:v", "120M", "-pix_fmt", "yuv422p"]),
    ("cfhd", "CineForm HD (Mega)", "mov", ["-c:v", "cfhd"]),
    ("ffv1", "ffdshow / VFW lossless", "mkv", ["-c:v", "ffv1", "-level", "3"]),
    ("huffyuv", "Huffyuv VFW", "avi", ["-c:v", "huffyuv", "-pix_fmt", "yuv422p"]),
    ("ffvhuff", "Huffyuv 2.2+ (FFVH)", "avi", ["-c:v", "ffvhuff"]),
    ("lagarith", "Lagarith VFW", "avi", None),
    ("utvideo", "Ut Video (Mega)", "mkv", ["-c:v", "utvideo", "-pix_fmt", "rgb24"]),
    ("magicyuv", "MagicYUV (MAGY)", "avi", ["-c:v", "magicyuv", "-pix_fmt", "argb"]),
    ("zlib", "Lossless Codec Library (ZLIB)", "avi", ["-c:v", "zlib"]),
    ("mszh", "Lossless Codec Library (MSZH)", "avi", None),
    ("cinepak", "Cinepak (CVID)", "avi", ["-c:v", "cinepak"]),
    ("camstudio", "CamStudio (CSCD)", "avi", None),
    ("qtrle", "QuickTime Animation (QRLE)", "mov", ["-c:v", "qtrle"]),
    ("rpza", "QuickTime RPZA (RPZA)", "mov", ["-c:v", "rpza"]),
    ("smc", "QuickTime Graphics (SMC)", "mov", ["-c:v", "smc"]),
    ("8bps", "QuickTime Planar RGB (8BPS)", "mov", None),
    ("msrle", "MS RLE (MRLE)", "avi", ["-c:v", "msrle"]),
    ("msvideo1", "MS Video 1 (CRAM, MSVC)", "avi", ["-c:v", "msvideo1"]),
    ("asv1", "ASUS Video v1 (ASV1)", "avi", ["-c:v", "asv1"]),
    ("asv2", "ASUS Video v2 (ASV2)", "avi", ["-c:v", "asv2"]),
    ("fraps", "Fraps (FPS1)", "avi", None),
    ("vmnc", "VMware screen codec (VMnc)", "avi", None),
    ("loco", "LOCO (LOCO)", "avi", None),
    ("qpeg", "Q-Team QPEG (QP10, QP11)", "mov", None),
    ("tscc", "TechSmith Camtasia (TSCC)", "avi", None),
    ("tscc2", "TechSmith Camtasia HD (TSC2)", "trec", None),
    ("truemotion1", "Duck / TrueMotion 1 (DUCK)", "avi", None),
    ("truemotion2", "TrueMotion 2 (TM20)", "avi", None),
    ("vcr1", "ATI VCR1 (VCR1)", "avi", None),
    ("wnv1", "Winnov Caviara (WNV1)", "avi", None),
    ("ultimotion", "IBM Ultimotion (ULTI)", "avi", None),
    ("vixl", "Miro VideoXL (VIXL)", "avi", None),
    ("indeo2", "Intel Indeo 2 (RT21)", "avi", None),
    ("indeo3", "Intel Indeo 3 (IV31, IV32)", "avi", None),
    ("indeo4", "Intel Indeo 4 (IV41)", "avi", None),
    ("indeo5", "Intel Indeo 5 (IV50)", "avi", None),
    ("rv10", "RealVideo 1 (RV10)", "rm", ["-s", "128x64", "-c:v", "rv10"]),
    ("rv20", "RealVideo G2 / 2 (RV20)", "rm", ["-c:v", "rv20"]),
    ("realvideo", "RealVideo 3 (RV30)", "rm", None),
    ("rv40", "RealVideo 4 (RV40)", "rm", None),
    ("hap", "Hap (Hap1, Hap5, HapY)", "mov", None),
    ("dxv", "Avid DXV (DXD3, DXDI)", "mov", ["-c:v", "dxv"]),
    ("mdec", "PlayStation Motion Decoder (MDEC)", "str", None),
    ("4xm", "4X Video (4XMV)", "4xm", None),
    ("canopushq", "Canopus HQ / HQX (CUVC)", "mov", None),
    ("mirillis", "Mirillis FIC (FICV)", "avi", None),
    ("cyuv", "Creative YUV (CYUV)", "avi", None),
    ("aasc", "Autodesk Animation RLE (AASC)", "avi", None),
    ("cpng", "CorePNG (PNG1, MPNG)", "mov", None),
    ("zmbv", "Zip Motion Blocks Video (ZMVB)", "avi", ["-c:v", "zmbv"]),
    ("icod", "Apple Intermediate Codec (ICOD)", "mov", None),
    ("nclc", "NotchLC (NCLC)", "mov", None),
    ("geov", "GeoVision (GEOV)", "avi", None),
    ("g2m", "GoToMeeting (G2M2-5)", "asf", None),
    ("arbc", "Gryphon's Anim Compressor (ARBC)", "avi", None),
    ("bink", "Bink Video (BIKI, BIKB)", "bik", None),
    ("smacker", "Smacker Video (SMK2, SMK4)", "smk", None),
    ("flic", "Autodesk FLIC (FLI8, FLC1)", "fli", None),
    ("vvc", "H.266/VVC (Mega)", "mp4", None),
    # The Full pack sells a 3-D plugin on top of H.264; no descriptor in `ffmpeg -codecs` is MVC,
    # so the row can only be recorded as out of this build, which is what the gate then measures.
    ("h264mvc", "H.264 MVC 3D (Full, LAV Video with the MVC plugin)", "mov", None),
]

AUDIO = [
    ("aac", "LAV Audio / CoreAAC", "m4a", ["-c:a", "aac"]),
    ("mp3", "LAV Audio / LAME ACM", "mp3", ["-c:a", "libmp3lame", "-b:a", "96k"]),
    ("mp2", "MPEG-1 Layer II (MP2)", "mp2", ["-c:a", "mp2", "-b:a", "128k"]),
    ("mp1", "MPEG-1 Layer I (MP1)", "mp2", None),
    ("flac", "LAV Audio", "flac", ["-c:a", "flac"]),
    ("alac", "LAV Audio (Apple)", "m4a", ["-c:a", "alac"]),
    ("vorbis", "LAV Audio / OggDs", "ogg", None),
    ("opus", "LAV Audio (WebM)", "ogg", None),
    ("ac3", "AC3Filter / LAV", "ac3", ["-c:a", "ac3", "-b:a", "128k"]),
    ("eac3", "LAV Audio (E-AC-3)", "eac3", None),
    ("truehd", "LAV Audio (Dolby TrueHD / MLP)", "mkv", ["-strict", "-2", "-c:a", "truehd"]),
    ("dts", "DTSDecoder / LAV (DTS, ES, HD MA/HRA, LBR)", "dts", ["-strict", "-2", "-c:a", "dca", "-b:a", "448k"]),
    ("ac4", "Dolby AC-4 (partial)", "mp4", None),
    ("wmav1", "WMA v1 (160)", "asf", ["-c:a", "wmav1"]),
    ("wmav2", "WMA v2 (161/353, DivX WMA)", "asf", ["-c:a", "wmav2"]),
    ("wmapro", "WMA Pro (162)", "wma", None),
    ("wmalossless", "WMA Lossless (163)", "wma", None),
    ("tta", "TrueAudio (Mega)", "tta", ["-c:a", "tta"]),
    ("wavpack", "WavPack (Mega)", "wv", ["-c:a", "wavpack"]),
    ("ape", "Monkey's Audio (Mega)", "ape", None),
    ("musepack", "Musepack SV7 / SV8", "mpc", None),
    ("tak", "TAK (Mega)", "tak", None),
    ("atrac1", "ATRAC 1 (ATRC)", "aea", None),
    ("atrac3", "Atrac / ATRAC3 (Mega)", "oma", None),
    ("atrac3p", "ATRAC 3+ (AT3+)", "aa3", None),
    ("speex", "Speex (Mega)", "spx", None),
    ("g711", "G.711 (A-law / Mu-law)", "wav", ["-c:a", "pcm_alaw", "-ar", "8000"]),
    ("g722", "G.722 / VoIP", "wav", ["-c:a", "g722", "-ar", "16000"]),
    ("g723_1", "G.723.1 / VoIP", "wav", ["-c:a", "g723_1", "-ar", "8000"]),
    ("g726", "G.726 / VoIP", "wav", ["-c:a", "g726", "-b:a", "32k", "-ar", "8000"]),
    ("g729", "G.729 / VoIP", "wav", None),
    ("gsm_ms", "MS GSM (49)", "wav", None),
    ("truespeech", "TrueSpeech (22)", "wav", None),
    ("metasound", "Voware Metasound (RT29)", "wav", None),
    ("imc", "Intel Music Coder (401)", "avi", None),
    ("qdm2", "QDesign Music 2 (QDM2)", "mov", None),
    ("mace", "Apple MACE (MAC3, MAC6)", "mov", None),
    ("als", "MPEG-4 ALS (lossless audio)", "mp4", None),
    ("dsd", "Direct Stream Digital (DSF / DFF)", "dff", None),
    # MPEG-TS is where ST 302M puts its essence; WAV/QuickTime/NUT refuse the tag and MXF
    # wants a video track first, which is why this row looked unprovable from a local encoder.
    # the encoder reads `-strict -2` only as an output option, so the fixture's global one is
    # not enough for this row.
    ("s302m", "SMPTE 302M (AES3)", "ts", ["-ac", "2", "-c:a", "s302m", "-strict", "-2"]),
    ("pcm", "LPCM (uncompressed PCM)", "wav", ["-c:a", "pcm_s24le"]),
    ("pcm_qt", "QuickTime PCM (TWOS, SOWT)", "mov", ["-c:a", "pcm_s16be"]),
    ("adpcm", "IMA ADPCM (11, 61, 62)", "avi", ["-c:a", "adpcm_ima_wav"]),
    ("adpcm_qt", "Apple IMA ADPCM (61)", "mov", ["-c:a", "adpcm_ima_qt"]),
    ("adpcm_ms", "Microsoft ADPCM (2, 17)", "avi", ["-c:a", "adpcm_ms"]),
    ("adpcm_ct", "Creative ADPCM (200)", "wav", None),
    ("nellymoser", "Flash / FLV", "flv", ["-ar", "8000", "-c:a", "nellymoser"]),
    ("shorten", "Shorten (Mega)", "shn", None),
    ("ralf", "RealAudio Lossless (Mega)", "rm", None),
    ("cook", "RealAudio Cook (COOK)", "ra", None),
    ("sipr", "RealAudio Sipr (SIPR)", "ra", None),
    ("ra_144", "RealAudio 1 / 2 (14_4, 28_8)", "ra", None),
    ("optimfrog", "OptimFROG (Mega)", "ofr", None),
]

# K-Lite plays these containers; fvid gates them behind a format_whitelist, so a
# codec can be fully supported and still unreachable because of the wrapper.
# label, extension, source ('v' video / 'a' audio), encoder args (None => no local encoder)
CONTAINERS = [
    ("mp4", "mp4", "v", ["-c:v", "libx264", "-preset", "ultrafast"]),
    ("mkv", "mkv", "v", ["-c:v", "libx264", "-preset", "ultrafast"]),
    ("avi", "avi", "v", ["-c:v", "mpeg4", "-q:v", "5"]),
    ("mov", "mov", "v", ["-c:v", "mpeg1video"]),
    ("webm", "webm", "v", ["-c:v", "libvpx-vp9", "-speed", "8", "-b:v", "0"]),
    ("flv", "flv", "v", ["-c:v", "flv", "-s", "320x240"]),
    ("asf", "asf", "v", ["-c:v", "wmv2", "-s", "320x240"]),
    ("mpg", "mpg", "v", ["-c:v", "mpeg2video"]),
    ("y4m", "y4m", "v", ["-c:v", "rawvideo", "-pix_fmt", "yuv420p"]),
    ("mp3", "mp3", "a", ["-c:a", "libmp3lame", "-b:a", "96k"]),
    ("m4a", "m4a", "a", ["-c:a", "aac"]),
    ("flac", "flac", "a", ["-c:a", "flac"]),
    ("wav", "wav", "a", ["-c:a", "pcm_s16le"]),
    ("aiff", "aiff", "a", ["-c:a", "pcm_s16be"]),
    ("caf", "caf", "a", ["-c:a", "alac"]),
    ("au", "au", "a", ["-c:a", "pcm_s16be"]),
    ("m2ts", "m2ts", "v", ["-c:v", "libx264", "-preset", "ultrafast"]),
    ("ts", "ts", "v", ["-c:v", "mpeg2video"]),
    ("vob", "vob", "v", ["-c:v", "mpeg2video", "-s", "720x576"]),
    ("dv", "dv", "v", ["-c:v", "dvvideo", "-s", "720x576"]),
    ("mxf", "mxf", "v", ["-c:v", "mpeg2video", "-s", "720x576"]),
    ("ogv", "ogv", "v", None),
    ("m4v", "m4v", "v", ["-c:v", "libx264", "-preset", "ultrafast"]),
    ("swf", "swf", "v", ["-c:v", "flv", "-s", "320x240"]),
    ("3gp", "3gp", "v", ["-c:v", "libx264", "-preset", "ultrafast", "-s", "176x144"]),
    ("wtv", "wtv", "v", None),
    ("bink", "bik", "v", None),
    ("smacker", "smk", "v", None),
    ("mka", "mka", "a", ["-c:a", "flac"]),
    ("opus", "ogg", "a", None),
    ("wv", "wv", "a", ["-c:a", "wavpack"]),
    ("tta", "tta", "a", ["-c:a", "tta"]),
    ("amr", "amr", "a", None),
    ("rm", "rm", "a", None),
    ("ape", "ape", "a", None),
    ("musepack", "mpc", "a", None),
    ("tak", "tak", "a", None),
    ("dss", "dss", "a", None),
    ("mtv", "mtv", "v", None),
    ("dav", "dav", "v", None),
    ("midi", "mid", "a", None),
    # proven through the MP2 track: no intact PVA take exists in either repository, and the
    # wrapper claim is that fvid parses .pva and decodes what it carries.
    ("pva", "pva", "a", None),
    ("ivf", "ivf", "v", ["-c:v", "libvpx", "-speed", "8", "-b:v", "0"]),
    ("cda", "cda", "a", None),
    ("tracker", "xm", "a", None),
]


# K-Lite markets image display as a Mega-only component (`MPC Image Source`, "for displaying image
# formats in DirectShow applications") and publishes no extension list for it, so the row set is
# derived from the build instead of from a sentence: every codec `ffmpeg -codecs` describes as an
# image/bitmap/raster format and this libavcodec decodes. `.webp` is the one format the ability
# table does name (a tag of the WebM row). A row is either a still this build can write, or a
# pinned reference take, or it sits in IMAGE_UNPROVABLE with the reason the gate re-checks.
# name, extension, ffmpeg fixture args ([] => a reference sample proves the row), frames expected
IMAGE = [
    ("alias_pix", "pix", ["-c:v", "alias_pix"], 1),
    ("apng", "apng", ["-f", "apng"], 5),
    ("bmp", "bmp", ["-c:v", "bmp"], 1),
    ("brender_pix", "pix", [], 1),
    ("dds", "dds", [], 1),
    ("dpx", "dpx", ["-c:v", "dpx"], 1),
    ("exr", "exr", ["-c:v", "exr"], 1),
    ("fits", "fits", ["-c:v", "fits"], 1),
    ("gem", "gem", "hand", 1),
    ("gif", "gif", ["-c:v", "gif"], 1),
    ("hdr", "hdr", ["-c:v", "hdr"], 1),
    ("jpegls", "jls", [], 1),
    # admitted on the replaced bar in `TOLERATED_EOF`: its demuxer ends a whole file with an error
    ("jv", "jv", "hand", 1),
    ("pam", "pam", ["-c:v", "pam"], 1),
    ("pbm", "pbm", ["-c:v", "pbm"], 1),
    ("pcx", "pcx", ["-c:v", "pcx"], 1),
    ("pfm", "pfm", ["-c:v", "pfm"], 1),
    ("pgm", "pgm", ["-c:v", "pgm"], 1),
    ("pgmyuv", "pgmyuv", ["-c:v", "pgmyuv"], 1),
    ("phm", "phm", ["-c:v", "phm"], 1),
    ("png", "png", ["-c:v", "png"], 1),
    ("ppm", "ppm", ["-c:v", "ppm"], 1),
    ("ptx", "ptx", [], 1),
    ("qoi", "qoi", ["-c:v", "qoi"], 1),
    ("sgi", "sgi", ["-c:v", "sgi"], 1),
    ("sunrast", "sun", ["-c:v", "sunrast"], 1),
    ("targa", "tga", ["-c:v", "targa"], 1),
    ("tiff", "tiff", ["-c:v", "tiff"], 1),
    ("txd", "txd", [], 1),
    ("vbn", "vbn", ["-c:v", "vbn", "-pix_fmt", "bgra"], 1),
    ("wbmp", "wbmp", ["-c:v", "wbmp"], 1),
    ("webp", "webp", [], 1),
    ("xbm", "xbm", ["-c:v", "xbm"], 1),
    ("xface", "xface", [], 1),
    ("xpm", "xpm", "hand", 1),
    ("xwd", "xwd", ["-c:v", "xwd"], 1),
]

# Image codecs this build decodes but cannot be handed a fixture for. The reason is not a memory:
# `unprovable_evidence` asks the three questions the sentence rests on every run — can this build
# write the format, does the public sample archive's own index name a take of it, and what does
# ffprobe answer for the video take these descriptors share bitstreams with. `image_gaps` turns any
# positive answer into a regression, so a format that becomes provable cannot hide here.
IMAGE_UNPROVABLE = {
    "vc1image": "no encoder here, no take in the sample archive, and the VC-1 takes probe as `vc1`",
    "wmv3image": "no encoder here, no take in the sample archive, and the VC-1 takes probe as `vc1`",
}

# The still-image wrappings of VC-1 are the Microsoft HD Photo family, so the archive is searched by
# the descriptors themselves and by that family's file extensions. An extension is matched as the
# path's suffix: as a substring it also lives inside unrelated file names (`rushdll.dll`).
UNPROVABLE_SEARCH = {
    "vc1image": {"descriptors": ["vc1image", "wmv9image", "jpegxr", "jpeg xr", "hd photo"],
                 "extensions": ["jxr", "wdp", "hdp", "hdl", "emv"]},
    "wmv3image": {"descriptors": ["wmv3image", "wmv9image", "jpegxr", "jpeg xr", "hd photo"],
                  "extensions": ["jxr", "wdp", "hdp", "hdl", "emv"]},
}
# Each exempt descriptor is asked of the takes the matrix proves for the bitstream family it wraps:
# a VC-1 image is a VC-1/WMV3 stream carrying one picture, so what those rows actually probe as is
# the measurement behind "no take of ours reports anything but `vc1`".
UNPROVABLE_FAMILY = {"vc1image": ("vc1", "wmv3"), "wmv3image": ("vc1", "wmv3")}


def plural(count, one, few, many):
    """Russian numeral agreement — the generated summary is prose people read."""
    mod10, mod100 = count % 10, count % 100
    if mod10 == 1 and mod100 != 11:
        return one
    if 2 <= mod10 <= 4 and not 12 <= mod100 <= 14:
        return few
    return many


def sh(command, timeout=90):
    return subprocess.run([str(part) for part in command], capture_output=True, timeout=timeout)


def run_json(command):
    result = sh(command)
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip()[-200:])
    return result.stdout.decode()


def fvid(*args):
    result = sh([BINARY, "media", *args])
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace").strip()[-400:])
    return json.loads(result.stdout)


def fvid_refuses(*args):
    """Whether the binary says no. Used to check that a requirement the caller can fake is
    really required: a read that succeeds here would show the format hint to be optional."""
    try:
        fvid(*args)
        return False
    except Exception:
        return True


def fixture(work, name, container, encode):
    path = work / f"{name}.{container}"
    base = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-strict", "-2", "-f", "lavfi", "-i",
            f"testsrc=size=128x72:rate=25:duration=0.2", "-frames:v", str(FRAMES)]
    result = sh(base + encode + [str(path)])
    if result.returncode:
        return None, result.stderr.decode(errors="replace").strip()
    return path, ""


def audio_fixture(work, name, container, encode):
    path = work / f"{name}.{container}"
    base = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-strict", "-2", "-f", "lavfi", "-i",
            "sine=frequency=440:sample_rate=48000:duration=0.2"]
    result = sh(base + encode + ["-frames:a", "40", str(path)])
    if result.returncode:
        return None, result.stderr.decode(errors="replace").strip()
    return path, ""


# --- ADPCM fixtures ---------------------------------------------------------------------------
#
# QuickTime carries ADPCM as one sample per block, so `stsz` states a block length and `stts` the
# frames one block holds. That is what FFmpeg's muxer writes for `ima4` (34 bytes, 64 frames) and
# not what it writes for its two Microsoft spellings: for those it indexes the track by audio
# frame, `stsz` at one byte per sample and `stts` at one frame per run, which describes packets
# that do not exist. A reader that follows those tables walks off the end of `mdat`, and the muxer
# agrees with that verdict about its own output - `-c copy` on such a file stops at "fatal error,
# input packet contains no samples". Its demuxer reads the file only by ignoring the tables and
# re-splitting the bytes by block size.
#
# So the two Microsoft rows are measured on a file whose tables are authored here to the shape the
# format states, from a file FFmpeg encoded: the sample description, the edit, the chunk offset and
# every byte of coded audio stay the muxer's own, and the honesty of the rewrite is checked rather
# than assumed - `ffprobe` has to report the same packet list before and after it, and the decode
# has to end at the length `mdhd` declares.

MOV_CONTAINERS = {b"moov", b"trak", b"mdia", b"minf", b"stbl"}


def mov_parse(data):
    """The file as (type, payload-or-children) pairs, recursing into the tracks' containers."""
    out, off = [], 0
    while off + 8 <= len(data):
        size = struct.unpack_from(">I", data, off)[0]
        typ = data[off + 4:off + 8]
        if size < 8 or off + size > len(data):
            raise ValueError(f"bad MOV box {typ!r} at {off}")
        body = data[off + 8:off + size]
        out.append((typ, mov_parse(body) if typ in MOV_CONTAINERS else body))
        off += size
    return out


def mov_render(boxes):
    out = b""
    for typ, body in boxes:
        payload = mov_render(body) if typ in MOV_CONTAINERS else body
        out += struct.pack(">I", 8 + len(payload)) + typ + payload
    return out


def mov_find(boxes, path):
    node = boxes
    for step in path:
        hit = next((body for typ, body in node if typ == step), None)
        if hit is None:
            raise ValueError(f"no {step!r} in the track")
        node = hit
    return node


def mov_packetize(path, block, frames_per_block):
    """Rewrite one audio track's `stsz`/`stts`/`stsc` into `block`-byte packets.

    Returns the frame count the authored tables declare, which is the `mdhd` duration the file
    already carried: the last packet keeps whatever of a block the track has left, so a track that
    is not a whole number of blocks says so instead of playing the encoder's padding.
    """
    data = pathlib.Path(path).read_bytes()
    tree = mov_parse(data)
    audio_bytes = next((len(body) for typ, body in tree if typ == b"mdat"), 0)
    stbl = mov_find(tree, [b"moov", b"trak", b"mdia", b"minf", b"stbl"])
    mdhd = mov_find(tree, [b"moov", b"trak", b"mdia", b"mdhd"])
    declared = struct.unpack_from(">I", mdhd, 16)[0]
    full, last = divmod(declared, frames_per_block)
    if last:
        full += 1
    else:
        last = frames_per_block
    if full * block > audio_bytes:
        raise ValueError(f"{full} packets of {block} exceed the {audio_bytes} audio bytes")

    def replace(tag, body):
        at = next(i for i, (typ, _) in enumerate(stbl) if typ == tag)
        stbl[at] = (tag, body)

    replace(b"stsz", struct.pack(">III", 0, block, full))
    replace(b"stts", struct.pack(">II", 0, 1 if full == 1 else 2)
            + (b"" if full == 1 else struct.pack(">II", full - 1, frames_per_block))
            + struct.pack(">II", 1, last))
    replace(b"stsc", struct.pack(">II", 0, 1) + struct.pack(">III", 1, full, 1))
    pathlib.Path(path).write_bytes(mov_render(tree))
    return declared


def mov_packets(path):
    """`ffprobe`'s view of the audio packets: where, how long, how many bytes."""
    result = sh(["ffprobe", "-v", "error", "-select_streams", "a:0",
                 "-show_entries", "packet=pts_time,duration,size", "-of", "csv=p=0", str(path)])
    return [line for line in result.stdout.decode(errors="replace").splitlines() if line.strip()]


def adpcm_fixture(work, name, encode, block, frames_per_block):
    """One second of 440 Hz at 8 kHz mono as ADPCM, its tables stating real packets.

    `block` and `frames_per_block` are left `None` for the coding whose muxer output is already
    honest, which then needs no authoring. Returns (path, declared frames, why)."""
    path = work / f"native-{name}.mov"
    base = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-strict", "-2", "-f", "lavfi", "-i",
            "sine=frequency=440:sample_rate=8000:duration=1", "-ac", "1"]
    result = sh(base + encode + [str(path)])
    if result.returncode:
        return None, 0, result.stderr.decode(errors="replace").strip()
    if block is None:
        mdhd = mov_find(mov_parse(path.read_bytes()), [b"moov", b"trak", b"mdia", b"mdhd"])
        return path, struct.unpack_from(">I", mdhd, 16)[0], ""
    muxed = mov_packets(path)
    try:
        declared = mov_packetize(path, block, frames_per_block)
    except ValueError as why:
        return None, 0, str(why)
    # The check that makes the rewrite a fixture and not a fudge: a reference reader has to see the
    # same packets in it as it saw in the muxer's own file, tables it derived its way.
    if mov_packets(path) != muxed:
        return None, 0, (f"authored tables changed what ffprobe sees: "
                         f"{muxed[:2]} -> {mov_packets(path)[:2]}")
    return path, declared, ""


# How each ADPCM row's fixture has to be built: the block length and frames per block the authored
# tables state, or `None` for the coding whose muxer output already states them. The numbers come
# from the coding, not from the file: these are 4-bit codes, so a 1024-byte block holds 2041 frames
# of IMA once its 4-byte header is off and 2036 of Microsoft's once its 7-byte one is, which is
# also what `ffprobe` counts on both.
ADPCM_GEOMETRY = {"adpcm": (1024, 2041), "adpcm_ms": (1024, 2036), "adpcm_qt": (None, None)}


# --- AVI fixtures -------------------------------------------------------------------------------
#
# An AVI needs no authored tables for the two Microsoft spellings: its muxer indexes an ADPCM run by
# block, which is exactly what the format states, so the file ffmpeg writes is the shape the
# specification describes and the QuickTime rewriting above does not apply. What the container does
# state unusually is one block's duration as a *reduced* ratio - 2 036 samples at 8 kHz is written
# 509 over 2 000 - so the frames a track holds come out of a product, not out of `dwScale`. The
# gate works that product out from the header's own bytes, in Python, so the count the reader has
# to reach is not the reader's arithmetic.

# The Wave format number each of these codings carries in a `strf` record, as the registry numbers
# it. Checked against the muxer's own output, so the fixture is shown to hold the coding the row
# names before anything this repository wrote is believed about it.
AVI_WAVE_TAG = {"adpcm_ima_wav": 0x0011, "adpcm_ms": 0x0002}


def riff_walk(data, lo, hi, depth, out):
    """Every chunk of a RIFF tree, lists descended into, as (tag, body start, body end).

    A chunk that promises more than the buffer holds is taken as far as the buffer goes, and an
    odd-length one is followed by a pad byte, which is how the walk stays on header boundaries.
    """
    at = lo
    while at + 8 <= hi:
        tag = data[at:at + 4]
        size = int.from_bytes(data[at + 4:at + 8], "little")
        body, end = at + 8, min(at + 8 + size, hi)
        if depth < 8 and tag in (b"RIFF", b"LIST"):
            # Four bytes of a RIFF or LIST body are the form type, not a chunk header.
            riff_walk(data, body + 4, end, depth + 1, out)
        else:
            out.append((tag, body, end))
        at = body + size + (size & 1)
    return out


def avi_audio_header(data):
    """The first audio stream's timeline and geometry, read out of its two header records.

    Raises `ValueError` for a file that does not state them, which the gate reports as a fixture
    that was not written rather than as a decode that failed.
    """
    chunks = riff_walk(data, 12, len(data), 0, [])
    for index, (tag, body, end) in enumerate(chunks):
        if tag != b"strh" or data[body:body + 4] != b"auds" or end - body < 36:
            continue
        fmt = next((c for c in chunks[index + 1:] if c[0] in (b"strf", b"strh")), None)
        if fmt is None or fmt[0] != b"strf" or fmt[2] - fmt[1] < 16:
            raise ValueError("an AVI audio stream header with no format record to follow it")
        scale, rate, _start, length = struct.unpack_from("<4I", data, body + 20)
        return {"tag": struct.unpack_from("<H", data, fmt[1])[0],
                "sample_rate": struct.unpack_from("<I", data, fmt[1] + 4)[0],
                "scale": scale, "rate": rate, "length": length}
    raise ValueError("an AVI with no auds stream header")


def avi_fixture(work, name, encode, wave_tag):
    """One second of 440 Hz at 8 kHz mono as ADPCM in an AVI, and the frames its header states.

    Returns (path, declared frames, why), with the same shape `adpcm_fixture()` gives back.
    """
    path = work / f"native-{name}.avi"
    base = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-strict", "-2", "-f", "lavfi", "-i",
            "sine=frequency=440:sample_rate=8000:duration=1", "-ac", "1"]
    result = sh(base + encode + [str(path)])
    if result.returncode:
        return None, 0, result.stderr.decode(errors="replace").strip()
    try:
        head = avi_audio_header(path.read_bytes())
    except ValueError as why:
        path.unlink(missing_ok=True)
        return None, 0, str(why)
    if head["tag"] != wave_tag:
        path.unlink(missing_ok=True)
        return None, 0, f"the muxer wrote Wave format {head['tag']:#06x}, not {wave_tag:#06x}"
    if head["length"] == 0:
        path.unlink(missing_ok=True)
        return None, 0, "the stream header counts no records"
    samples = head["scale"] * head["sample_rate"]
    if samples % head["rate"]:
        path.unlink(missing_ok=True)
        return None, 0, (f"one record's {head['scale']} over {head['rate']} seconds is no whole "
                         f"number of {head['sample_rate']} Hz samples")
    return path, head["length"] * (samples // head["rate"]), ""


# Formats with no encoder in this build and no take in either repository, but whose file is a
# layout the script can write to spec itself. The gate asks the same two things of these as of any
# fixture: the file has to probe back as the row's own descriptor, and `fvid media decode` has to
# return the frame count ffprobe counts on it. `ffmpeg -xerror` reads both clean, so they prove the
# decode path rather than error handling.
HAND = {
    # XPM is a C array of strings: dimensions, palette, then one quoted entry per image line.
    "xpm": lambda: (b"/* XPM */\nstatic char * fvid[] = {\n"
                    b'"4 3 2 1",\n". c #FFFFFF",\n"# c #000000",\n'
                    b'"....",\n"####",\n"...#"};\n'),
    # GEM raster: two bytes skipped, then seven big-endian u16s — header length in 2-byte words,
    # colour planes, pattern block size, sample aspect, width, height — and the planar RLE run
    # starting at header*2. This one is 8x2 at 1 bit per pixel: a copy opcode (0x80), a count, then
    # two row bytes. The format has no file signature, which is what `UNREACHABLE` measures.
    "gem": lambda: (b"\x00\x00\x00\x08\x00\x01\x00\x08\x00\x01\x00\x01\x00\x08\x00\x02"
                    + bytes([0x80, 0x02, 0xAA, 0x55])),
    # Bitmap Brothers JV: "JV", two pad bytes, the 75-byte signature string, one pad, then width,
    # height, frame count, a pts numerator and the audio rate (ten bytes of padding before the
    # 16-byte frame records). Video type 2 is one fill byte; the palette is 256 RGB triples written
    # straight after it, which is also what the frame record's palette byte (768) announces. Unlike
    # GEM this format probes by signature, so it is measured for a different reason.
    "jv": lambda: (b"JV\x00\x00" + JV_SIGNATURE + b"\x00"
                   + (64).to_bytes(2, "little") + (64).to_bytes(2, "little")
                   + (1).to_bytes(2, "little") + (40).to_bytes(2, "little") + b"\x00" * 4
                   + (8000).to_bytes(2, "little") + b"\x00" * 10
                   + (769).to_bytes(4, "little") + (0).to_bytes(4, "little")
                   + (1).to_bytes(4, "little") + bytes([1, 0, 2, 0])
                   + b"\x40" + b"".join(bytes([i, 255 - i, (i * 2) & 0xFF]) for i in range(256))),
}

# The signature FFmpeg's `jv` demuxer probes for; the two leading bytes are checked separately.
JV_SIGNATURE = (b" Compression by John M Phillips Copyright (C) 1995 "
                b"The Bitmap Brothers Ltd.")


# A hand-written file whose bytes name no format at all: the reference reads it only through the
# demuxer named here, and so does fvid, through `media decode --input-format`. The row is measured
# both ways — the named read has to give the frames and the unnamed one has to refuse — so the
# hint stays a requirement of the format rather than a convenience of the harness.
HINTED = {"gem": "gem_pipe"}

# A format whose own demuxer answers the read after its last frame with AVERROR_INVALIDDATA instead
# of EOF, so the reference fails the `-xerror` bar on a file as whole as the specification can write
# it. For these names that bar is replaced, not dropped, by two checks the strict bar cannot stand
# in for: the reference has to pull every frame the file declares once the ceiling is lowered and
# exit clean, and fvid's read has to report the damaged tail it skipped.
TOLERATED_EOF = {"jv"}


def frame_count(path, demuxer=None):
    """How many frames the reference itself counts on a file, naming the demuxer if the format needs
    it. This is the strict count: frames the decoder accepts, not frames a lenient read reports."""
    named = ["-f", demuxer] if demuxer else []
    out = sh(["ffprobe", "-v", "error", *named, "-select_streams", "v:0", "-count_frames",
              "-show_entries", "stream=nb_read_frames", "-of", "default=nw=1", str(path)])
    found = re.search(r"nb_read_frames=(\d+)", out.stdout.decode(errors="replace"))
    return int(found.group(1)) if found else None


# FFmpeg prints the address of the context it complains from, and that address changes every run.
# Kept in a measurement it would rewrite the deliverable's matrix on an idempotent run.
CONTEXT_ADDRESS = re.compile(r" @ 0x[0-9a-f]+")


def last_line(text, limit=90):
    """The line a tool actually failed on — the ones before it are boilerplate."""
    line = (text.strip().splitlines() or [""])[-1]
    return CONTEXT_ADDRESS.sub("", line)[:limit]


def image_fixture(work, name, ext, encode, frames):
    """One still (or a short animated run) written straight to the format's own file.

    A hand fixture has to clear the same bar an external take clears: the reference reads it clean,
    or the file would prove error handling instead of decoding. Formats the reference cannot read
    at all are measured in `measure_unreachable`, which asks that question on purpose — and for the
    few in `TOLERATED_EOF` the bar is the one written there, because the reference cannot clear it on
    a whole file of their own making."""
    path = work / f"img-{name}.{ext}"
    if name in HAND:
        path.write_bytes(HAND[name]())
        # The same bar an external video take has to clear: if the reference implementation cannot
        # read the file clean, the fixture proves error handling instead of decoding.
        named = ["-f", HINTED[name]] if name in HINTED else []
        check = sh(["ffmpeg", "-nostdin", "-v", "error", "-xerror", *named, "-i", str(path),
                    "-f", "null", "-"])
        if check.returncode == 0:
            if name in TOLERATED_EOF:
                return None, (f"{name} reads clean under -xerror now, so the end-of-stream tolerance "
                              "this fixture is admitted by has to be dropped")
            return path, ""
        if name not in TOLERATED_EOF:
            path.unlink(missing_ok=True)
            return None, check.stderr.decode(errors="replace").strip()
        # The replaced bar. Without the ceiling the same read has to deliver every frame the file
        # declares and still exit clean: that says the bytes are whole and only the demuxer's last
        # word is wrong, which is the one thing a fixture may not be excused for.
        lenient = sh(["ffmpeg", "-nostdin", "-v", "info", *named, "-i", str(path),
                      "-f", "null", "-"])
        counted = re.findall(rb"frame=\s*(\d+)", lenient.stderr)
        pulled = int(counted[-1]) if counted else None
        declared = frame_count(path, HINTED.get(name))
        if lenient.returncode or pulled != frames or declared != frames:
            path.unlink(missing_ok=True)
            return None, (f"{name}: without -xerror the reference gets {pulled} frames and exits "
                          f"{lenient.returncode}, ffprobe counts {declared}, not the {frames} a row "
                          "has to prove")
        return path, ""
    base = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-strict", "-2", "-f", "lavfi", "-i",
            "testsrc=size=64x48:rate=25:duration=0.2"]
    args = list(encode) + (["-frames:v", str(frames)] if "-frames:v" not in encode else [])
    result = sh(base + args + [str(path)])
    if result.returncode:
        return None, result.stderr.decode(errors="replace").strip()
    return path, ""


# --- Wave fixtures that pack several samples into one block --------------------------------------
#
# Some Wave codings hold more than one frame in a block, and for those the header's derived fields
# stop being evidence of a run's length. G.722 codes four bits a sample, so a block is one byte per
# channel standing for two frames, and measured over files this build's muxer wrote at 8, 16 and
# 44.1 kHz all three state 16 000 in `dwAvgBytesPerSec` while the runs go at 4 000, 8 000 and 22 050
# bytes a second. G.726 codes two to five bits a sample, so its block is one to five bytes wide and
# holds two to eight frames, and it is the byte rate rather than the bit-depth field that states the
# width - measured, patching that field to anything from 0 to 64 leaves the reference's output
# unchanged. Both readers take the frame count from the `data` chunk and cross-check it against
# `fact`; the gate works the same count out here, in Python, so the number the native path has to
# reach is not the reader's own arithmetic.
#
# A value of `None` for the frames per block says the width is not fixed by the coding but stated by
# the file, which is then checked to divide its own block a whole number of times.

NATIVE_PACKED_WAV = {"g722": (0x028F, 2), "g726": (0x0045, None)}


def wav_coded_frames(path, wave_tag, samples_per_block, block_bytes=None, require_fact=True,
                     check_fact=True):
    """The frames a Wave run codes, out of the file's chunks, for a coding that packs samples.

    `block_bytes` is the width such a block has to be where the coding states it itself rather than
    deriving it from a channel count - GSM 06.10's 65-byte frame pair. Raises `ValueError` for a
    fixture that is not that coding, misstates its block, or has no `fact` for the run to be checked
    against - a fixture that cannot be counted independently proves nothing. `require_fact` is off
    only for a coding whose run *is* its byte count, where the count below comes from the `data`
    chunk's own payload and a `fact` chunk is a cross-check if the file bothers to carry one.
    `check_fact` is off where the field states something other than the run's frames - TrueSpeech's
    own writer puts a count ten-odd frames short of what its blocks code, over a run the reference
    plays whole - so the count below is worked out from the bytes and the claim is left alone.
    """
    data = path.read_bytes()
    chunks = riff_walk(data, 12, len(data), 0, [])
    fmt = next((c for c in chunks if c[0] == b"fmt "), None)
    if fmt is None or fmt[2] - fmt[1] < 16:
        raise ValueError("a Wave file with no fmt chunk to read the geometry from")
    tag, channels, rate, byte_rate, block, bits = struct.unpack_from("<HHIIHH", data, fmt[1])
    if tag != wave_tag:
        raise ValueError(f"the fixture's fmt chunk states {tag:#06x}, not the {wave_tag:#06x} the "
                         "row names")
    if samples_per_block is None:
        bits_per_frame = byte_rate * 8 / (rate * channels)
        if block == 0 or not bits_per_frame.is_integer() or not (2 <= bits_per_frame <= 5) \
                or (block * 8) % int(bits_per_frame):
            raise ValueError(f"{byte_rate} bytes a second at {rate} Hz over {channels} channel(s) "
                             f"codes no whole width inside a {block}-byte block")
        samples_per_block = block * 8 // int(bits_per_frame)
    elif block != (block_bytes if block_bytes is not None else channels) or block == 0:
        if block_bytes is None:
            raise ValueError(f"{channels} channel(s) state a {block}-byte block; {bits} bits a "
                             f"sample packed {samples_per_block} to a byte per channel needs one "
                             "a channel wide")
        raise ValueError(f"the fixture states a {block}-byte block where this coding's own is "
                         f"{block_bytes} bytes")
    run = next((c for c in chunks if c[0] == b"data"), None)
    if run is None:
        raise ValueError("a Wave file with no data chunk")
    frames = ((run[2] - run[1]) // block) * samples_per_block
    fact = next((c for c in chunks if c[0] == b"fact"), None)
    if fact is None or fact[2] - fact[1] < 4:
        if require_fact:
            raise ValueError("no fact chunk, so nothing but this count states the run's length")
    else:
        stated = int.from_bytes(data[fact[1]:fact[1] + 4], "little")
        if check_fact and stated != frames:
            raise ValueError(f"the fact chunk states {stated} frames where the run codes {frames}")
    return frames


def compare_samples(probe, path, work, tag):
    """Whether fvid's own dispatch and libavcodec give the same numbers for one file.

    These codings are integer from end to end, so the bar is the reference's exact sample values and
    not merely a matching frame count: a predictor, a step adaptation or a filter tap that goes wrong
    moves a sample, and a count would not show it. Returns ("", a count) when every sample agrees,
    and a message naming the first difference otherwise.
    """
    pcm = work / f"bitexact-{tag}.f32"
    result = sh([str(probe), "--pcm", str(pcm), str(path)], timeout=180)
    if result.returncode:
        detail = result.stderr.decode(errors="replace").strip() or "nothing printed"
        return f"the native path refused its own fixture ({detail[:80]})", ""
    got = struct.unpack(f"<{pcm.stat().st_size // 4}f", pcm.read_bytes())
    out = sh(["ffmpeg", "-nostdin", "-v", "error", "-i", str(path), "-f", "s16le", "-"], timeout=180)
    if out.returncode:
        return ("the reference could not decode the fixture ("
                + out.stderr.decode(errors="replace").strip()[-80:] + ")"), ""
    want = [sample / 32768.0 for sample in struct.unpack(f"<{len(out.stdout) // 2}h", out.stdout)]
    if len(got) != len(want):
        return f"the native path gives {len(got)} samples where libavcodec gives {len(want)}", ""
    first = next((i for i, pair in enumerate(zip(got, want)) if pair[0] != pair[1]), None)
    if first is not None:
        return (f"sample {first} of {len(want)} is {got[first]} where libavcodec gives "
                f"{want[first]}"), ""
    return "", f"{len(got)} samples alike"


NATIVE = {"h264", "hevc", "vp9", "av1", "aac", "vorbis"}
# Audio rows fvid's own player pipeline decodes without libavcodec: per row a list of proofs, each
# one the codec id as the container spells it, the container that carries that id, and how to mux
# the fixture. A row counts as native only when every proof in its list passes, because the row
# stands for the whole K-Lite line it absorbed: `pcm_qt` covers "TWOS, SOWT" — two byte orders — and
# `pcm` is muxed at the 24-bit width the row's own fixture uses, not at the 16-bit default.
# `measure_native()` runs these instead of trusting the list, which is why it stays short.
NATIVE_AUDIO = {
    "aac": [("mp4a", "m4a", ["-c:a", "aac"])],
    # the native Vorbis encoder is experimental and, like s302m, reads `-strict -2` only as an
    # output option, so the one the fixture helper passes globally does not reach it.
    "vorbis": [("A_VORBIS", "mkv", ["-c:a", "vorbis", "-strict", "-2", "-ac", "2"])],
    "mp3": [("A_MPEG/L3", "mkv", ["-c:a", "libmp3lame"])],
    "flac": [("A_FLAC", "mkv", ["-c:a", "flac"])],
    "pcm": [("A_PCM/INT/LIT", "mkv", ["-c:a", "pcm_s24le"])],
    # G.711's two tables, both proven on the container K-Lite's own row names them in: a Wave
    # file states them as a format number and one byte a sample, so each table is measured as
    # the row's reference descriptor (`pcm_alaw`) and as its sibling, which the same row covers.
    "g711": [("pcm_alaw", "wav", ["-c:a", "pcm_alaw", "-ar", "8000"]),
             ("pcm_mulaw", "wav", ["-c:a", "pcm_mulaw", "-ar", "8000"])],
    # G.722 in the container K-Lite's own row names it in. The Wave number the registry gives it,
    # four bits a sample, one byte per channel for two samples - which is why the fixture's frame
    # count is derived above rather than read off the header's byte rate.
    "g722": [("adpcm_g722", "wav", ["-c:a", "g722", "-ar", "16000"])],
    # G.726 at all four widths the format states. The row's own fixture is the 32 kbit/s one K-Lite's
    # line names, and the three siblings are the same coding at the widths its tables hold: a proof
    # of one of them would leave the table choice, and the code that straddles a byte at 24 and 40
    # kbit/s, untested.
    "g726": [("adpcm_g726", "wav", ["-c:a", "g726", "-b:a", "16k", "-ar", "8000"]),
             ("adpcm_g726", "wav", ["-c:a", "g726", "-b:a", "24k", "-ar", "8000"]),
             ("adpcm_g726", "wav", ["-c:a", "g726", "-b:a", "32k", "-ar", "8000"]),
             ("adpcm_g726", "wav", ["-c:a", "g726", "-b:a", "40k", "-ar", "8000"])],
    "pcm_qt": [("sowt", "mov", ["-c:a", "pcm_s16le"]), ("twos", "mov", ["-c:a", "pcm_s16be"])],
    # The three ADPCM spellings K-Lite's pack carries. The two Microsoft ones name themselves by the
    # WAVE format tag in their last byte and Apple's by its own fourcc, and their MOV fixtures are
    # built by `adpcm_fixture()` rather than `audio_fixture()`, for the reason the note above that
    # function gives. The first two also come in an AVI, which is the container K-Lite's own rows
    # name them in, and there the muxer's tables are already honest - so a row proves the reader for
    # the coding, not just the decoder, in both of the shapes the codec reaches a player through.
    "adpcm": [("adpcm_ima_wav", "mov", ["-c:a", "adpcm_ima_wav"]),
              ("adpcm_ima_wav", "avi", ["-c:a", "adpcm_ima_wav"])],
    "adpcm_ms": [("adpcm_ms", "mov", ["-c:a", "adpcm_ms"]),
                 ("adpcm_ms", "avi", ["-c:a", "adpcm_ms"])],
    "adpcm_qt": [("adpcm_ima_qt", "mov", ["-c:a", "adpcm_ima_qt"])],
    # Apple Lossless in both of the containers K-Lite's row reaches it through: the ISO BMFF entry
    # names the coding by its own fourcc and nests the cookie in an `alac` box, Matroska spells the
    # same fields into a `A_ALAC` track's `CodecPrivate`.
    "alac": [("alac", "m4a", ["-c:a", "alac"]), ("A_ALAC", "mkv", ["-c:a", "alac"])],
}

# On a lossless stream fvid's decoder and libavcodec must hand back the same sample count, which
# is what turns "the code ran" into "it decoded this stream".
# A companding table is one code in, one sample out: as with the lossless rows, the two decoders
# have to agree on where the run ends, not merely on having run.
NATIVE_LOSSLESS = {"flac", "pcm", "pcm_qt", "g711", "alac"}

# Rows whose native proof is the reference's sample values rather than its sample count: codings that
# stay integer end to end, where agreeing on the length of a run is a much weaker claim than
# agreeing on the run. The committed decoder tests compare whole files this way; the gate row says
# the same thing about the file the row's own fixture is.
NATIVE_BITEXACT = {"g722", "g726", "gsm_ms", "adpcm_ct", "truespeech", "mace"}

# Rows whose native proof cannot be a file this run muxes, because no encoder in this build produces
# the coding K-Lite names. For those the only honest fixture is a take the repository ships, so per
# row: the committed file, the codec id fvid's dispatch has to report for it, and the Wave geometry
# the file's own chunks have to state - format number, block width, frames per block - which is how
# the count the native path must reach gets worked out here in Python rather than taken from the
# reader's arithmetic. The fixture's sha256 is checked against the row's pin in
# `benchmarks/klite-samples.json`, the tie between the committed copy and the FATE file it came
# from, and a row listed above in `NATIVE_BITEXACT` is compared sample for sample here too.
NATIVE_SHIPPED = {
    "gsm_ms": ("tests/fixtures/gsm/ciao.wav", "gsm_ms", (0x0031, 65, 320)),
    "adpcm_ct": ("tests/fixtures/adpcm_ct/intro-partial.wav", "adpcm_ct", (0x0200, 1, 2)),
    # TrueSpeech in the only container K-Lite's row names for it. The block is the coding's own
    # 32 bytes and states nothing about the channels, and the take's header is the format's writer:
    # a depth of one bit and a byte rate of 1 067 off a run that costs 1 066.6.
    "truespeech": ("tests/fixtures/truespeech/a6.wav", "truespeech", (0x0022, 32, 240)),
}

# Shipped rows whose fixture states its length by nothing but the bytes the run holds. Creative
# ADPCM has no block to align to and the take the reference ships carries no `fact` chunk - measured
# over it, the same 262 096 bytes give the same 524 192 samples whichever `wBlockAlign`,
# `dwAvgBytesPerSec` or `fact` the header states, so the byte count is the file's only honest claim
# and the one both readers use. The count is still worked out here from the chunks, not taken from
# the reader.
NATIVE_SHIPPED_UNFRAMED = {"adpcm_ct"}

# Shipped rows whose `fact` chunk is not the run's length. TrueSpeech's states 345 972 frames where
# its 1 442 whole blocks code 346 080, and measured over the take the reference plays every one of
# them - the field counts something other than the samples this coding decodes. The count the native
# path has to reach is still worked out here from the chunks, and the reader carries the claim
# separately as `declared_frames` beside the frames it plays.
NATIVE_SHIPPED_FACT_UNCHECKED = {"truespeech"}

# Shipped rows whose take is a QuickTime track rather than a Wave file, so there is no `fmt` and
# `data` pair to count and the run's length comes out of the track's own tables instead: the `mdhd`
# duration at the track's timescale, which the `stts` runs have to add up to, at the rate the sample
# description states. MACE is the coding whose tables say something other than what they look like -
# measured over both takes below, `stsz` states one byte per *frame*: 630 630 entries over a MAC3
# stereo run whose blocks are four bytes apiece and so cost 420 420 bytes, and 17 856 entries over a
# MAC6 mono run of 2 976. That byte total is not the run and is not used here; what both codings
# agree on is the block, six samples wide and two bytes a channel for 3-to-1 against one for 6-to-1,
# and a whole number of blocks is what the count below insists on.
NATIVE_SHIPPED_QT = {
    # One proof for each coding the row names, because K-Lite writes the row as "Apple MACE
    # (MAC3, MAC6)": the pinned take is 3-to-1 in stereo, the committed fixture is 6-to-1 mono. The
    # second digest is written here rather than in `benchmarks/klite-samples.json`, which holds one
    # take per row name.
    "mace": [
        ("benchmarks/data/klite/mace--mac3audio.mov", "mace3", "MAC3", None),
        ("tests/fixtures/mace/mjpega.mov", "mace6", "MAC6",
         "58cd4648de82efa347c4c0d09a63ebdca351967a6112ac8c7b47af3a99e6f678"),
    ],
}

# Bytes a MACE block holds per channel, and the samples every MACE block codes however wide it is.
MACE_BLOCK_BYTES = {b"MAC3": 2, b"MAC6": 1}
MACE_SAMPLES_PER_BLOCK = 6


def mace_qt_frames(path, tag):
    """The frames a MACE sound track declares, worked out of that track's own QuickTime tables.

    `tag` is the sample description the track has to carry, so a retagged file cannot borrow a
    proof. Raises `ValueError` for a track that is not that coding, tables that disagree about its
    length, a run that is not a whole number of blocks, or audio that costs more than the file holds.
    """
    tree = mov_parse(path.read_bytes())
    moov = next((body for typ, body in tree if typ == b"moov"), None)
    if moov is None:
        raise ValueError("a QuickTime file with no moov to read the tracks from")
    sound = None
    for typ, trak in moov:
        if typ != b"trak":
            continue
        mdia = next((body for t, body in trak if t == b"mdia"), None)
        hdlr = next((body for t, body in (mdia or []) if t == b"hdlr"), b"")
        minf = next((body for t, body in (mdia or []) if t == b"minf"), None)
        stbl = next((body for t, body in (minf or []) if t == b"stbl"), None)
        stsd = next((body for t, body in (stbl or []) if t == b"stsd"), b"")
        if hdlr[8:12] == b"soun" and stsd[12:16] == tag.encode():
            sound = (mdia, stbl, stsd)
    if sound is None:
        raise ValueError(f"no sound track described as {tag}")
    mdia, stbl, stsd = sound
    mdhd = next((body for t, body in mdia if t == b"mdhd"), b"")
    if mdhd[:1] != b"\x00":
        raise ValueError(f"a version {mdhd[0]} mdhd, whose 64-bit fields this count does not read")
    timescale, duration = struct.unpack_from(">II", mdhd, 12)
    channels = struct.unpack_from(">H", stsd, 32)[0]
    rate = struct.unpack_from(">I", stsd, 40)[0] >> 16
    stts = next((body for t, body in stbl if t == b"stts"), b"")
    ticks = sum(struct.unpack_from(">I", stts, 8 + i * 8)[0]
                for i in range(struct.unpack_from(">I", stts, 4)[0]))
    if timescale != rate:
        raise ValueError(f"the track runs at {timescale} a second while its sample description "
                         f"states {rate}")
    if duration != ticks:
        raise ValueError(f"mdhd duration {duration} where the stts runs add to {ticks}")
    if duration % MACE_SAMPLES_PER_BLOCK:
        raise ValueError(f"{duration} frames is not a whole number of "
                         f"{MACE_SAMPLES_PER_BLOCK}-sample blocks")
    blocks = duration // MACE_SAMPLES_PER_BLOCK
    cost = blocks * MACE_BLOCK_BYTES[tag.encode()] * channels
    held = sum(len(body) for typ, body in tree if typ == b"mdat")
    if cost > held:
        raise ValueError(f"{blocks} blocks of {MACE_BLOCK_BYTES[tag.encode()] * channels} bytes "
                         f"cost {cost}, more than the {held} mdat bytes the file holds")
    return duration


NATIVE_PROBE = ROOT / "target-player" / "release" / "examples" / "audio_probe"
NATIVE_VIDEO_PROBE = ROOT / "target-player" / "release" / "examples" / "decode_native_rgb"
MCP_BINARY = ROOT / "target-mcp" / "release" / "fvid"


def parse_probe(text):
    """`key=value` lines printed by examples/audio_probe."""
    stats = {}
    for part in text.replace("\n", " ").split():
        if "=" in part:
            key, value = part.split("=", 1)
            stats[key] = value
    return stats


def measure_native(rows, probe, work):
    """Prove the matrix's own-decoder column by running fvid's audio dispatch, libavcodec aside.

    `examples/audio_probe` calls `fvid::codec::make_audio_decoder` directly, so a row counted here
    is decoded by code in this repository. Kept by hand the column drifts the way the
    `absent-in-build` exemption did: a decoder wired up later stays invisible, and one dropped from
    the dispatch keeps claiming coverage.
    """
    if not pathlib.Path(probe).exists():
        return [f"no native probe at {probe}; build it with `cargo build --release " \
                f"--features player --example audio_probe`"]
    by_name = {row["codec"]: row for row in rows if row["kind"] == "audio"}
    problems = []
    for name, proofs in NATIVE_AUDIO.items():
        row = by_name.get(name)
        if row is None:
            problems.append(f"{name}: listed here but the matrix has no such audio row")
            continue
        passed = []
        for index, (codec_id, container, encode) in enumerate(proofs):
            declared = 0
            if container == "avi":
                tag = AVI_WAVE_TAG.get(codec_id)
                if tag is None:
                    problems.append(f"{name}: {codec_id} in an AVI states no Wave format number "
                                    "here")
                    continue
                path, declared, why = avi_fixture(work, f"{name}-{index}", encode, tag)
            elif name in ADPCM_GEOMETRY:
                block, per_block = ADPCM_GEOMETRY[name]
                path, declared, why = adpcm_fixture(work, f"{name}-{index}", encode,
                                                   block, per_block)
            else:
                path, why = audio_fixture(work, f"native-{name}-{index}", container, encode)
                if path is not None and name in NATIVE_PACKED_WAV:
                    tag, per_block = NATIVE_PACKED_WAV[name]
                    try:
                        declared = wav_coded_frames(path, tag, per_block)
                    except ValueError as error:
                        problems.append(f"{name}: {error}")
                        continue
            if path is None:
                problems.append(f"{name}: no native fixture ({why[:80]})")
                continue
            result = sh([str(probe), str(path)], timeout=180)
            stats = parse_probe(result.stdout.decode(errors="replace"))
            detail = (result.stderr.decode(errors="replace").strip()
                      or result.stdout.decode(errors="replace").strip())
            if result.returncode or not stats.get("frames"):
                problems.append(f"{name}: the native path refused {codec_id} ({detail[:80]})")
            elif stats.get("codec") != codec_id:
                problems.append(f"{name}: dispatch reports {stats.get('codec')}, expected {codec_id}")
            elif stats.get("decoded") != stats.get("packets"):
                problems.append(f"{name}: native decoded {stats['decoded']} of "
                                f"{stats['packets']} packets")
            elif declared and int(stats["frames"]) != declared:
                # A block-coded track ends where its declared length says, not where the encoder's
                # padding to the next block does.
                problems.append(f"{name}: native gives {stats['frames']} frames where the "
                                f"track declares {declared}")
            elif name in NATIVE_LOSSLESS and row.get("sample_frames") not in (None,
                                                                              int(stats["frames"])):
                problems.append(f"{name}: native gives {stats['frames']} samples where "
                                f"libavcodec gives {row['sample_frames']}")
            else:
                if name in NATIVE_BITEXACT:
                    differs, counts = compare_samples(probe, path, work, f"{name}-{index}")
                    if differs:
                        problems.append(f"{name}: {differs}")
                        continue
                    row["native_bitexact"] = counts
                passed.append((codec_id, container, int(stats["frames"])))
        if passed:
            # Half a row's proofs is not the row: `pcm_qt` stands for both byte orders K-Lite names
            # in one line, so a row that only proved one of them prints as unproven.
            # A row's proofs can name the same coding in two containers, as the two ADPCM rows do;
            # then the dispatch has one name to report and the variants list says where it worked.
            row["native_codec"] = " ".join(dict.fromkeys(codec for codec, _, _ in passed))
            row["native_frames"] = passed[0][2]
            row["native_variants"] = [f"{codec}@{container}={frames}"
                                      for codec, container, frames in passed]
            if len(passed) == len(proofs):
                row["native"], row["native_proof"] = True, "probe"
    for name, (fixture, codec_id, (tag, block, per_block)) in NATIVE_SHIPPED.items():
        row = by_name.get(name)
        if row is None:
            problems.append(f"{name}: listed here but the matrix has no such audio row")
            continue
        path = ROOT / fixture
        if not path.exists():
            problems.append(f"{name}: the shipped fixture {fixture} is not in the tree")
            continue
        pin = PINS.get(name)
        if pin is None:
            problems.append(f"{name}: no take is pinned for it, so {fixture} proves nothing")
            continue
        if hashlib.sha256(path.read_bytes()).hexdigest() != pin["sha256"]:
            problems.append(f"{name}: {fixture} is not the pinned {pin['file']}")
            continue
        try:
            declared = wav_coded_frames(path, tag, per_block, block,
                                        require_fact=name not in NATIVE_SHIPPED_UNFRAMED,
                                        check_fact=name not in NATIVE_SHIPPED_FACT_UNCHECKED)
        except ValueError as error:
            problems.append(f"{name}: {error}")
            continue
        result = sh([str(probe), str(path)], timeout=180)
        stats = parse_probe(result.stdout.decode(errors="replace"))
        detail = (result.stderr.decode(errors="replace").strip()
                  or result.stdout.decode(errors="replace").strip())
        if result.returncode or not stats.get("frames"):
            problems.append(f"{name}: the native path refused its own fixture ({detail[:80]})")
        elif stats.get("codec") != codec_id:
            problems.append(f"{name}: dispatch reports {stats.get('codec')}, expected {codec_id}")
        elif stats.get("decoded") != stats.get("packets"):
            problems.append(f"{name}: native decoded {stats['decoded']} of "
                            f"{stats['packets']} packets")
        elif int(stats["frames"]) != declared:
            problems.append(f"{name}: native gives {stats['frames']} frames where the run codes "
                            f"{declared}")
        else:
            if name in NATIVE_BITEXACT:
                differs, counts = compare_samples(probe, path, work, name)
                if differs:
                    problems.append(f"{name}: {differs}")
                    continue
                row["native_bitexact"] = counts
            row["native"], row["native_proof"] = True, "probe"
            row["native_codec"], row["native_frames"] = codec_id, declared
            row["native_variants"] = [f"{codec_id}@wav={declared}"]
    for name, proofs in NATIVE_SHIPPED_QT.items():
        row = by_name.get(name)
        if row is None:
            problems.append(f"{name}: listed here but the matrix has no such audio row")
            continue
        passed = []
        for fixture, codec_id, tag, digest in proofs:
            path = ROOT / fixture
            if not path.exists():
                problems.append(f"{name}: the shipped fixture {fixture} is not in the tree")
                continue
            pin = PINS.get(name)
            sha = digest or (pin or {}).get("sha256")
            if sha is None:
                problems.append(f"{name}: no take is pinned for it, so {fixture} proves nothing")
                continue
            if hashlib.sha256(path.read_bytes()).hexdigest() != sha:
                problems.append(f"{name}: {fixture} is not the take its digest pins")
                continue
            try:
                declared = mace_qt_frames(path, tag)
            except ValueError as error:
                problems.append(f"{name}: {error}")
                continue
            result = sh([str(probe), str(path)], timeout=180)
            stats = parse_probe(result.stdout.decode(errors="replace"))
            detail = (result.stderr.decode(errors="replace").strip()
                      or result.stdout.decode(errors="replace").strip())
            if result.returncode or not stats.get("frames"):
                problems.append(f"{name}: the native path refused its own fixture ({detail[:80]})")
            elif stats.get("codec") != codec_id:
                problems.append(f"{name}: dispatch reports {stats.get('codec')}, expected {codec_id}")
            elif stats.get("decoded") != stats.get("packets"):
                problems.append(f"{name}: native decoded {stats['decoded']} of "
                                f"{stats['packets']} packets")
            elif int(stats["frames"]) != declared:
                problems.append(f"{name}: native gives {stats['frames']} frames where the track "
                                f"declares {declared}")
            elif name in NATIVE_BITEXACT:
                differs, counts = compare_samples(probe, path, work, f"{name}-{tag}")
                if differs:
                    problems.append(f"{name}: {differs}")
                    continue
                row["native_bitexact"] = counts
                passed.append((codec_id, tag, declared))
            else:
                passed.append((codec_id, tag, declared))
        # Both codings the row names have to answer, the same bar `NATIVE_AUDIO` holds its proofs to.
        if len(passed) == len(proofs):
            row["native"], row["native_proof"] = True, "probe"
            row["native_codec"] = " ".join(codec for codec, _, _ in passed)
            row["native_frames"] = passed[0][2]
            row["native_variants"] = [f"{codec}@mov={frames}" for codec, _, frames in passed]
    for row in rows:
        if row.get("native") and row.get("native_proof") is None:
            if row["kind"] == "audio":
                problems.append(f"{row['codec']}: marked native but never run through the probe")
            row["native_proof"] = "tests"
    return problems

# Row name -> the FFmpeg codec descriptor ffprobe reports for it, only where the two differ:
# K-Lite names rows after the FourCC or the filter, not after libavcodec.
# The native video path is proven the same way, over the container fixtures this repository already
# ships for its own decoder tests: the frames fvid reconstructs must equal what ffprobe counts on
# that very file. H.264 has no committed container fixture, and its decoder refuses x264's default
# stream ("unexpected AVC profile extension"), so that one proof is generated at baseline/level 3.0.
NATIVE_VIDEO = {
    "hevc": "tests/fixtures/hevc/main-ipb.mp4",
    "vp9": "tests/fixtures/vp9/motion.webm",
    "av1": "tests/fixtures/av1/random-access.webm",
}
NATIVE_VIDEO_GENERATED = {
    "h264": ["-c:v", "libx264", "-preset", "ultrafast", "-g", "5",
             "-profile:v", "baseline", "-level", "3.0", "-pix_fmt", "yuv420p"],
}


def measure_native_video(rows, probe, work):
    """Run fvid's own video reconstruction headlessly, libavcodec aside.

    `examples/decode_native_rgb` drives `NativeReader` — the dispatch the GUI player uses — without
    a window, and fails if rewinding does not reproduce the first frame byte for byte.
    """
    if not pathlib.Path(probe).exists():
        return [f"no native video probe at {probe}; build it with `cargo build --release "
                f"--features player --example decode_native_rgb`"]
    by_name = {row["codec"]: row for row in rows if row["kind"] == "video"}
    problems, targets = [], []
    for name, rel in NATIVE_VIDEO.items():
        path = ROOT / rel
        if not path.exists():
            problems.append(f"{name}: committed native fixture {rel} is gone")
            continue
        targets.append((name, path))
    for name, encode in NATIVE_VIDEO_GENERATED.items():
        path, why = fixture(work, f"native-{name}", "mp4", encode)
        if path is None:
            problems.append(f"{name}: no native fixture ({why[:80]})")
            continue
        targets.append((name, path))
    for name, path in targets:
        row = by_name.get(name)
        if row is None:
            problems.append(f"{name}: listed here but the matrix has no such video row")
            continue
        out = work / f"native-{name}.rgb"
        out.unlink(missing_ok=True)
        result = sh([str(probe), str(path), str(out)], timeout=300)
        frames = parse_probe(result.stdout.decode(errors="replace")).get("frames")
        if result.returncode or not frames:
            detail = (result.stderr.decode(errors="replace").strip()
                      or result.stdout.decode(errors="replace").strip())
            problems.append(f"{name}: the native path refused {path.name} ({detail[:90]})")
            continue
        want = oracle_count(path, "v:0")
        row["native_frames"] = int(frames)
        row["native_oracle_frames"] = want
        if want is None or int(frames) != want:
            problems.append(f"{name}: native gives {frames} frames where ffprobe gives {want} "
                            f"on {path.name}")
        else:
            row["native"], row["native_proof"] = True, "probe"
    return problems


# Rows fvid's own pipeline carries while the linked FFmpeg has nothing to compare against at all:
# neither a demuxer nor a decoder, so no oracle exists for them. Each row is proven by a file this
# repository wrote from the format's own specification, where every count follows from how the file
# was built, and — where one was available — by a pinned external take on top. A take states no
# length the run can predict, so it is held to the invariants that hold whatever it is: every
# packet decoded, and the PCM one frame per tick its own stamps state.
#
# The fixtures are written by the same code that reads them, so their bytes are pinned here too:
# a change that moves the writer and the reader together keeps every count below true, and the
# digest is what turns that into a failure someone has to look at.
NATIVE_UNORACLE = {
    # `tests/fixtures/midi/two-notes.mid` is two quarter notes at 120 bpm over a 2 s track. Its
    # reader hands out 100 ms windows plus one more so a note held to the end is heard releasing:
    # 21 to cover the track and 1 after it, 22 packets of a tenth of the rate each. The notes sound
    # in 12 of those windows (0–5 and 10–15, each with its release tail), so 10 stay silent —
    # counted from the PCM the run writes, not taken on trust.
    "midi": {"codec": "midi", "file": "tests/fixtures/midi/two-notes.mid",
             "sha256": "55118571e7325c2415ee7d3e0c8cbeaea7fd27fffd7815b7bf745e975e58e880",
             "packets": 22, "sounding": 12},
    # `tests/fixtures/xm/two-notes.xm` is three rows of two channels at FastTracker II's own
    # defaults - six ticks a row at 125 a minute, which is 120 ms apiece - and the packet after
    # them, so a note still sounding at the end is heard letting go: 4 packets. Both notes are
    # struck on the first row and cut on the third, where the release takes the ramp's 48 frames,
    # so three of the tenths of a second its 480 ms span holds carry sound and two do not.
    "tracker": {"codec": "xm", "file": "tests/fixtures/xm/two-notes.xm",
                "sha256": "e37153032d0c8d9cb29cef522bb60218c6361e51842e3d52775ac01bd25c5771",
                "packets": 4, "sounding": 3,
                # Where a take for this row would be filed if the archive held one: the extensions
                # the formats write themselves, and the directory names a module corpus goes under.
                # Deliberately narrower than `.mod` and the word `module`, which the archive answers
                # with a JVC camera's DVR dump and an IP-module paper.
                "take": {"extensions": ["xm", "s3m", "it"],
                         "descriptors": ["fasttracker", "modplug", "tracker/", "/mod/"]}},
}


def window_peaks(pcm_path, channels, window):
    """The loudest f32 frame of each window of `window` frames, from interleaved PCM bytes."""
    data = pcm_path.read_bytes()
    count = len(data) // 4
    samples = struct.unpack(f"<{count}f", data)
    frames = count // channels
    peaks = []
    for start in range(0, frames, window):
        chunk = samples[start * channels:min(start + window, frames) * channels]
        peaks.append(max((abs(sample) for sample in chunk), default=0.0))
    return peaks


def own_decode(probe, path, work, tag):
    """Run fvid's own audio pipeline over one file and count what came out of it.

    The PCM goes to a file and comes back as numbers here: `decoded == packets` would be satisfied
    by a stream of silence, and silence is exactly what a wired-up-but-deaf decoder produces.
    """
    pcm = work / f"own-{tag}.f32"
    result = sh([str(probe), "--pcm", str(pcm), str(path)], timeout=180)
    stats = parse_probe(result.stdout.decode(errors="replace"))
    detail = (result.stderr.decode(errors="replace").strip()
              or result.stdout.decode(errors="replace").strip())
    if result.returncode or not stats.get("frames"):
        return None, detail[:90] or "nothing printed"
    window = int(stats["rate"]) // 10
    channels = int(stats["channels"])
    got = int(stats["frames"])
    out = {"codec": stats.get("codec"), "packets": int(stats["packets"]),
           "decoded": int(stats["decoded"]), "frames": got, "window": window,
           "rate": int(stats["rate"]),
           "seconds": float(stats["audio"].rstrip("s")),
           "stamped": float(stats["stamped"].rstrip("s")),
           "sounding": sum(1 for peak in window_peaks(pcm, channels, window) if peak > 0.0)}
    if pcm.stat().st_size != got * channels * 4:
        return None, (f"{pcm.stat().st_size} PCM bytes for {got} frames x {channels} channels, "
                      f"which is not 4 bytes a frame")
    return out, ""


def measure_own_pipeline(rows, probe, work):
    """Prove the matrix rows only fvid's own code can carry, and re-check the reference's refusal.

    `measure_native` proves a row by making fvid agree with libavcodec over one file. These rows
    have no libavcodec to agree with, so the counts come from the fixture's construction and from
    what the reader itself declares, and the refusal of the reference is measured again on every
    run: the day libavformat grows something that reads these files, the row stops being fvid-only
    and this gate says so rather than letting the sentence in the doc go stale.
    """
    if not pathlib.Path(probe).exists():
        return [f"no native probe at {probe}; build it with `cargo build --release " \
                f"--features player --example audio_probe`"]
    by_name = {row["codec"]: row for row in rows}
    # Read from the fetcher's cache only, like the image exemption does: a row that claims no
    # external take exists is claiming an absence, and an absence nobody searched is not a
    # measurement.
    index = sample_archive_index()
    problems = []
    for name, spec in NATIVE_UNORACLE.items():
        row = by_name.get(name)
        if row is None:
            problems.append(f"{name}: listed here but the matrix has no such row")
            continue
        taken = PINS.get(name)
        files = [("fixture", ROOT / spec["file"])] + (
            [] if not taken else [("take", SAMPLE_DIR / taken["file"])])
        clean = True
        record = {}
        for kind, path in files:
            if not path.exists():
                problems.append(f"{name}: {kind} {path.name} is not there")
                clean = False
                continue
            if kind == "fixture" and spec.get("sha256"):
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                if digest != spec["sha256"]:
                    problems.append(f"{name}: fixture {path.name} sha256 drifted, so the counts "
                                    f"below are no longer the ones its construction says")
                    clean = False
                    continue
            if not sh(["ffprobe", "-v", "error", "-show_format", path]).returncode:
                problems.append(f"{name}: ffprobe reads {path.name} now, so this is no longer a "
                                f"row only fvid carries and needs the ordinary proof")
                clean = False
            if kind == "take" and taken:
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                if digest != taken["sha256"]:
                    problems.append(f"{name}: pinned sample {path.name} sha256 drifted")
                    clean = False
                    continue
                # One window per tenth of a second is the reader's own claim about the timeline it
                # cut; a renderer that dropped or duplicated a window breaks it. How many of those
                # windows carry sound is a property of the take, so only the trivial silence is
                # refused here — the fixture above is where the exact count is checked.
                want = None
            else:
                want = spec["packets"]
            got, why = own_decode(probe, path, work, f"{name}-{kind}")
            if got is None:
                problems.append(f"{name}: the own pipeline refused {path.name} ({why})")
                clean = False
                continue
            if got["codec"] != spec["codec"]:
                problems.append(f"{name}: dispatch reports {got['codec']}, "
                                f"expected {spec['codec']}")
                clean = False
            elif got["decoded"] != got["packets"]:
                problems.append(f"{name}: the own pipeline decoded {got['decoded']} of "
                                f"{got['packets']} windows of {path.name}")
                clean = False
            elif want is not None and got["packets"] != want:
                problems.append(f"{name}: {path.name} gives {got['packets']} windows where its "
                                f"construction says {want}")
                clean = False
            # Every packet's frames are the whole frames of its own stamped length, so the
            # track can fall short of the timeline it states by less than a frame a packet -
            # and no more. This is the reader's claim about its own timeline, not a fixed
            # window size, so a row of any length is held to it.
            elif abs(got["frames"] - got["stamped"] * got["rate"]) > got["packets"]:
                problems.append(f"{name}: {got['frames']} frames are not the {got['stamped']:.3f} s "
                                f"of {got['packets']} packet stamps")
                clean = False
            elif want is not None and got["sounding"] != spec["sounding"]:
                problems.append(f"{name}: only {got['sounding']} of {got['packets']} windows carry "
                                f"sound where the note times say {spec['sounding']}")
                clean = False
            elif got["sounding"] == 0:
                problems.append(f"{name}: every window of {path.name} came out silent")
                clean = False
            else:
                record[kind] = got
        take_search = None
        if name not in PINS:
            needles = spec.get("take")
            if not needles:
                problems.append(f"{name}: no take is pinned, so the row has to say what a hunt "
                                f"for one looks in")
                clean = False
            elif index is None:
                problems.append(f"{name}: no take is pinned and the sample archive was not "
                                f"searched, so «there is none to pin» is not a measurement — run "
                                f"scripts/fetch_klite_samples.py")
                clean = False
            else:
                hits = sorted(path for path in index
                              if any(path.lower().endswith(f".{ext}")
                                     for ext in needles["extensions"])
                              or any(needle in path.lower() for needle in needles["descriptors"]))
                take_search = {"index": len(index), "candidates": len(hits), "hits": hits[:5]}
                if hits:
                    problems.append(f"{name}: the sample archive lists {len(hits)} take(s) for it "
                                    f"({', '.join(hits[:3])}), so the row owes one")
                    clean = False
        if clean:
            fixture = record["fixture"]
            row.update(native=True, native_proof="probe", native_codec=fixture["codec"],
                       native_frames=fixture["frames"], own_only=True,
                       own_packets=fixture["packets"], own_window=fixture["window"],
                       own_sounding=fixture["sounding"], own_seconds=fixture["seconds"],
                       own_take={key: entry for key, entry in record.items()
                                 if key != "fixture"} or None,
                       own_take_search=take_search,
                       file=str(ROOT / spec["file"]))
    return problems


# The desktop player runs fvid's own pipeline, not libavcodec: `NativeReader` for pictures
# (`src/playback_native.rs` dispatches on the file signature) and the Matroska/ISO-BMFF audio
# readers plus `make_audio_decoder` for sound. How far that reach goes is a measured question, so
# both playback examples are run over every row's own file and the answer lands in the matrix.
# Informational: no regression hangs on it, because widening the player is a product decision.
RGB_CAP = 64 << 20

# What a refusal means, by the error the pipeline itself printed. Anything that stops matching
# lands in `unclassified`, so a renamed message shows up as a number rather than as a wrong
# sentence in the doc.
SURFACE_CAUSES = (
    ("container", r"unrecognized video format|no reader recognised this envelope"),
    # A reader that understood the envelope and refused the coding it names: the container is
    # there, the decode arm is not. `audio_probe` prints the coding it read, so the two halves
    # of the queue are separate rows rather than one bucket.
    ("codec-unselected", r"no track in a codec|no supported VP9 or AV1"
                         r"|refused its audio coding|no decoder arm for it"),
    ("codec-limits", r"profile extension|picture tools|segmentation|requires coded"
                     r"|decoder init|sample data reference"),
    # `NativeReader` knows the PNG, GIF and JPEG signatures but hands back an error for a picture
    # with no timeline, so an image row can reach the image branch and still be refused.
    ("still-image", r"still-image playback is not supported"),
    # A pinned raw-bitstream sample can sit in a Wave header that declares plain PCM (the G.729
    # take in the archive does exactly that). The pipeline then plays the wrapper's claim rather
    # than the row's codec, so that is not the row being carried.
    ("header-names-another-codec", r"the pipeline reads this file as"),
)


CAUSE_NAMES = {
    "container": "нет демуксера",
    "codec-unselected": "контейнер прочитан, ветки раскодирования нет",
    "codec-limits": "нативный декодер отбраковал поток по своим границам",
    "still-image": "неподвижная картинка",
    "header-names-another-codec": "заголовок называет другой кодек",
    "unclassified": "сообщение не распознано",
}


def surface_cause_problems(rows):
    """A player refusal has to say which half of the pipeline is missing.

    One message for both made the deliverable's queue-of-work sentence a guess: it said the
    player lacked demuxers while some of the rows counted there were files whose envelope a
    reader opens and whose coding nothing decodes. The conflated wording is therefore a
    regression, and so is a refusal no cause matches."""
    problems = []
    for row in rows:
        if row.get("player") != "no":
            continue
        detail = row.get("player_detail") or ""
        if "no audio track this player can decode" in detail:
            problems.append(f"{row['codec']}: a refusal naming neither half ({detail})")
        elif row.get("player_cause") == "unclassified":
            problems.append(f"{row['codec']}: refusal no cause matches ({detail})")
    return problems


def run_capped(command, timeout):
    """Run a probe with RLIMIT_FSIZE set: a native path that accepts a long stream would write
    gigabytes of RGB before finishing, so it is cut off at the cap instead of filling the disk.
    Dying at the cap still answers the question asked here — the player can open the file."""

    def limit():
        resource.setrlimit(resource.RLIMIT_FSIZE, (RGB_CAP, RGB_CAP))

    try:
        return subprocess.run([str(part) for part in command], capture_output=True,
                              timeout=timeout, preexec_fn=limit)
    except subprocess.TimeoutExpired:
        return None


def measure_surfaces(rows, video_probe, audio_probe, work):
    """Ask fvid's playback pipeline, headless, which rows it can actually carry."""
    missing = [str(p) for p in (video_probe, audio_probe) if not pathlib.Path(p).exists()]
    if missing:
        return {"error": "no playback probes: " + ", ".join(missing)}
    tally = {"yes": 0, "no": 0, "not-run": 0}
    for row in rows:
        raw = row.get("file")
        path = pathlib.Path(raw) if raw else None
        # A row the reference cannot read has no fixture from that side, so its column usually goes
        # unmeasured. A row `measure_own_pipeline` proved is the exception: there the player has
        # something to open, and it is the only thing this row can be asked about.
        if (path is None or not path.exists()
                or (row["status"] != "covered" and not row.get("own_only"))):
            row["player"], tally["not-run"] = "not-run", tally["not-run"] + 1
            continue
        media = row.get("media") or ("a" if row["kind"] == "audio" else "v")
        mismatch = ""
        if media == "a":
            result, out = run_capped([audio_probe, path], 60), None
            stats = parse_probe(result.stdout.decode(errors="replace")) if result else {}
            got = int(stats.get("frames") or 0)
            full = stats.get("decoded") == stats.get("packets")
            # The pipeline names the tag it read the stream as. A PCM tag on a row
            # the reference calls by another codec means the file's own header is a
            # wrapper around a bitstream, and frames produced from it are the header's
            # claim being honoured, not this row being played.
            tag, described = str(stats.get("codec") or ""), str(row.get("descriptor") or "")
            # A container row states no codec of its own, so only a row the reference
            # does name can disagree with the header.
            if tag.startswith("A_PCM/") and described and not described.startswith("pcm"):
                mismatch = (f"the pipeline reads this file as {tag}, the reference as {described}")
        else:
            out = work / f"surface-{row['codec']}.rgb"
            out.unlink(missing_ok=True)
            result = run_capped([video_probe, path, out], 60)
            stats = parse_probe(result.stdout.decode(errors="replace")) if result else {}
            got, full = int(stats.get("frames") or 0), True
        if out is not None:
            out.unlink(missing_ok=True)
        if result is None:
            row["player"], row["player_detail"] = "no", "timeout"
        elif mismatch:
            row["player"], row["player_detail"] = "no", mismatch
        elif got and full:
            row["player"] = "yes"
        elif got:
            # the pipeline opened the stream and read frames, then stopped early — a size cap on
            # video, a packet the own decoder gave up on for audio. Either way the player reaches
            # this row; the detail says how far.
            row["player"], row["player_detail"] = "yes", (
                f"{got} frames, capped" if media == "v"
                else f"{stats.get('decoded')}/{stats.get('packets')} packets")
        else:
            row["player"], row["player_detail"] = "no", (
                result.stderr.decode(errors="replace").strip().splitlines() or [""])[-1][:90]
        tally[row["player"]] += 1
    tally["carried"] = sorted(row["codec"] for row in rows if row.get("player") == "yes")
    tally["refused"] = sorted(row["codec"] for row in rows if row.get("player") == "no")
    causes = {}
    for row in rows:
        if row.get("player") != "no":
            continue
        detail = row.get("player_detail") or ""
        cause = next((name for name, pattern in SURFACE_CAUSES if re.search(pattern, detail)),
                     "unclassified")
        row["player_cause"] = cause
        causes.setdefault(cause, []).append(row["codec"])
    # A count alone lets a whole axis hide inside one number: the image rows are 33 of the matrix,
    # and whether the player carries any of them is a question the tally has to answer by itself.
    tally["causes"] = {cause: len(names) for cause, names in sorted(causes.items())}
    tally["cause_rows"] = {cause: sorted(names) for cause, names in causes.items()}
    # Audio refusals are where the two halves of the queue are worth separating by hand: the
    # sound side is the one whose readers already exist, so a row can be missing only an arm.
    tally["audio_causes"] = {
        cause: sorted(row["codec"] for row in rows if row["kind"] == "audio"
                      and row.get("player_cause") == cause)
        for cause in sorted({row.get("player_cause") for row in rows
                             if row["kind"] == "audio" and row.get("player") == "no"})}
    tally["by_kind"] = {
        kind: {answer: sum(1 for row in rows if row["kind"] == kind
                           and row.get("player") == answer)
               for answer in ("yes", "no", "not-run")}
        for kind in ("video", "audio", "image", "container")}
    return tally


def standalone_allowlist():
    """The container policy the MCP surface runs under, read out of the code that applies it.

    `with_standalone_inputs` hands libavformat a `format_whitelist`, so this is the only place the
    boundary can be written down without turning it into a sentence nobody checks.
    """
    source = (ROOT / "crates/fvid-media/src/lib.rs").read_text()
    match = re.search(r'c"format_whitelist"\.as_ptr\(\),\s*c"([^"]+)"', source)
    if match is None:
        raise RuntimeError("crates/fvid-media/src/lib.rs sets no format_whitelist")
    return sorted(match.group(1).split(","))


class McpProbe:
    """Minimal JSON-RPC-over-stdio client: enough to drive one read-only tool per file."""

    def __init__(self, binary, root):
        self.log = tempfile.TemporaryFile()
        self.process = subprocess.Popen([str(binary), "mcp", "--root", str(root), "--jobs", "1",
                                        "--stdio"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.log)
        self.buffer = b""
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.requests = 0

    def _reply(self):
        deadline = time.monotonic() + 120
        while b"\n" not in self.buffer:
            if not self.selector.select(max(0, deadline - time.monotonic())):
                raise TimeoutError("the MCP server sent no response")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                self.log.seek(0)
                raise RuntimeError(self.log.read().decode(errors="replace")[-160:] or "MCP EOF")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)

    def call(self, method, params=None):
        self.requests += 1
        request = {"jsonrpc": "2.0", "id": self.requests, "method": method}
        if params is not None:
            request["params"] = params
        self.process.stdin.write((json.dumps(request) + "\n").encode())
        self.process.stdin.flush()
        return self._reply()

    def notify(self, method):
        self.process.stdin.write((json.dumps({"jsonrpc": "2.0", "method": method}) + "\n").encode())
        self.process.stdin.flush()

    def tool(self, name, arguments):
        reply = self.call("tools/call", {"name": name, "arguments": arguments})
        result = reply.get("result") or {}
        detail = " ".join(item.get("text", "") for item in result.get("content", []))
        if reply.get("error"):
            detail = json.dumps(reply["error"])
        return not reply.get("error") and not result.get("isError"), detail.strip()

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()


def measure_mcp(rows, binary, work):
    """Ask the MCP server itself which matrix files it may open, and why it refuses the rest.

    `fvid media` and MCP share libavcodec; what separates them is the standalone-input policy. Only
    the server can say where that line falls, and each answer is checked against the same file
    probed with no policy, so a refusal counts as policy only while the demuxer really is outside
    the allowlist. A row the allowlist names that MCP still refuses, or one it excludes that MCP
    opened, is a regression rather than a boundary.
    """
    binary = pathlib.Path(binary)
    if not binary.exists():
        return ({"error": f"no MCP binary at {binary}; build it with `cargo build --release "
                          f"--features mcp --target-dir target-mcp`"}, [])
    allow = set(standalone_allowlist())
    # One server per volume the fixtures live on: a server sees only its own --root, and the gate
    # writes neither of those directories, so no file is copied to be read.
    # Compared after resolving, because a `/var` -> `/private/var` prefix would otherwise read as
    # "under no root" and drop every generated fixture out of the column.
    roots = [root.resolve() for root in (ROOT, work)]
    tally = {"yes": 0, "no": 0, "not-run": 0, "allowlist": sorted(allow)}
    problems = []
    clients = {}
    try:
        for root in roots:
            client = McpProbe(binary, root)
            client.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                       "clientInfo": {"name": "klite-coverage-gate",
                                                      "version": "1"}})
            client.notify("notifications/initialized")
            clients[root] = client
    except (OSError, TimeoutError, ValueError) as error:
        for client in clients.values():
            client.close()
        return {"error": f"the MCP server would not serve: {str(error)[:160]}"}, []
    try:
        for row in rows:
            raw = row.get("file")
            # Resolved because the roots are compared by path, and a relative fixture name would
            # otherwise read as "under no root" and quietly drop out of the column.
            path = pathlib.Path(raw).resolve() if raw else None
            if path is None or not path.exists() or row["status"] != "covered":
                row["mcp"], tally["not-run"] = "not-run", tally["not-run"] + 1
                continue
            # `input()` canonicalises and demands the result stay under --root, which is why a
            # symlink out of the root is not a way to skip this lookup.
            root = next((candidate for candidate in roots if path_in(candidate, path)), None)
            if root is None:
                problems.append(f"{row['codec']}: its fixture is under no MCP root")
                row["mcp"], tally["no"] = "no", tally["no"] + 1
                continue
            opened, detail = clients[root].tool("fvid_probe",
                                               {"input": str(path.relative_to(root))})
            try:
                # The same file, read the way its own row reads it: a format whose bytes carry no
                # signature has no probe for the server to run either, so asking it unhinted would
                # record a refusal the row itself does not describe.
                hinted = (["--input-format", row["input_format"]]
                          if row.get("input_format") else [])
                demuxers = set(fvid("probe", path, *hinted)["format"].split(","))
            except (RuntimeError, json.JSONDecodeError) as error:
                problems.append(f"{row['codec']}: fvid media probe refuses its own fixture "
                                f"({str(error)[:80]})")
                demuxers = set()
            allowed = demuxers & allow
            row["mcp"] = "yes" if opened else "no"
            tally[row["mcp"]] += 1
            if opened and not allowed:
                problems.append(f"{row['codec']}: MCP opened {'/'.join(sorted(demuxers))}, which "
                                f"the standalone allowlist excludes")
            elif not opened and allowed:
                problems.append(f"{row['codec']}: MCP refused {'/'.join(sorted(allowed))}, a "
                                f"demuxer the allowlist names ({detail[:80]})")
            if not opened:
                row["mcp_detail"] = detail.splitlines()[-1][:90] if detail else ""
                # The demuxer the policy-free probe picked, so a refusal names a container rather
                # than an error code nobody can read.
                row["mcp_demuxer"] = "/".join(sorted(demuxers))
        # Row names repeat across the matrix (a video and an audio row share a label), so the lists
        # are deduplicated; the counts above stay per row.
        tally["carried"] = sorted({row["codec"] for row in rows if row.get("mcp") == "yes"})
        tally["refused"] = sorted({row["codec"] for row in rows if row.get("mcp") == "no"})
    except (OSError, TimeoutError, ValueError) as error:
        return {**tally, "error": f"the MCP session stopped: {str(error)[:160]}"}, problems
    finally:
        for client in clients.values():
            client.close()
    return tally, problems


def path_in(root, path):
    """Whether a file already sits under a directory, by path and without resolving symlinks."""
    try:
        path.relative_to(root)
        return True
    except ValueError:
        return False


DECODER = {
    "vc1": "vc1", "vvc": "vvc", "realvideo": "rv30", "lagarith": "lagarith",
    "eac3": "eac3", "ape": "ape", "atrac3": "atrac3", "speex": "speex", "ralf": "ralf",
    "optimfrog": "optimfrog", "amr": "amrnb", "rm": "cook", "g726": "g726", "shorten": "shorten",
    "vorbis": "vorbis", "opus": "opus", "ogv": "theora", "dv": "dvvideo",
    "camstudio": "cscd", "ultimotion": "ulti", "bink": "binkvideo", "smacker": "smackvideo",
    "mirillis": "fic", "icod": "aic", "nclc": "notchlc", "canopushq": "hq_hqa",
    "musepack": "musepack8", "g722": "adpcm_g722", "pcm": "pcm_s24le", "pcm_qt": "pcm_s16be",
    "adpcm_qt": "adpcm_ima_qt", "mace": "mace3", "dsd": "dsd_msbf", "cpng": "png",
    "mtv": "mjpeg", "dav": "h264", "midi": "midi", "i263": "h263i",
}

# Wrapper-only rows K-Lite claims but no fixture source carries: the demuxer name in
# `ffmpeg -formats` settles whether this build could open such a file at all.
EXT_DEMUXER = {"cda": "cda", "xm": "xm", "pva": "pva", "ivf": "ivf"}

# K-Lite labels that are not FFmpeg codec descriptors.
DECODE_ID = {
    "adpcm": "adpcm_ima_wav", "g711": "pcm_alaw", "mpeg1": "mpeg1video",
    "mpeg2": "mpeg2video", "mpeg4p2": "mpeg4",
}


def has_decoder(name):
    """Whether the linked libavcodec can decode the codec descriptor behind a matrix row."""
    return DECODER.get(name, DECODE_ID.get(name, name)) in DECODERS


def descriptor_of(path, stream, demuxer=None):
    """(codec descriptor, decodable in this build) for the first stream of a real file.

    Measured rather than looked up by row name: K-Lite labels formats by FourCC, and ffprobe
    reports the codec descriptor, which is what the decoder flag has to be about. A file the
    build cannot even probe reports (None, False), which is what `absent-in-build` means. A file
    that carries no signature is asked of the reference through the demuxer named here, since
    without it the reference reports nothing about a file it can read perfectly well.
    """
    named = ["-f", demuxer] if demuxer else []
    try:
        streams = json.loads(run_json(["ffprobe", "-v", "error", *named,
                                       "-select_streams", f"{stream}:0",
                                       "-show_entries", "stream=codec_name", "-of", "json",
                                       path]))["streams"]
    except Exception:
        return None, False
    name = streams[0]["codec_name"] if streams else None
    return name, name in DECODERS


def ffmpeg_decoders():
    """Codec names this build can decode: decoder names *and* the descriptors ffprobe reports.

    They differ more often than you would hope — the ALS decoder is `als` behind descriptor
    `mp4als`, CamStudio is `camstudio` behind `cscd` — and matching only one of the two makes a
    present decoder look absent.
    """
    names = set()
    for argument in ("-codecs", "-decoders"):
        text = subprocess.run(["ffmpeg", "-hide_banner", argument], capture_output=True).stdout.decode()
        for line in text.splitlines():
            parts = line.split()
            if len(parts) > 1 and (argument == "-decoders" or parts[0].startswith("D")):
                names.add(parts[1])
    return names


def decodable_descriptions():
    """What libavcodec calls the codecs this build decodes: descriptor -> human description."""
    text = subprocess.run(["ffmpeg", "-hide_banner", "-codecs"], capture_output=True).stdout.decode()
    found = {}
    for line in text.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0].startswith("D"):
            found[parts[1]] = re.sub(r"\((decoders|encoders):[^)]*\)", "", parts[2]).strip().lower()
    return found


def ffmpeg_formats():
    """Format names this build can demux (`ffmpeg -formats` lines flagged D).

    One demuxer publishes several names on a single line (`mov,mp4,m4a,3gp,3g2,mj2`), and a row
    carries whichever of them its file ends with, so the aliases are collected as well — dropping
    them makes a readable container look unsupported.
    """
    text = subprocess.run(["ffmpeg", "-hide_banner", "-formats"], capture_output=True).stdout.decode()
    found = set()
    for line in text.splitlines():
        if line.startswith(" D") and len(line.split()) > 1:
            found.update(line.split()[1].split(","))
    return found


def build_configuration():
    """What the linked FFmpeg reports about itself: version, and the flags it was configured with.

    `absent-in-build` means two different things in a stock full build and in one that disabled
    components on purpose — in the first no configuration brings the decoder back, in the second a
    rebuild would. The prose says "full build", so the flag list is measured rather than assumed.
    """
    conf = subprocess.run(["ffmpeg", "-hide_banner", "-buildconf"],
                          capture_output=True).stdout.decode()
    flags = [line.strip() for line in conf.splitlines() if line.strip().startswith("--")]
    version = subprocess.run(["ffmpeg", "-hide_banner", "-version"],
                             capture_output=True).stdout.decode()
    found = re.search(r"ffmpeg version (\S+)", version)
    return {"version": found.group(1) if found else "unknown",
            "disable_flags": sorted(flag for flag in flags if flag.startswith("--disable-"))}


DECODERS = ffmpeg_decoders()


def decodable_codecs():
    """Distinct codec descriptors this build decodes, without counting decoder aliases twice."""
    text = subprocess.run(["ffmpeg", "-hide_banner", "-codecs"], capture_output=True).stdout.decode()
    return len({line.split()[1] for line in text.splitlines()
                if len(line.split()) > 1 and line.split()[0].startswith("D")})
FORMATS = ffmpeg_formats()
BUILD = build_configuration()

# `absent-in-build` is the only status this gate can reach without measuring fvid at all: it says the
# linked libavcodec has nothing to compare against. That is a claim about two halves — the row's
# container and its decoder — and about who serves the format instead, so every row carrying the
# status has to name that reason here. A row without one, and a reason whose row is covered again,
# are both red (`absent-reasons`).
ABSENT_REASON = {
    "h264mvc": "нужен отдельный MVC-плагин LAV в Full-издании; H.264 MVC libavcodec не раскодировывает",
    "ac4": "Dolby-фильтр K-Lite; libavcodec читает и пишет сырой AC-4, но декодера у него нет",
    "optimfrog": "принятая явная отказка: в паке фильтр OptimFROG, декодера OptimFROG в FFmpeg нет",
    "midi": "K-Lite отдаёт MIDI компоненту Bass Audio Source; демуксера .mid в libavformat нет",
    "cda": "K-Lite отдаёт CD-DA компоненту Bass Audio Source; демуксера .cda в libavformat нет",
    "tracker": "K-Lite отдаёт it/xm/s3m/mod компоненту Bass Audio Source; модульных демуксеров "
               "в libavformat нет",
}
SAMPLE_DIR = ROOT / "benchmarks" / "data" / "klite"
PINS_PATH = ROOT / "benchmarks" / "klite-samples.json"
PINS = json.loads(PINS_PATH.read_text()) if PINS_PATH.exists() else {}


def oracle_count(path, stream):
    """How many frames the reference decoder reads; fvid must agree."""
    result = subprocess.run(["ffprobe", "-v", "error", "-select_streams", stream, "-count_frames",
                             "-show_entries", "stream=nb_read_frames", "-of", "default=nw=1:nk=1", str(path)],
                            capture_output=True, timeout=120)
    text = result.stdout.decode().strip()
    return int(text) if text.isdigit() else None


def audio_stream_index(path):
    streams = json.loads(run_json(["ffprobe", "-v", "error", "-show_entries", "stream=index,codec_type",
                                   "-of", "json", path]))["streams"]
    audio = [s["index"] for s in streams if s["codec_type"] == "audio"]
    return audio[0] if audio else None


def channel_hashes(path, channels):
    """Per-channel s16le hashes.

    Whole-stream hashes are not comparable: a decoded WAV without a written channel
    layout gets remapped against the source's 5.1(side), so only per-channel equality
    says something real about the decoder output.
    """
    hashes = []
    for index in range(channels):
        result = sh(["ffmpeg", "-nostdin", "-v", "error", "-i", path, "-map", "0:a:0",
                     "-af", f"pan=mono|c0=c{index}", "-f", "s16le", "-"])
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors="replace").strip()[-200:])
        hashes.append(hashlib.sha256(result.stdout).hexdigest())
    return hashes


def pcm_match(reference, produced, channels):
    return channel_hashes(reference, channels) == channel_hashes(produced, channels)


def remux_clean(path, work):
    """Rewrap a damaged reference sample without touching its bitstream.

    FATE carries a few legacy formats only as deliberately broken files (truncated index,
    corrupt-flagged packets). Those prove error handling, not the codec, and the video path is
    strict about damage by design. A lossless `-c copy` rewrap keeps every packet and lets the
    row measure decoding; the pin still records the original sha256.
    """
    clean = work / f"{path.stem}-clean{path.suffix}"
    result = sh(["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", path, "-map", "0", "-c", "copy", clean],
                timeout=180)
    return clean if not result.returncode and clean.exists() else None


def prove_with_sample(row, work):
    """A pinned FATE sample replaces the missing local encoder; returns True when handled."""
    pin = PINS.get(row["codec"])
    if not pin:
        return False
    path = SAMPLE_DIR / pin["file"]
    if not path.exists():
        row.update(status="needs-sample", fixture="sample-not-fetched")
        return True
    if hashlib.sha256(path.read_bytes()).hexdigest() != pin["sha256"]:
        row.update(status="fails", fixture="reference-sample", error="pinned sample sha256 drifted")
        return True
    row["fixture"] = ("reference-sample-cut" if pin.get("cut_frames") else
                      "reference-sample-retagged" if pin.get("retagged") else "reference-sample")
    row["native"] = row["codec"] in NATIVE
    media = row.get("media") or ("a" if row["kind"] == "audio" else "v")
    descriptor, decodable = descriptor_of(path, media)
    row["descriptor"], row["decoder"] = descriptor, decodable
    if not decodable:
        row.update(status="absent-in-build",
                   error=f"no FFmpeg decoder for {descriptor}" if descriptor
                   else "this build cannot even demux the sample")
        return True

    def attempt(target):
        if media == "a":
            out = work / f"{row['kind']}-{row['codec']}-sample-out.wav"
            stats = fvid("decode-audio", target, out, "--streams", str(audio_stream_index(target)))
            channels = stats.get("channels")
            row["decoded_frames"], row["sample_frames"] = stats.get("decoded_frames"), stats.get("sample_frames")
            row["decode_errors"] = stats.get("decode_errors")
            if not stats.get("sample_frames") or not channels:
                row["status"] = "fails"
            else:
                row["pcm_match"] = pcm_match(target, out, channels)
                row["status"] = "covered" if row["pcm_match"] else "fails"
        else:
            stats = fvid("decode", target)
            got, want = stats.get("video_frames"), oracle_count(target, "v:0")
            row["frames"], row["oracle_frames"] = got, want
            row["status"] = "covered" if got and want is not None and got == want else "fails"

    def prove(target):
        try:
            attempt(target)
            return None
        except Exception as error:
            return error

    used, error = path, prove(path)
    if row.get("status") != "covered":
        clean = remux_clean(path, work)
        if clean is not None:
            row["fixture"] = "reference-sample-remuxed"
            used, error = clean, prove(clean)
    if error is not None:
        row["status"] = status_for(error, used)
        row["error"] = str(error)[:200]
    row["file"] = str(used)
    return True


def classify(error):
    text = error.lower()
    if "unsupported" in text or "no decoder" in text or "unknown codec" in text or "not found" in text:
        return "missing"
    if "demuxer" in text or "probe" in text or "unknown format" in text or "invalid argument" in text:
        return "container-gap"
    return "fails"


def status_for(error, path=None):
    """A row fvid cannot read is only a regression when the linked FFmpeg reads it itself.

    If the CLI cannot even probe the file, the build has no decoder or demuxer for that
    format: K-Lite ships its own filter for it, and the row is reported as absent-in-build
    rather than silently counted as an fvid failure.
    """
    if path is not None and sh(["ffprobe", "-v", "error", "-show_format", path]).returncode:
        return "absent-in-build"
    return classify(str(error))


def copy_strictness(work):
    """Decode tolerates AV_PKT_FLAG_CORRUPT like the FFmpeg CLI; remux must not.

    Damaged bytes are fine to hand to a decoder, which decides what to salvage, but never
    to publish into a new file. The atrac3 reference sample carries a corrupt-flagged
    packet, so it pins both halves of that split.
    """
    pin = PINS.get("atrac3")
    path = SAMPLE_DIR / pin["file"] if pin else None
    if path is None or not path.exists():
        return "skipped"
    destination = work / "strictness-atrac3.wav"
    result = subprocess.run([str(BINARY), "media", "remux", str(path), str(destination)],
                            capture_output=True, timeout=90)
    if result.returncode == 0 or destination.exists():
        return "leaked"
    if b"corrupt packet rejected" not in result.stderr:
        return "wrong-error"
    return "rejected"


def run(work):
    rows = []
    for name, component, container, encode in VIDEO:
        row = {"codec": name, "component": component, "kind": "video", "container": container,
               "decoder": has_decoder(name)}
        if encode is None:
            row.update(status="needs-sample", fixture="decode-only")
            prove_with_sample(row, work)
            if row["status"] == "needs-sample" and not row["decoder"]:
                row.update(status="absent-in-build", error="no FFmpeg decoder for this row")
        else:
            path, why = fixture(work, name, container, encode)
            if path is None:
                row.update(status="no-fixture", fixture="encoder-unavailable", error=why[:200])
            else:
                row["fixture"] = "generated"
                row["file"] = str(path)
                row["descriptor"], row["decoder"] = descriptor_of(path, "v")
                try:
                    stats = fvid("decode", path)
                    row["frames"] = stats.get("video_frames")
                    row["width"] = stats.get("width")
                    row["native"] = name in NATIVE
                    # No fingerprint of these bytes: an encoder with automatic threading does not
                    # write the same file twice, so a hash here would be contradicted by the next
                    # run of this gate over the same covered row. What the fixture is stays
                    # recorded by the descriptor it probes back as and the frame count it gives.
                    row["status"] = "covered" if row["frames"] == FRAMES else "fails"
                except Exception as error:
                    row["status"] = status_for(error, path)
                    row["error"] = str(error)[:200]
        rows.append(row)
    for name, component, container, encode in AUDIO:
        row = {"codec": name, "component": component, "kind": "audio", "container": container,
               "decoder": has_decoder(name)}
        if encode is None:
            row.update(status="needs-sample", fixture="decode-only")
            prove_with_sample(row, work)
            if row["status"] == "needs-sample" and not row["decoder"]:
                row.update(status="absent-in-build", error="no FFmpeg decoder for this row")
        else:
            path, why = audio_fixture(work, name, container, encode)
            if path is None:
                row.update(status="no-fixture", fixture="encoder-unavailable", error=why[:200])
            else:
                row["fixture"] = "generated"
                row["file"] = str(path)
                row["descriptor"], row["decoder"] = descriptor_of(path, "a")
                try:
                    out = work / f"audio-{name}-out.wav"
                    stats = fvid("decode-audio", path, out)
                    row["sample_frames"] = stats.get("sample_frames")
                    row["rate"] = stats.get("sample_rate")
                    row["native"] = name in NATIVE
                    channels = stats.get("channels") or 1
                    row["pcm_match"] = pcm_match(path, out, channels)
                    row["status"] = "covered" if stats.get("sample_frames") and row["pcm_match"] else "fails"
                except Exception as error:
                    row["status"] = status_for(error, path)
                    row["error"] = str(error)[:200]
        rows.append(row)
    for label, ext, kind, encode in CONTAINERS:
        row = {"codec": label, "component": f"container .{ext}", "kind": "container",
               "container": ext, "media": kind}
        demuxer = EXT_DEMUXER.get(ext)
        if encode is None:
            row.update(status="needs-sample", fixture="decode-only", decoder=has_decoder(label),
                       demuxer=None if demuxer is None else demuxer in FORMATS)
            prove_with_sample(row, work)
            unreachable = (row["demuxer"] is False) if demuxer else not row["decoder"]
            if row["status"] == "needs-sample" and unreachable:
                row.update(status="absent-in-build",
                           error="no FFmpeg demuxer or decoder for this row")
            rows.append(row)
            continue
        path, why = (fixture(work, f"ct-{label}", ext, ["-an", *encode]) if kind == "v"
                     else audio_fixture(work, f"ct-{label}", ext, ["-vn", *encode]))
        if path is None:
            row.update(status="no-fixture", fixture="muxer-unavailable", error=why[:200])
            rows.append(row)
            continue
        row["fixture"] = "generated"
        row["file"] = str(path)
        try:
            if kind == "v":
                stats = fvid("decode", path)
                row["frames"] = stats.get("video_frames")
                row["status"] = "covered" if stats.get("video_frames") else "fails"
            else:
                out = work / f"container-{label}-out.wav"
                stats = fvid("decode-audio", path, out)
                row["sample_frames"] = stats.get("sample_frames")
                channels = stats.get("channels") or 1
                row["pcm_match"] = pcm_match(path, out, channels)
                row["status"] = "covered" if stats.get("sample_frames") and row["pcm_match"] else "fails"
        except Exception as error:
            row["status"] = status_for(error, path)
            row["error"] = str(error)[:200]
        rows.append(row)
    for name, ext, encode, frames in IMAGE:
        row = {"codec": name, "component": "MPC Image Source (Mega)", "kind": "image",
               "container": ext, "decoder": has_decoder(name), "expected_frames": frames}
        if not encode:
            row.update(status="needs-sample", fixture="decode-only")
            prove_with_sample(row, work)
            if row["status"] == "needs-sample" and not row["decoder"]:
                row.update(status="absent-in-build", error="no FFmpeg decoder for this row")
            rows.append(row)
            continue
        path, why = image_fixture(work, name, ext, encode, frames)
        if path is None:
            row.update(status="no-fixture", fixture="encoder-unavailable", error=why[:200])
            rows.append(row)
            continue
        row["fixture"] = "hand-authored" if encode == "hand" else "generated"
        row["file"] = str(path)
        demuxer = HINTED.get(name)
        if demuxer:
            row["input_format"] = demuxer
        row["descriptor"], row["decoder"] = descriptor_of(path, "v", demuxer)
        # A still that probes back as some other codec proves that other row, not this one — so the
        # fixture has to carry its own format's signature, which is measured here rather than assumed.
        wanted = DECODER.get(name, DECODE_ID.get(name, name))
        if row["descriptor"] != wanted:
            row.update(status="fails",
                       error=f"the fixture probes back as {row['descriptor']}, not {wanted}")
            rows.append(row)
            continue
        try:
            stats = fvid("decode", path, *(["--input-format", demuxer] if demuxer else []))
            row["frames"] = stats.get("video_frames")
            row["width"] = stats.get("width")
            row["native"] = name in NATIVE
            row["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()[:16]
            row["status"] = "covered" if row["frames"] == frames else "fails"
            # The name is the format's own requirement, not a convenience of this harness: a file
            # whose bytes carry no signature states nothing for a probe to find, so a read that
            # works without the name would prove the hint optional and the row would say so.
            if demuxer and row["status"] == "covered" and not fvid_refuses("decode", path):
                row.update(status="fails",
                           error=f"{name} decoded without naming {demuxer}, so the hint is not a requirement")
            # The other half of the replaced bar: a `TOLERATED_EOF` row is covered by a read that
            # skipped a damaged tail, and the counter is what says so. Zero here means either the
            # format stopped ending its stream in damage — in which case the fixture check above has
            # already demanded the exemption be dropped — or the frames came from somewhere else.
            if name in TOLERATED_EOF and row["status"] == "covered":
                row["decode_errors"] = stats.get("decode_errors") or 0
                if not row["decode_errors"]:
                    row.update(status="fails",
                               error=f"{name} decoded with nothing skipped, so the end-of-stream "
                                     "tolerance this row is covered by did not run")
        except Exception as error:
            row["status"] = status_for(error, path)
            row["error"] = str(error)[:200]
        rows.append(row)
    return rows


MARKER = "## Матрица\n"


# A number in prose is a claim about a measurement, and a hand-copied one goes stale the day the
# matrix grows a row (this happened twice in a week). So the prose writes `33<!--image_rows-->`
# and the measurement re-numbers the digits; a marker may also carry the Russian noun forms it
# agrees with (`50<!--mcp_no_non_image:строка:строки:строк--> строк`), and then the same run
# re-inflects the noun that follows. An unknown marker name fails the gate, so a counter cannot
# outlive the number it was copied from.
COUNTER_RE = re.compile(r"(\d+)(<!--([a-z0-9_]+)(?::([^\s:<>]+):([^\s:<>]+):([^\s:<>]+))?-->)"
                        r"([ \t]*)([^\s,.;:()—\n]*)")


def prose_counters(rows, report):
    """The counts the hand-written prose is allowed to restate, computed from this run."""
    inventory = report["inventory"]
    summary = report["summary"]
    surface = report["player_surface"]
    mcp = report["mcp_surface"]
    unprovable = report["image_surface"]["unprovable_evidence"]

    def where(**attrs):
        return sum(1 for row in rows if all(row.get(key) == value for key, value in attrs.items()))

    image_rows = where(kind="image")
    still = where(kind="image", player_cause="still-image")
    return {
        "rows_total": len(rows), "rows_covered": summary.get("covered", 0),
        "rows_waiting": summary.get("needs-sample", 0),
        "rows_absent": summary.get("absent-in-build", 0),
        "video_rows": where(kind="video"), "audio_rows": where(kind="audio"),
        # How many rows of each kind the native column now proves by running them, which is what the
        # prose quotes instead of a hand-kept count.
        "audio_native": where(kind="audio", native=True),
        "video_native": where(kind="video", native=True),
        "image_rows": image_rows, "container_rows": inventory["matrix_container_rows"],
        "codec_rows": inventory["matrix_codec_rows"],
        "codec_plus_image_rows": inventory["matrix_codec_rows"] + image_rows,
        "decodable_codecs": inventory["decodable_codecs"],
        "ability_lines": report["ability"]["lines"],
        "pack_components": (report["pack_components"] or {}).get("components", 0),
        "player_yes": surface.get("yes", 0), "player_no": surface.get("no", 0),
        "player_not_run": surface.get("not-run", 0),
        "mcp_yes": mcp.get("yes", 0), "mcp_no": mcp.get("no", 0),
        "mcp_not_run": mcp.get("not-run", 0),
        "image_mcp_yes": where(kind="image", mcp="yes"),
        "image_mcp_no": where(kind="image", mcp="no"),
        "mcp_no_non_image": sum(1 for row in rows
                                if row["kind"] != "image" and row.get("mcp") == "no"),
        # The two audio queues, so the prose quotes the split instead of the one bucket that
        # used to hold both.
        "player_audio_demux": len((surface.get("audio_causes") or {}).get("container", [])),
        "player_audio_arms": len((surface.get("audio_causes") or {})
                                 .get("codec-unselected", [])),
        "image_player_still": still, "image_player_signature": image_rows - still,
        "image_generated": where(kind="image", fixture="generated"),
        "image_reference": where(kind="image", fixture="reference-sample"),
        "image_hand": where(kind="image", fixture="hand-authored"),
        "image_unprovable": len(report["image_surface"]["unprovable"]),
        # The exemption's own evidence, so the prose quotes the search rather than a memory of it:
        # how many takes the sample archive indexes for the exempt formats, and how many of the
        # exemptions had every one of its three questions answered by this run.
        "image_unprovable_candidates": sum(ev["archive_candidates"] for ev in unprovable.values()),
        "image_unprovable_measured": sum(1 for ev in unprovable.values()
                                         if ev["archive_index"] == "present" and ev["family_rows"]),
        "image_codecs": report["image_surface"]["codecs"],
        # The two facts the prose states about the rows no decoder in this build serves: how many
        # carry a measured reason, and whether the build disabled anything itself.
        "rows_absent_explained": len(inventory["absent_in_build"])
                                 - len(inventory["absent_without_reason"]),
        # How many of those rows fvid's own code carries with nothing in the reference to agree
        # with — a number the prose counts and this run measures (`measure_own_pipeline`).
        "rows_own_pipeline": where(own_only=True),
        "ffmpeg_disable_flags": len(inventory["ffmpeg_disable_flags"]),
    }


def stamp_prose_counters(prose, counters):
    """Re-number (and re-agree) every counter the prose quotes, reporting what drifted."""
    problems = []

    def replace(match):
        digits, name = match.group(1), match.group(3)
        if name not in counters:
            problems.append(f"{name}: the prose quotes a counter this gate does not measure")
            return match.group(0)
        value = counters[name]
        if int(digits) != value:
            problems.append(f"{name}: the prose said {digits}, this run measured {value}")
        noun = match.group(8)
        if match.group(4):
            noun = plural(value, match.group(4), match.group(5), match.group(6))
        return f"{value}{match.group(2)}{match.group(7)}{noun}"

    return COUNTER_RE.sub(replace, prose), problems


def render_markdown(rows, heading="## Матрица", summary=None, strictness=None, surfaces=None,
                    mcp=None, unreachable=None):
    """Rebuild the matrix section of docs/KLITE_COVERAGE.md from the measured rows."""

    def proof(row):
        if row.get("own_only"):
            text = ("сравнить не с чем: у собранного FFmpeg нет ни демуксера, ни декодера для этой "
                    "строки, и отказ `ffprobe` этот прогон проверяет заново. "
                    "Доказательство — собственный конвейер: на фикстуре, написанной здесь по "
                    "спецификации формата, `examples/audio_probe` даёт {packets} окна "
                    "длительностью в {window} кадров каждое, {sounding} из них звучащие, а объём "
                    "выданного PCM сверяется побайтово").format(packets=row["own_packets"], window=row["own_window"],
                                        sounding=row["own_sounding"])
            take = (row.get("own_take") or {}).get("take")
            if take:
                text += ("; внешний сэмпл строки, сверенный по sha256, проходит тот же путь: "
                         f"{take['packets']} окон, {take['frames']} кадров, "
                         f"{take['sounding']} из них звучащих")
            return text + "; " + ABSENT_REASON.get(row["codec"], "")
        if row.get("fixture") == "reference-sample-cut":
            return "референс-сэмпл, обрезан без перекодирования"
        if row.get("fixture") == "reference-sample-remuxed":
            return "референс-сэмпл, переупакован без перекодирования"
        if row.get("fixture") == "reference-sample-retagged":
            return "внешний битстрим того же стандарта, пере-тегнут в FourCC строки"
        if row.get("fixture") == "reference-sample":
            return "референс-сэмпл извне"
        if row.get("fixture") == "generated":
            return "локальная фикстура"
        if row.get("fixture") == "hand-authored":
            named = row.get("input_format")
            if named:
                return ("фикстура, написанная здесь по спецификации формата; байты без подписи, "
                        f"поэтому обе стороны читают её только с именем демуксера (`-f {named}` "
                        f"справочно, `--input-format {named}` в fvid), а чтение без имени обязано "
                        "отказать")
            if row["codec"] in TOLERATED_EOF:
                return ("фикстура, написанная здесь по спецификации формата; её справочный демуксер "
                        "кончает поток ошибкой вместо конца, поэтому планка `-xerror` заменена "
                        "замером: без неё справочное чтение обязано взять все объявленные кадры и "
                        "выйти чисто, а чтение fvid — назвать число пропущенных повреждённых "
                        "чтений (столбец «Результат»)")
            return "фикстура, написанная здесь по спецификации формата"
        if row["status"] == "absent-in-build":
            return ABSENT_REASON.get(row["codec"], "причина не измерена")
        return "декодера нет в сборке" if not row.get("decoder", True) else "внешнего сэмпла нет"

    def result(row):
        if row["status"] == "absent-in-build":
            half = ("контейнер этой сборки читается, декодера нет"
                    if row.get("container") in FORMATS
                    else "ни демуксера, ни декодера в FFmpeg нет")
            text = f"вне FFmpeg: {half}"
            if row.get("own_only"):
                text += (f", но строку несёт собственный конвейер fvid: {row['native_frames']} "
                         f"кадров за {row['own_seconds']:g} с")
            return text
        if row["status"] != "covered":
            tail = "декодер в FFmpeg есть" if row.get("decoder") else "декодера нет в сборке"
            return f"не доказано ({tail})"
        if row["kind"] == "audio" or row.get("media") == "a":
            text = f"покрыто ({row['sample_frames']} сэмплов)"
        else:
            text = f"покрыто ({row['frames']} кадров)"
        if row.get("decode_errors"):
            text += f", пропущено повреждённых пакетов: {row['decode_errors']}"
        if row.get("pcm_match"):
            text += ", PCM бит-в-бит по каналам"
        return text

    parts = [heading, "", "Сгенерировано `scripts/validate_klite_coverage.py`; числа берутся из",
             "`benchmarks/klite-coverage.json`, руками не правятся.", ""]
    if summary:
        waiting = [row["codec"] for row in rows if row["status"] == "needs-sample"]
        absent = [row["codec"] for row in rows if row["status"] == "absent-in-build"]
        broken = [row["codec"] for row in rows
                  if row["status"] not in ("covered", "needs-sample", "absent-in-build")]
        measured = [row["codec"] for row in rows if row.get("native_proof") == "probe"]
        by_tests = [row["codec"] for row in rows if row.get("native_proof") == "tests"]
        rows_word = plural(len(rows), "строка", "строки", "строк")
        rows_from = plural(len(rows), "строки", "строк", "строк")
        parts += [f"- {len(rows)} {rows_word}: **{summary.get('covered', 0)} покрыто доказательно**, "
                  f"ждут внешнего сэмпла {len(waiting)}, вне сборки FFmpeg {len(absent)}, "
                  f"падает {len(broken)}.",
                  f"- нативные декодеры fvid, доказанные прогоном своего пайплайна: "
                  f"{', '.join(measured)}.",
                  f"- `copy_strictness: {strictness}`."]
        if by_tests:
            parts.append(f"- native, но доказано тестами, не этим прогоном: {', '.join(by_tests)}.")
        if surfaces and "error" not in surfaces:
            parts.append(f"- собственный плеер без окна открывает **{surfaces['yes']}** "
                         f"{plural(surfaces['yes'], 'строку', 'строки', 'строк')} из "
                         f"{len(rows)} {rows_from} (отказ {surfaces['no']}, не мерялось "
                         f"{surfaces['not-run']}): {', '.join(surfaces['carried'])}.")
            parts.append("- причины отказов: " + ", ".join(
                f"{name} — {count}" for name, count in sorted(surfaces["causes"].items())) + ".")
            audio_causes = surfaces.get("audio_causes") or {}
            if audio_causes:
                parts.append(
                    "- у аудио-отказов причины разделяются на две очереди работ: "
                    + ", ".join(
                        f"{CAUSE_NAMES.get(cause, cause)} — {len(names)} "
                        f"({', '.join(f'`{name}`' for name in names)})"
                        for cause, names in sorted(audio_causes.items()))
                    + ". Разделение печатается по тому, что сам прогон назвал кодирующим словом "
                      "файла; смешивать их нельзя, потому что демуксер и ветка раскодирования — "
                      "разные работы, и регрессия `player-cause-split` ловит отказ, который не "
                      "называет ни ту, ни другую.")
        elif surfaces:
            parts.append(f"- колонка «в плеере» не снята: {surfaces['error']}.")
        if mcp and "error" not in mcp:
            parts.append(f"- сервер `fvid mcp` открывает через `fvid_probe` **{mcp['yes']}** "
                         f"{plural(mcp['yes'], 'строку', 'строки', 'строк')} "
                         f"из {len(rows)} {rows_from} (отказ {mcp['no']}, не мерялось "
                         f"{mcp['not-run']}); "
                         f"каждый отказ — демуксер вне allowlist-а политики standalone-input, "
                         f"несовпадение ловит регрессия `mcp-policy`. Список демуксеров вычитан из "
                         f"`crates/fvid-media/src/lib.rs`: {', '.join(f'`{d}`' for d in mcp['allowlist'])}.")
        elif mcp:
            parts.append(f"- колонка «через MCP» не снята: {mcp['error']}.")
        image_rows = [row for row in rows if row["kind"] == "image"]
        if image_rows:
            parts.append(
                f"- изображения (это компонент Mega `MPC Image Source`, а не строка таблицы "
                f"возможностей): **{sum(1 for row in image_rows if row['status'] == 'covered')}** "
                f"из {len(image_rows)}. Набор выведен из `ffmpeg -codecs` — дескриптор с декодером "
                f"и со словами image/bitmap/raster в описании, — а не из списка расширений, "
                f"которого у вендора нет; пустоту в нём ловит регрессия `image-codec-rows`.")
            if surfaces and surfaces.get("by_kind"):
                still = (surfaces.get("cause_rows") or {}).get("still-image", [])
                carried = surfaces["by_kind"]["image"]["yes"]
                parts.append(
                    f"- в собственном плеере из этих {len(image_rows)} "
                    f"{plural(len(image_rows), 'строки', 'строк', 'строк')} открыто **{carried}**: "
                    f"{len(still)} {plural(len(still), 'строка', 'строки', 'строк')} доходят до ветки "
                    f"картинок `NativeReader` и отбраковываются как неподвижные "
                    f"({', '.join(f'`{c}`' for c in still)}), остальные не распознаются по сигнатуре "
                    f"файла. Это граница плеера, а не матрицы: строка доказана раскодированием через "
                    f"`fvid media`.")
        for name, entry in sorted((unreachable or {}).items()):
            parts.append(f"- `{name}` без строки, и это не отсутствие материала: {entry['reason']}.")
        if waiting:
            parts.append(f"- строки без сэмпла: {', '.join(f'`{c}`' for c in waiting)}.")
        if absent:
            parts.append(f"- строк нет в собранном FFmpeg: {', '.join(f'`{c}`' for c in absent)}.")
        if broken:
            parts.append(f"- падает: {', '.join(f'`{c}`' for c in broken)}.")
        parts.append("")
    unreach = sorted(unreachable or {})
    unreach_note = ("" if not unreach else
                    f" У {len(unreach)} "
                    f"{plural(len(unreach), 'формата', 'формата', 'форматов')} без собственной "
                    "сигнатуры строки нет (" + ", ".join(f"`{name}`" for name in unreach)
                    + "); причина по каждому замеряется этим прогоном и напечатана списком выше, "
                    "а не держится в памяти.")
    parts += ["", "_Колонка «свой декодер» — код этого репозитория, а не libavcodec, и она "
              "измеряется этим гейтом. Аудио-строки гонятся через `examples/audio_probe`, который "
              "вызывает `fvid::codec::make_audio_decoder` напрямую; видео-строки — через "
              "`examples/decode_native_rgb`, который ведёт `NativeReader` (тот же dispatch, что у "
              "плеера) без окна и проверяет, что перемотка воспроизводит первый кадр побайтово. "
              "Видео считается покрытым, когда собственный декодер отдал ровно то же число кадров, "
              "что насчитал ffprobe на этом же файле; у lossless-аудио так же сверяется число "
              "сэмплов. Прогон идут на фикстурах самого репозитория (`tests/fixtures/...`), потому "
              "что нативные декодеры по конструкции уже отбраковывают дефолтные настройки "
              "энкодеров — см. раздел о границах native-пути. Пометка «тесты» остаётся только за "
              "строками, которые прогнать не удалось, но которые закрыты `cargo test`._",
              "",
              "_У строки, за которой в FFmpeg нет решительно ничего, сверять не с чем, и "
              "доказательство у неё своё: `measure_own_pipeline` гонит конвейер fvid на фикстуре, "
              "написанной здесь по спецификации формата, и на закреплённом внешнем файле, если он "
              "для строки есть. Байты фикстуры тоже закреплены по sha256: пишут её те же "
              "`src/container/...`, что и читают, так что правка, устроившая обе стороны, "
              "иначе прошла бы незаметно. Дальше прогон требует раскода всех пакетов, сверяет "
              "число кадров с длиной, которую заявляют их собственные штампы, читает обратно "
              "выданный PCM и считает, в скольких десятых долях секунды в нём есть звук; отказ "
              "`ffprobe` измеряется заново на каждом из файлов. Числа при этом печатает та же "
              "строка матрицы, а не текст вокруг неё._",
              "",
              "_Колонка «в плеере» — тот же прогон, но вопрос другой: может ли файл открыть "
              "собственный конвейер fvid, то есть `NativeReader` для картинки и "
              "`Mp4AudioReader`/`WebmAudioReader`/`SmfAudioReader`/`XmAudioReader` + "
              "`make_audio_decoder` для звука. Это узьше "
              "матрицы: main-колонка доказывает `fvid media`, а значит весь набор libavcodec._",
              "",
              "_Колонка «через MCP» — тот же файл строки, но открытый сервером `fvid mcp` "
              "(`fvid_probe` поверх `with_standalone_inputs`). «нет» здесь значит, что демуксер "
              "контейнера не входит в allowlist политики, а не то, что кодек не раскодывается: "
              "для каждой строки тот же файл отдельно пробуется `fvid media probe` без политики, и "
              "расхождение замера с allowlist-ом роняет гейт как `mcp-policy`._", "",
              "_Строки раздела «изображения» — одиночный кадр, записанный форматом в собственное "
              "расширение (`.dpx`, `.pfm`, `.sgi` и так далее); для `apng` кадр не один, а пять, "
              "иначе файл был бы обычным PNG. Строка считается покрытой только когда фикстура "
              "пробуется тем же дескриптором, который она называет." + unreach_note + "_", ""]
    for kind, title in (("video", "Видео"), ("audio", "Аудио"), ("image", "Изображения"),
                        ("container", "Контейнер")):
        parts += [f"### {title}", "",
                  "| Строка | Компонент K-Lite | Доказательство | Результат | Свой декодер | "
                  "В плеере | Через MCP |",
                  "| --- | --- | --- | --- | --- | --- | --- |"]
        for row in rows:
            if row["kind"] != kind:
                continue
            label = ("container ." + row["container"] if kind == "container" else
                     f"{row['component']} / .{row['container']}" if kind == "image"
                     else row["component"])
            own = ("да (замер)" if row.get("native_proof") == "probe" else
                   "да (тесты)" if row.get("native") else "—")
            where = {"yes": "да", "no": "нет", "not-run": "не мерялось"}.get(row.get("player"), "")
            if row.get("player_detail") and row.get("player") == "yes":
                where = "да (частично)"
            served = {"yes": "да", "no": "нет",
                      "not-run": "не мерялось"}.get(row.get("mcp"), "")
            parts.append(f"| `{row['codec']}` | {label} | {proof(row)} | {result(row)} | "
                         f"{own} | {where} | {served} |")
        parts.append("")
    if PACK_PATH.exists():
        titles = {"video": "Видео", "audio": "Аудио", "image": "Изображения",
                  "container": "Контейнер"}
        parts += ["### Компоненты пака", "",
                  "Состав K-Lite по изданиям — `benchmarks/klite-pack-components.json`, его "
                  "пересобирает `scripts/fetch_klite_pack_components.py` со страниц вендора "
                  "(таблица содержимого + страница загрузок). Каждый компонент обязан быть "
                  "отражён в `COMPONENT_MAP` этого скрипта: либо строками матрицы, либо явным "
                  "сказуемым «это не раскодирование» с файлом, где видно, чем fvid отвечает. "
                  "Неотражённый компонент, пропавшая строка матрицы или устаревшая отписка "
                  "роняют гейт как `pack-components`.", "",
                  "| Компонент | Basic | Standard | Full | Mega | Ответ fvid |",
                  "| --- | --- | --- | --- | --- | --- |"]
        for entry in json.loads(PACK_PATH.read_text())["components"]:
            mapping = COMPONENT_MAP.get(entry["component"], {})
            answer = mapping.get("not_codec", "")
            if mapping.get("kinds"):
                answer = "все строки раздела «" + "», «".join(
                    titles.get(kind, kind) for kind in mapping["kinds"]) + "»"
            elif mapping.get("rows"):
                answer = "строки " + ", ".join(f"`{row}`" for row in mapping["rows"])
                if mapping.get("note"):
                    answer += f" — {mapping['note']}"
            if mapping.get("paths"):
                answer += " (см. " + ", ".join(f"`{path}`" for path in mapping["paths"]) + ")"
            marks = [{"yes": "да", "no": "—", "partial": "частично"}[entry["editions"][edition]]
                     for edition in EDITIONS]
            parts.append(f"| {entry['component']} | " + " | ".join(marks)
                         + f" | {answer or 'не отражён'} |")
        parts.append("")
    return "\n".join(parts)


ABILITY_PATH = ROOT / "benchmarks" / "klite-ability.json"

# Vendor lines that name a family resolved by one decoder row (the row's own component text
# carries the FourCCs, so only the family label needs spelling out).
ABILITY_FAMILY = {
    "DTS-HD (Master Audio / HRA)": "dts", "DTS-ES": "dts", "DTS Express (LBR)": "dts",
    "MPEG-4/DivX/Xvid": "mpeg4p2", "MP41": "msmpeg4v1", "MP42": "msmpeg4v2", "MP43": "msmpeg4v3",
    "Windows Media Video": "wmv2", "Dolby TrueHD/MLP": "truehd", "Ogg": "vorbis",
    "MPEG-4 Audio": "aac", "Apple Lossless Audio Codec": "alac", "Matroska": "mka",
    "Flash Video": "flv", "RealMedia": "rm", "Direct Stream Digital (DSD)": "dsd",
    "Various other formats": "pcm_qt", "Various other": "mtv",
    "True Audio": "tta", "MPEG-4 (Xvid)": "mpeg4p2", "H.264 (x264VFW)": "h264",
}


def ability_gaps(rows):
    """Vendor format lines that no matrix row answers — the row list must not shrink silently.

    `benchmarks/klite-ability.json` is the K-Lite ability table parsed once from codecguide;
    a line counts as covered when one of its FourCC/AudioTag tokens or file extensions appears
    in a row, its label matches a row name, or ABILITY_FAMILY maps it to the family row.
    """
    if not ABILITY_PATH.exists():
        return None
    lines = json.loads(ABILITY_PATH.read_text())
    haystack = " ".join(f"{r['codec']} {r['component']} {r.get('container','')}" for r in rows).upper()
    names = {re.sub(r"[^a-z0-9]", "", r["codec"].lower()) for r in rows}
    uncovered = []
    for line in lines:
        label, tags = line["label"], line["tags"]
        if (label.startswith(("Supported", "Video encoding", "Audio encoding", "Green", "Name",
                              "DVD", "Blu-ray", "VCD", "DTS audio CD"))
                or label in ("File extensions", "Notes", "FourCC", "AudioTag or FourCC")
                or tags.startswith(("Works only", "Note", "FourCC", "AudioTag", "Green", "Windows"))):
            continue
        keys = [t.strip().upper() for t in re.split(r"[,/]", tags) if 1 < len(t.strip()) < 14]
        keys += [e.upper().lstrip(".") for e in re.findall(r"\.\w{1,5}", tags)]
        slug = re.sub(r"[^a-z0-9]", "", label.lower())
        named = any(len(n) > 2 and (n in slug or slug in n) for n in names)
        if (any(k in haystack for k in keys if k not in ("S", "D")) or named
                or ABILITY_FAMILY.get(label, "").lower() in names):
            continue
        uncovered.append(label)
    return {"lines": len(lines), "uncovered": uncovered}


def ffmpeg_encoders():
    """Encoder names this FFmpeg writes with — the image rows that need no external sample."""
    text = subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True).stdout.decode()
    found = set()
    for line in text.splitlines():
        parts = line.split()
        if len(parts) > 1 and parts[0][0] in "VAS" and len(parts[0]) == 7:
            found.add(parts[1])
    return found


ENCODERS = ffmpeg_encoders()


def image_codecs():
    """Codecs this build decodes that FFmpeg itself describes as an image, bitmap or raster format.

    The marketed Mega component says "image formats" and nothing more, so the row set is taken
    from the build's own codec descriptions rather than from a guessed extension list.
    """
    text = subprocess.run(["ffmpeg", "-hide_banner", "-codecs"], capture_output=True).stdout.decode()
    found = set()
    for line in text.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0].startswith("D") and \
                any(word in parts[2].lower() for word in ("image", "bitmap", "raster")):
            found.add(parts[1])
    # `.webp` is the one still the vendor's own ability table names (a tag of its WebM row).
    return found | {"webp"}


def sample_archive_index():
    """Paths the public sample archive lists, from the cache `fetch_klite_samples.py --auto` writes.

    None means nobody looked this run, which the exemption must not report as a search that
    happened. Read from the cache only: the gate measures the build, the network is the fetcher's.
    """
    cache = SAMPLE_DIR / ".allsamples.txt"
    if not cache.exists():
        return None
    return [line[2:] for line in cache.read_text(errors="replace").splitlines()
            if line.startswith("./") and not line.endswith((".txt", ".log"))]


def unprovable_evidence(rows):
    """The exemption's three claims, each answered by the run rather than by a memory of one.

    The probe half is read off the descriptors this very run measured with `ffprobe`, so it cannot
    quote a file the run did not look at, and it says nothing about a family with no row.
    """
    index = sample_archive_index()
    evidence = {}
    for name, family in sorted(UNPROVABLE_FAMILY.items()):
        needles = UNPROVABLE_SEARCH[name]
        hits = sorted(path for path in index or ()
                      if any(needle in path.lower() for needle in needles["descriptors"])
                      or any(path.lower().endswith(f".{ext}") for ext in needles["extensions"]))
        evidence[name] = {
            "encoder": name in ENCODERS,
            "pinned_sample": name in PINS,
            "archive_index": "present" if index is not None else "absent",
            "archive_candidates": len(hits),
            "archive_hits": hits[:5],
            "family_rows": {row["codec"]: row["descriptor"] for row in rows
                            if row.get("descriptor") in (*family, name)},
        }
    return evidence


def image_gaps(rows, unreachable=None):
    """Image codecs the build decodes that the matrix neither carries nor honestly exempts.

    An exemption is only while it stays unmeetable: if this build grows an encoder for the format,
    or a reference take gets pinned, the row has to be added, so the exemption is reported as the
    regression instead of quietly keeping the hole.
    """
    have = {row["codec"] for row in rows if row["kind"] == "image"}
    derived = image_codecs()
    evidence = unprovable_evidence(rows)
    stale = []
    for name, reason in sorted(IMAGE_UNPROVABLE.items()):
        ev = evidence[name]
        if name in have:
            stale.append(f"{name}: it has a matrix row now, drop the exemption")
        elif name not in derived:
            stale.append(f"{name}: this build no longer describes it as an image codec")
        elif ev["encoder"] or ev["pinned_sample"]:
            stale.append(f"{name}: now producible ({reason}), so it needs a row")
        elif ev["archive_index"] == "absent":
            stale.append(f"{name}: the sample archive was not searched this run, so the claim "
                         f"«{reason}» is not a measurement — run scripts/fetch_klite_samples.py")
        elif ev["archive_candidates"]:
            stale.append(f"{name}: the sample archive lists {ev['archive_candidates']} candidate "
                         f"take(s) ({', '.join(ev['archive_hits'])}), so the exemption owes them "
                         f"an answer")
        elif not ev["family_rows"]:
            stale.append(f"{name}: the matrix proved no take of {'/'.join(UNPROVABLE_FAMILY[name])}, "
                         f"so \"{reason}\" describes an answer nobody measured")
        elif name in ev["family_rows"].values():
            stale.append(f"{name}: a take of this run probed as the image descriptor "
                         f"({ev['family_rows']}), so the row is available")
    for name in sorted(set(UNREACHABLE) - set(IMAGE_UNPROVABLE)):
        stale.append(f"{name}: measured as signature-less but not exempted anywhere")
    for name in sorted(set(unreachable or {}) & have):
        stale.append(f"{name}: it has a matrix row now, drop it from UNREACHABLE")
    # A `TOLERATED_EOF` name claims a row on a read that skipped a damaged tail, so it has to carry
    # that row: an exemption written in the fixture helper and answered by nothing is a hole, not a
    # measurement.
    for name in sorted(set(TOLERATED_EOF) - have):
        stale.append(f"{name}: excused from the -xerror bar but has no matrix row to spend it on")
    for name in sorted(set(TOLERATED_EOF) & set(UNREACHABLE)):
        stale.append(f"{name}: listed both as a tolerated row and as an unreadable exemption")
    return {"codecs": len(derived), "rows": len(have), "unprovable": sorted(IMAGE_UNPROVABLE),
            "unprovable_evidence": evidence, "unreachable": unreachable or {},
            "gaps": sorted(derived - have - set(IMAGE_UNPROVABLE)), "stale": stale}


# An exempt format whose file the gate *can* write from its specification, so the open question is
# not whether material exists but whether either side can read the file: the reference, the way it
# reads any file, and fvid on the same bytes. Every half of the answer is measured each run, and an
# exemption whose measurement turns positive is a regression that has to be answered with a row.
# The two formats it was written for are rows now — `gem` since the input layer takes a demuxer name,
# `jv` since the video path reads the damaged end of stream its own demuxer produces. A format that
# goes back to being unreadable is listed here, in the same shape:
# name -> (extension, the demuxer the reference has to be named with, or None if it probes itself)
UNREACHABLE: dict[str, tuple[str, str | None]] = {}


def measure_unreachable(work):
    """Ask both sides about a format whose file the gate writes from its specification.

    An exemption used to be a sentence about the sample archives. Whether what is missing is
    material, a signature, or the reference's own end-of-stream manners is measurable on a file the
    gate can write, so each run asks: does the reference read it clean, how many frames does it
    decode, what does the probe answer when nobody names a format, and what does fvid say?"""
    answers, problems = {}, []
    for name, (ext, demuxer) in sorted(UNREACHABLE.items()):
        path = work / f"img-{name}.{ext}"
        path.write_bytes(HAND[name]())
        named = ["-f", demuxer] if demuxer else []
        clean = sh(["ffmpeg", "-nostdin", "-v", "error", "-xerror", *named, "-i",
                    str(path), "-f", "null", "-"])
        counted = sh(["ffprobe", "-v", "error", *named, "-select_streams", "v:0",
                      "-count_frames", "-show_entries", "stream=nb_read_frames",
                      "-of", "default=nw=1", str(path)])
        found = re.search(r"nb_read_frames=(\d+)", counted.stdout.decode(errors="replace"))
        frames = int(found.group(1)) if found else None
        probe_as, _ = descriptor_of(path, "v")
        entry = {"demuxer": demuxer, "probe_as": probe_as, "fixture": path.name,
                 "reference_frames": frames, "reference_clean": clean.returncode == 0}
        answers[name] = entry
        if frames != 1:
            problems.append(f"{name}: the spec fixture yields {frames} frames for the reference, "
                            f"not the one the exemption describes")
            continue
        wanted = DECODER.get(name, DECODE_ID.get(name, name))
        try:
            stats = fvid("decode", path)
        except Exception as error:
            refused = last_line(str(error), 120)
            entry["fvid_refused"] = refused
            named_s = f" по `-f {demuxer}`" if demuxer else ""
            if not entry["reference_clean"]:
                failure = last_line(clean.stderr.decode(errors="replace"), 120)
                entry["reference_error"] = failure
                # `-xerror` failing says either "the file is damaged" or "the demuxer ends its
                # stream with an error code". Only one of those is a tolerance question, and the
                # difference is measurable: drop the bar and see whether the same read still
                # pulls every frame the file declares and exits clean.
                lenient = sh(["ffmpeg", "-nostdin", "-v", "info", *named, "-i", str(path),
                              "-f", "null", "-"])
                counts = re.findall(rb"frame=\s*(\d+)", lenient.stderr)
                pulled = int(counts[-1]) if counts else None
                entry["reference_frames_whole_read"] = pulled
                entry["reference_whole_read_clean"] = lenient.returncode == 0
                if pulled != frames or lenient.returncode != 0:
                    problems.append(
                        f"{name}: without -xerror the reference gets {pulled} frames "
                        f"(return code {lenient.returncode}) instead of the {frames} ffprobe "
                        f"counts, so the fixture itself is damaged — that is a different "
                        f"exemption than the one written here")
                    continue
                entry["reason"] = (
                    f"материала хватает: файл, записанный по спецификации{named_s}, раскодируется "
                    f"справочно ({frames} кадр(ов)). Не проходит только бар `-xerror`, который "
                    f"обязана проходить фикстура строки, — и не проходит он у самой справочной "
                    f"реализации: её чтение за последним кадром кончается ошибкой вместо конца "
                    f"потока ({failure}). Без этой планки та же команда достаёт все {frames} "
                    f"кадр(ов) и завершается чисто, то есть фикстура целая, а fvid на ней "
                    f"отвечает той же ошибкой ({refused}). Строку здесь разрешает не образец, а "
                    f"решение о терпимости входа")
            elif probe_as is None:
                entry["reason"] = (f"записанная по спецификации фикстура раскодируется "
                                   f"справочно{named_s} ({frames} кадр(ов)), без указания "
                                   f"демуксера проба даёт ничего, и `fvid media decode` её "
                                   f"отвергает: {refused}")
            else:
                entry["reason"] = (f"записанная по спецификации фикстура раскодируется "
                                   f"справочно{named_s} ({frames} кадр(ов)), но без названия "
                                   f"демуксера её поток проба отдаёт как `{probe_as}`, и "
                                   f"`fvid media decode` её отвергает: {refused}")
            continue
        entry["fvid_frames"] = stats.get("video_frames")
        if probe_as == wanted:
            problems.append(f"{name}: fvid reaches the descriptor the row names on its own "
                            f"({stats.get('video_frames')} frames) — the exemption is stale and "
                            f"the row has to be written")
            continue
        entry["reason"] = (f"фикстура раскодируется справочно ({frames} кадр(ов)), но без "
                           f"названия демуксера её поток проба отдаёт как `{probe_as}`, и "
                           f"`fvid media decode` читает файл именно так "
                           f"({stats.get('video_frames')} кадр(ов)) — строка о `{name}` "
                           f"доказывала бы чужой декодер")
    return answers, problems


PACK_PATH = ROOT / "benchmarks" / "klite-pack-components.json"

# K-Lite's own component list, per edition, is `benchmarks/klite-pack-components.json` (regenerated
# by scripts/fetch_klite_pack_components.py). Every component has to be answered here with either
# the matrix rows that carry it (`rows`, or whole `kinds` of the matrix) or an explicit statement
# that it is not a decoding capability, with the files that document what fvid does instead.
COMPONENT_MAP = {
    "Media Player Classic Home Cinema (MPC-HC)": {
        "not_codec": "плеер, а не декодер; у fvid два своих воспроизводящих интерфейса, и оба "
                     "замерены построчно в колонках «В плеере» и «Через MCP»",
        "paths": ["docs/NATIVE_PLAYBACK.md"]},
    "LAV Video decoder": {"kinds": ["video"]},
    "LAV Audio decoder": {"kinds": ["audio"]},
    "LAV Splitter": {"kinds": ["container"]},
    "MPC Image Source": {"kinds": ["image"]},
    "ffdshow video processor": {
        "not_codec": "фильтр пост-обработки, а не декодер; у fvid свой набор фильтров "
                     "квалифицирован сравнением с FFmpeg",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "ffdshow audio processor": {
        "not_codec": "фильтр обработки звука; fvid считает громкость и нормализует её своими "
                     "командами, сравнение с FFmpeg тоже квалифицировано",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "ffdshow VFW interface (32-bit only)": {
        "not_codec": "интерфейс VFW для приложений-энкодеров на Windows; у fvid нет и не "
                     "запланировано Win32-API, кодирование отдаётся вызывающей стороне",
        "paths": ["README.md"]},
    "Bass Audio Source": {
        "rows": ["optimfrog", "tracker"],
        "statuses": {"optimfrog": ("absent-in-build",), "tracker": ("absent-in-build",)},
        "note": "OptimFROG — сознательный отказ (нет ни декодера в сборке, ни решения его "
                "писать); модули трекеров читаются демуксером xm, которого в этой сборке нет"},
    "DirectVobSub (xy-VSFilter)": {
        "not_codec": "рендер субтитров; fvid пробрасывает субтитрованные потоки, конвертирует "
                     "SRT в ASS и прожигает внешние субтитры",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "MPC Video Renderer": {"not_codec": "видеорендерер Windows", "paths": ["benchmarks/GPU_REPORT.md"]},
    "madVR": {"not_codec": "видеорендерер Windows с апскейлом", "paths": ["benchmarks/GPU_REPORT.md"]},
    "x264": {"rows": ["h264"]},
    "Xvid": {"rows": ["mpeg4p2"]},
    "Lagarith": {"rows": ["lagarith"]},
    "Huffyuv": {"rows": ["huffyuv"]},
    "AC3ACM": {"rows": ["ac3"]},
    "LAME MP3 Encoder": {"rows": ["mp3"]},
    "Icaros ThumbnailProvider": {
        "not_codec": "миниатюры в проводнике; fvid отдаёт кадр фильтром --thumbnail",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "Icaros PropertyHandler": {
        "not_codec": "свойства файла в проводнике; метаданные отдаёт `fvid media probe`",
        "paths": ["README.md"]},
    "Codec Tweak Tool": {"not_codec": "настройка регистров DirectShow на Windows"},
    "MediaInfo Lite": {
        "not_codec": "показ информации о файле; это делает `fvid media probe`",
        "paths": ["README.md"]},
    "GraphStudioNext": {"not_codec": "конструктор графов DirectShow для отладки"},
    "Plugin for 3D video decoding (H.264 MVC)": {"rows": ["h264mvc"],
                                                 "statuses": {"h264mvc": ("absent-in-build",)},
                                                 "note": "в этой сборке FFmpeg нет ни одного "
                                                         "MVC-дескриптора"},
    "HDR video playback": {
        "not_codec": "не кодек: тонмаппинг HDR в SDR — свойство конвейера фильтров, он "
                     "квалифицирован сравнением с FFmpeg",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "Subtitle display": {"not_codec": "не кодек: см. строки контейнеров субтитров и burn-subtitles",
                         "paths": ["docs/FEATURE_MATRIX.md"]},
    "Hardware accelerated video decoding": {
        "not_codec": "аппаратное декодирование; у fvid свой GPU-путь, он замерен отдельно",
        "paths": ["benchmarks/hw-validation.json", "benchmarks/GPU_REPORT.md"]},
    "Audio bitstreaming": {
        "not_codec": "сжатая дорожка на AV-ресивер сквозь HDMI; fvid звук всегда раскодирует в PCM "
                     "(decode-audio, loudnorm, mix-audio), а remux уносит сжатую дорожку в файл, а "
                     "не на устройство"},
    "DVD and Blu-ray (after decryption)": {
        "not_codec": "навигация по диску; fvid читает сами потоки (.vob/.m2ts строками контейнеров), "
                     "IFS-навигации у него нет",
        "paths": ["docs/KLITE_COVERAGE.md"]},
    "Video thumbnails in Explorer": {
        "not_codec": "оболочка Windows; кадр отдаёт фильтр --thumbnail",
        "paths": ["docs/FEATURE_MATRIX.md"]},
    "File association options": {"not_codec": "регистрация типов в реестре Windows"},
    "Broken codec detection": {
        "not_codec": "не кодек: о повреждённых потоках матрица судит замером строгости копии "
                     "(copy_strictness), а не по слову вендора"},
}

EXEMPT_STATUSES = ("covered", "absent-in-build")

# The four K-Lite editions, in the order the component table prints them. Cells are looked up by
# name, never by dict position, so a manifest that renames or drops one has to fail the check below
# rather than shift every column by one.
EDITIONS = ("basic", "standard", "full", "mega")


def component_gaps(rows):
    """Pack components the matrix leaves unanswered, and map entries the vendor no longer lists.

    Both directions matter: a component nobody mapped is a silent hole in the parity claim, and a
    mapping whose rows went missing (or which names a component the regenerated manifest dropped)
    is a claim that no longer points at anything.
    """
    if not PACK_PATH.exists():
        return None
    components = json.loads(PACK_PATH.read_text())["components"]
    by_kind, by_codec = {}, {}
    for row in rows:
        by_kind.setdefault(row["kind"], []).append(row)
        by_codec.setdefault(row["codec"], []).append(row)
    gaps, problems, stale = [], [], []
    for entry in components:
        name = entry["component"]
        if set(entry["editions"]) != set(EDITIONS):
            problems.append(f"{name}: the manifest marks editions "
                            f"{'/'.join(sorted(entry["editions"]))} instead of "
                            f"{'/'.join(EDITIONS)}, so the table columns would shift")
        mapping = COMPONENT_MAP.get(name)
        if mapping is None:
            gaps.append(name)
            continue
        if not (mapping.get("kinds") or mapping.get("rows") or mapping.get("not_codec")):
            problems.append(f"{name}: the mapping says neither which rows carry it nor why it "
                            f"is not a decoding capability")
        allowed = mapping.get("statuses", {})
        for kind in mapping.get("kinds", []):
            carried = by_kind.get(kind)
            if not carried:
                problems.append(f"{name}: no matrix row of kind `{kind}`")
                continue
            # A component answered by a whole slice of the matrix is only answered if every row of
            # that slice still stands; a kind that exists but fails proves nothing.
            weak = [row for row in carried if row["status"] not in allowed.get(row["codec"],
                                                                               EXEMPT_STATUSES)]
            if weak:
                problems.append(f"{name}: {len(weak)} of {len(carried)} `{kind}` rows are not "
                                f"proven — " + ", ".join(f"{r['codec']}:{r['status']}"
                                                         for r in weak[:5]))
        for codec in mapping.get("rows", []):
            matches = by_codec.get(codec)
            if not matches:
                problems.append(f"{name}: matrix row `{codec}` does not exist")
                continue
            for row in matches:
                if row["status"] not in allowed.get(codec, EXEMPT_STATUSES):
                    problems.append(f"{name}: row `{codec}` is {row['status']}")
        for path in mapping.get("paths", []):
            if not (ROOT / path).exists():
                problems.append(f"{name}: cites {path}, which is not in the tree")
    stale = [name for name in COMPONENT_MAP if name not in
             {entry["component"] for entry in components}]
    return {"components": len(components), "mapped": len(components) - len(gaps),
            "gaps": gaps, "problems": problems, "stale": stale,
            "editions": {edition: sum(1 for entry in components
                                      if entry["editions"].get(edition, "no") != "no")
                         for edition in EDITIONS}}


def decoder_mapping_gaps(rows):
    """Rows written off as absent from the build whose label names a codec the build does decode.

    `absent-in-build` is exempt from the gate, so a row mapped to the wrong descriptor vanishes
    quietly instead of failing. K-Lite names rows after a FourCC or a filter while libavcodec
    answers for its own descriptors, and the two differ (Intel H.263 is `i263` to K-Lite and
    `h263i` to the build), so the exemption is only honest while it matches what libavcodec
    actually calls its decodable codecs.
    """
    gaps = []
    for row in rows:
        if row["kind"] == "container" or row.get("decoder"):
            continue
        label = re.sub(r"\([^)]*\)", "", row["component"]).strip().lower()
        if len(label) < 3:
            continue
        hit = next((d for d, text in DECODABLE.items() if label in text), None)
        if hit:
            gaps.append(f'{row["codec"]}: "{row["component"]}" is decoded by `{hit}`')
    return gaps


DECODABLE = decodable_descriptions()


def inventory(rows):
    """Where the marketed row list sits relative to what the linked libavcodec can decode."""
    codecs = [row for row in rows if row["kind"] in ("video", "audio")]
    absent = [row for row in rows if row["status"] == "absent-in-build"]
    # Which half is missing per absent row: a row whose container this build demuxes lacks only a
    # decoder, and the prose says so differently from one nothing opens at all.
    halves = {row["codec"]: row.get("container") in FORMATS for row in absent}
    named = {row["codec"] for row in absent}
    return {
        "decodable_codecs": decodable_codecs(),
        "matrix_codec_rows": len(codecs),
        "matrix_image_rows": len([row for row in rows if row["kind"] == "image"]),
        "matrix_container_rows": len([row for row in rows if row["kind"] == "container"]),
        "rows_without_decoder_in_build": sorted(row["codec"] for row in codecs if not row["decoder"]),
        "ffmpeg_version": BUILD["version"],
        "ffmpeg_disable_flags": BUILD["disable_flags"],
        "absent_in_build": sorted(named),
        "absent_container_demuxes": halves,
        "absent_without_reason": sorted(named - set(ABSENT_REASON)),
        "stale_absent_reasons": sorted(set(ABSENT_REASON) - named),
    }


def row_accounting(rows):
    """The matrix cannot lose a row on its way out of `run()`, and cannot list one twice.

    Every other check here compares rows against the vendor's lists; none of them notices that a
    loop stopped appending, because a missing row simply stops being asked about. So the counts are
    asserted against the source lists themselves, which is the only thing that catches a dropped
    `rows.append`.
    """
    want = {"video": len(VIDEO), "audio": len(AUDIO), "container": len(CONTAINERS),
            "image": len(IMAGE)}
    have, seen, duplicates = {}, {}, []
    for row in rows:
        have[row["kind"]] = have.get(row["kind"], 0) + 1
        key = (row["kind"], row["codec"])
        seen[key] = seen.get(key, 0) + 1
    problems = [f"{kind}: {have.get(kind, 0)} rows measured, {count} listed"
                for kind, count in want.items() if have.get(kind, 0) != count]
    problems += [f"{kind}/{codec} listed twice" for (kind, codec), count in seen.items() if count > 1]
    return problems


def main():
    global BINARY
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(BINARY))
    parser.add_argument("--out", default=str(ROOT / "benchmarks" / "klite-coverage.json"))
    parser.add_argument("--markdown", default=None,
                        help="rewrite the matrix section of a doc from the measured rows")
    parser.add_argument("--native-probe", default=str(NATIVE_PROBE),
                        help="player-feature audio_probe binary that runs fvid's own decoders")
    parser.add_argument("--native-video-probe", default=str(NATIVE_VIDEO_PROBE),
                        help="player-feature decode_native_rgb binary driving NativeReader")
    parser.add_argument("--mcp-binary", default=str(MCP_BINARY),
                        help="mcp-feature fvid binary serving fvid_probe over stdio")
    args = parser.parse_args()
    BINARY = pathlib.Path(args.binary)
    with tempfile.TemporaryDirectory() as tmp:
        rows = run(pathlib.Path(tmp))
        strictness = copy_strictness(pathlib.Path(tmp))
        native_problems = measure_native_video(rows, args.native_video_probe, pathlib.Path(tmp))
        native_problems += measure_native(rows, args.native_probe, pathlib.Path(tmp))
        native_problems += measure_own_pipeline(rows, args.native_probe, pathlib.Path(tmp))
        surfaces = measure_surfaces(rows, args.native_video_probe, args.native_probe,
                                    pathlib.Path(tmp))
        mcp_surface, mcp_problems = measure_mcp(rows, args.mcp_binary, pathlib.Path(tmp))
        unreachable, unreachable_problems = measure_unreachable(pathlib.Path(tmp))
    # A fixture this run built for itself is named by what it is, not by the
    # random directory it was built in: otherwise the record of an unchanged
    # measurement differs every time it is rewritten.
    for row in rows:
        for key, value in row.items():
            if isinstance(value, str) and str(tmp) in value:
                row[key] = value.replace(f"{tmp}/", "<run-tmp>/")
    summary = {}
    for row in rows:
        summary[row["status"]] = summary.get(row["status"], 0) + 1
    ability = ability_gaps(rows)
    images = image_gaps(rows, unreachable)
    components = component_gaps(rows)
    report = {"binary": str(BINARY), "copy_strictness": strictness, "total": len(rows),
              "row_accounting": row_accounting(rows),
              "inventory": inventory(rows), "ability": ability, "player_surface": surfaces,
              "image_surface": images, "pack_components": components,
              "mcp_surface": {key: value for key, value in mcp_surface.items()
                              if not isinstance(value, list)},
              "native_probe": str(args.native_probe),
              "native_video_probe": str(args.native_video_probe), "native_decode": native_problems,
              "decoder_mapping": decoder_mapping_gaps(rows),
              "summary": summary, "rows": rows}
    pathlib.Path(args.out).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    prose_problems = []
    if args.markdown:
        doc = pathlib.Path(args.markdown)
        prose = doc.read_text().split(MARKER)[0].rstrip() + "\n\n"
        prose, prose_problems = stamp_prose_counters(prose, prose_counters(rows, report))
        doc.write_text(prose + render_markdown(rows, summary=summary, strictness=strictness,
                                               surfaces=surfaces, mcp=mcp_surface,
                                               unreachable=unreachable))
    for row in rows:
        if row["status"] != "covered":
            print(f'{row["status"]:>14}  {row["kind"]:<5} {row["codec"]:<12} {row["component"]}'
                  f'{" decoder-missing" if row.get("decoder") is False else ""}  {row.get("error","")[:120]}')
    print(json.dumps({**summary, "copy_strictness": strictness, "inventory": report["inventory"],
                      "ability": ability, "decoder_mapping": report["decoder_mapping"],
                      "player_surface": {key: value for key, value in surfaces.items()
                                         if not isinstance(value, list)},
                      "image_surface": {key: value for key, value in images.items()
                                          if key != "problems"},
                      "unreachable_problems": unreachable_problems,
                      "pack_components": {key: value for key, value in (components or {}).items()
                                          if not isinstance(value, list)},
                      "mcp_surface": {key: value for key, value in mcp_surface.items()
                                      if key != "allowlist"},
                      "native_decode": native_problems},
                     sort_keys=True))
    regressions = [row["codec"] for row in rows
                   if row["status"] not in ("covered", "needs-sample", "absent-in-build")]
    accounting = row_accounting(rows)
    if accounting:
        print("the matrix does not contain the rows it claims to measure: "
              + "; ".join(accounting), file=sys.stderr)
        regressions.append("row-accounting")
    mapping = decoder_mapping_gaps(rows)
    if mapping:
        print("rows written off as absent-in-build that the build does decode: "
              + "; ".join(mapping), file=sys.stderr)
        regressions.append("decoder-mapping")
    if native_problems:
        print("the own-decoder column is not proven: " + "; ".join(native_problems),
              file=sys.stderr)
        regressions.append("native-column")
    if ability and ability["uncovered"]:
        print(f"vendor ability lines with no matrix row: {', '.join(ability['uncovered'])}",
              file=sys.stderr)
        regressions.append("vendor-ability-lines")
    if images["gaps"] or images["stale"]:
        print("the image axis of the matrix is not closed: "
              + "; ".join([f"no row for {name}" for name in images["gaps"]] + images["stale"]),
              file=sys.stderr)
        regressions.append("image-codec-rows")
    if unreachable_problems:
        print("the signature-less image exemptions are not what they claim: "
              + "; ".join(unreachable_problems), file=sys.stderr)
        regressions.append("image-unreachable")
    build = report["inventory"]
    if (build["absent_without_reason"] or build["stale_absent_reasons"]
            or build["ffmpeg_disable_flags"]):
        print("the absent-in-build rows are not the reason the prose gives: "
              + "; ".join(
                  [f"{name} carries no stated reason" for name in build["absent_without_reason"]]
                  + [f"{name}: a reason is kept for a row that is covered again"
                     for name in build["stale_absent_reasons"]]
                  + [f"the linked FFmpeg was configured with {flag}, so an absent component might be "
                     "a build choice rather than an upstream gap"
                     for flag in build["ffmpeg_disable_flags"]]),
              file=sys.stderr)
        regressions.append("absent-reasons")
    if components and (components["gaps"] or components["problems"] or components["stale"]):
        print("the pack's own component list is not answered: "
              + "; ".join([f"unmapped component {name}" for name in components["gaps"]]
                          + components["problems"]
                          + [f"{name}: no such component in the manifest anymore"
                             for name in components["stale"]]), file=sys.stderr)
        regressions.append("pack-components")
    if strictness == "leaked":
        regressions.append("remux-accepts-corrupt-packets")
    if mcp_problems:
        print("the MCP surface does not match its own container policy: "
              + "; ".join(mcp_problems), file=sys.stderr)
        regressions.append("mcp-policy")
    if "error" in mcp_surface:
        print(f'MCP surface not measured: {mcp_surface["error"]}', file=sys.stderr)
    cause_problems = ([] if "error" in surfaces else surface_cause_problems(rows))
    if cause_problems:
        print("the player column does not say which half of the pipeline refused: "
              + "; ".join(cause_problems[:8]), file=sys.stderr)
        regressions.append("player-cause-split")
    if prose_problems:
        print("the prose half of the deliverable restates numbers the run did not measure: "
              + "; ".join(prose_problems), file=sys.stderr)
        regressions.append("prose-counters")
    if regressions:
        print(f"coverage regression: {', '.join(regressions)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

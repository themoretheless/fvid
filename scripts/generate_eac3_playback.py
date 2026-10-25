"""Author synthetic E-AC-3 playback regression fixtures and reference PCM."""

import hashlib
import json
import struct
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "tests/fixtures/playback-errors/eac3-playback"


def crc16_eac3(data: bytes) -> int:
    c = 0
    for v in data:
        c ^= v << 8
        for _ in range(8):
            c = ((c << 1) ^ (0x8005 if c & 0x8000 else 0)) & 0xFFFF
    return c


def patch_crc(frame: bytearray) -> bytes:
    frame[-2] = 0
    frame[-1] = 0
    c = crc16_eac3(frame[2:-2])
    frame[-2] = (c >> 8) & 0xFF
    frame[-1] = c & 0xFF
    assert crc16_eac3(frame[2:]) == 0
    return bytes(frame)


class BitWriter:
    def __init__(self):
        self.bits: list[int] = []

    def put(self, val: int, width: int):
        for i in range(width - 1, -1, -1):
            self.bits.append((val >> i) & 1)
        return self

    def zeros(self, width: int):
        self.bits.extend([0] * width)
        return self

    def to_bytes(self, total_bytes: int) -> bytearray:
        assert len(self.bits) <= (total_bytes - 2) * 8
        out = bytearray(total_bytes)
        for idx, b in enumerate(self.bits):
            if b:
                out[idx >> 3] |= 1 << (7 - (idx & 7))
        return out


def build_blksw_frame(blksw_pattern: list[tuple[int, int]], absexp: int = 8, *, converted: bool = False, frmsizecod: int = 20, mixing: bool = False, blkid: bool = True) -> bytes:
    """Author a 6-block 48 kHz stereo E-AC-3 syncframe with `blkswe = 1` and per-block
    short-window switching flags `blksw[ch]`."""
    blocks = len(blksw_pattern)
    code = {1: 0, 2: 1, 3: 2, 6: 3}[blocks]
    frame_bytes = 1024
    frmsiz = frame_bytes // 2 - 1
    w = BitWriter()
    w.put(0x0B77, 16)
    w.put(2 if converted else 0, 2)  # stream type
    w.put(0, 3)  # substreamid = 0
    w.put(frmsiz, 11)
    w.put(0, 2)  # fscod = 0 (48 kHz)
    w.put(code, 2)  # numblkscod
    w.put(2, 3)  # acmod = 2 (stereo)
    w.put(0, 1)  # lfeon = 0
    w.put(16, 5)  # bsid = 16
    w.put(31, 5)  # dialnorm
    w.put(0, 1)  # compre
    w.put(int(mixing), 1)  # converted stereo has no additional mixing fields
    assert not mixing or converted
    w.put(0, 1)  # infomdate
    if converted:
        if blocks != 6: w.put(int(blkid), 1)
        if blocks == 6 or blkid: w.put(frmsizecod, 6)
    elif blocks != 6:
        w.put(0, 1)  # convsync
    w.put(0, 1)  # addbsie
    # audfrm:
    if blocks == 6:
        w.put(1, 1)  # expstre = 1 (PerBlock)
        w.put(0, 1)  # ahte = 0
    w.put(0, 2)  # snroffststr = 0 (frame-wide)
    w.put(0, 1)  # transproce = 0
    w.put(1, 1)  # blkswe = 1
    w.put(1, 1)  # dithflage = 1
    w.put(0, 1)  # bamode = 0
    w.put(0, 1)  # frmfgaincode = 0
    w.put(0, 1)  # dbaflde = 0
    w.put(0, 1)  # skipflde = 0
    w.put(0, 1)  # spxattene = 0
    w.put(0, 1).zeros(blocks-1)  # coupling off
    w.put(3, 2).put(3, 2)  # blk 0: D45, D45
    for _ in range(blocks-1):
        w.put(0, 2).put(0, 2)  # blk 1..5: Reuse
    if not converted:
        if blocks != 6: w.put(1, 1)  # convexpstre
        w.zeros(10)  # convexpstr
    w.put(18, 6).put(4, 4)  # frmcsnroffst + frmfsnroffst
    if blocks != 1: w.put(0, 1)  # blkstrtinfoe = 0
    for blk in range(blocks):
        b0, b1 = blksw_pattern[blk]
        w.put(b0, 1).put(b1, 1)  # blksw[0..2]
        w.put(0, 1).put(0, 1)  # dithflag[0..2] = 0
        w.put(0, 1)  # dynrnge = 0
        w.put(0, 1)  # spx off
        if blk == 0:
            w.zeros(4)  # rematflg[0..4] = 0
            w.put(0, 6).put(0, 6)  # bwcod[0..2] = 0 (end = 73)
            for _ in range(2):
                w.put(absexp, 4)
                for _ in range(6):
                    w.put(62, 7)
                w.put(0, 2)  # gainrng
        else:
            w.put(0, 1)  # rematstr = 0
        if not converted: w.put(0, 1)  # convsnroffste = 0
        for i in range(974):
            w.put((i * 5 + blk + 1) & 1, 1)
    return patch_crc(w.to_bytes(frame_bytes))


def build_spx_atten_frame(atten_codes: list[int | None] | None, *, blend: int = 31, dependent: bool = False, channel_map: int = 0x0200, gains=None) -> bytes:
    """Author a 6-block 48 kHz stereo E-AC-3 syncframe with Spectral Extension (`spxinu = 1`,
    wrapping copy region `37..61` into `61..97` across 3 subbands so wrap boundaries occur at
    `bnd = 0` and `bnd = 2`) and optional `spxattene = 1` (`spxattncod[ch]`)."""
    frame_bytes = 1024
    frmsiz = frame_bytes // 2 - 1
    w = BitWriter()
    w.put(0x0B77, 16)
    w.put(int(dependent), 2)  # independent or dependent
    w.put(0, 3)  # substreamid = 0
    w.put(frmsiz, 11)
    w.put(0, 2)  # fscod = 0 (48 kHz)
    w.put(3, 2)  # numblkscod = 3 (6 blocks)
    w.put(2, 3)  # acmod = 2 (stereo)
    w.put(0, 1)  # lfeon = 0
    w.put(16, 5)  # bsid = 16
    w.put(31, 5)  # dialnorm
    w.put(int(dependent), 1)  # final dependent descriptor
    if dependent:
        w.put(0, 8).put(1, 1).put(channel_map, 16)
    w.put(0, 1)  # mixmdate
    w.put(0, 1)  # infomdate
    w.put(0, 1)  # addbsie
    # audfrm:
    w.put(1, 1)  # expstre = 1 (PerBlock)
    w.put(0, 1)  # ahte = 0
    w.put(0, 2)  # snroffststr = 0 (frame-wide)
    w.put(0, 1)  # transproce = 0
    w.put(0, 1)  # blkswe = 0
    w.put(1, 1)  # dithflage = 1
    w.put(0, 1)  # bamode = 0
    w.put(0, 1)  # frmfgaincode = 0
    w.put(0, 1)  # dbaflde = 0
    w.put(0, 1)  # skipflde = 0
    w.put(1 if atten_codes is not None else 0, 1)  # spxattene
    w.put(0, 1).zeros(5)  # coupling off
    w.put(3, 2).put(3, 2)  # blk 0: D45, D45
    for _ in range(5):
        w.put(0, 2).put(0, 2)  # blk 1..5: Reuse
    if not dependent: w.zeros(10)  # convexpstr
    w.put(18, 6).put(4, 4)  # frmcsnroffst + frmfsnroffst
    if atten_codes is not None:
        for code in atten_codes:
            if code is None:
                w.put(0, 1)
            else:
                w.put(1, 1).put(code, 5)
    w.put(0, 1)  # blkstrtinfoe = 0
    for blk in range(6):
        w.put(0, 1).put(0, 1)  # dithflag = 0
        gain = None if gains is None else gains[blk]
        w.put(int(gain is not None), 1)
        if gain is not None: w.put(gain, 8)
        if blk == 0:
            w.put(1, 1)  # spxinu = 1
            w.put(1, 1).put(1, 1)  # chinspx[0..2] = 1, 1
            w.put(1, 2)  # spxstrtf = 1 (copy_first = 37)
            w.put(1, 3)  # spxbegf = 1 -> begin = 3 (first = 61)
            w.put(1, 3)  # spxendf = 1 -> end = 6 (last = 97)
            w.put(1, 1)  # spxbndstrce = 1
            w.put(0, 1).put(1, 1)  # spxbndstrc[4..6] = 0, 1 -> bands = [12, 24], wraps[1] = true!
            for _ in range(2):
                w.put(blend, 5)  # spxblnd; 31 retains the original signal-only authoring
                w.put(0, 2)  # mstrspxco = 0
                for _ in range(2):
                    w.put(5, 4)  # spxcoexp = 5
                    w.put(2, 2)  # spxcomant = 2
            w.zeros(3)  # rematflg[0..3] = 0
            for _ in range(2):
                w.put(8, 4)  # absexp = 8
                for _ in range(5):
                    w.put(62, 7)
                w.put(0, 2)  # gainrng
            if not dependent: w.put(0, 1)  # convsnroffste = 0
        else:
            w.put(0, 1)  # spxstre = 0
            w.put(0, 1).put(0, 1)  # spxcoe[0..2] = 0, 0
            w.put(0, 1)  # rematstr = 0
            if not dependent: w.put(0, 1)  # convsnroffste = 0
        for i in range(830):
            w.put((i * 7 + blk + 3) & 1, 1)
    return patch_crc(w.to_bytes(frame_bytes))


def mutate_reduced_rate(ec3_bytes: bytes, fscod: int, fscod2: int) -> bytes:
    """Convert an E-AC-3 stream encoded at base rate_code `fscod` (48, 44.1, or 32 kHz)
    to reduced sample rate (`fscod = 3, fscod2 = fscod`: 24, 22.05, or 16 kHz)
    with updated full-frame CRC2."""
    out = bytearray()
    pos = 0
    while pos < len(ec3_bytes):
        frmsiz = ((ec3_bytes[pos + 2] & 0x07) << 8) | ec3_bytes[pos + 3]
        fb = (frmsiz + 1) * 2
        frame = bytearray(ec3_bytes[pos : pos + fb])
        assert (frame[4] >> 6) == fscod
        assert ((frame[4] >> 4) & 3) == 3  # numblkscod == 3 (6 blocks)
        frame[4] = (frame[4] & 0x0F) | (3 << 6) | (fscod2 << 4)
        out.extend(patch_crc(frame))
        pos += fb
    return bytes(out)


def generate() -> dict[str, bytes]:
    OUT.mkdir(parents=True, exist_ok=True)
    files: dict[str, bytes] = {}
    cases = []

    # 1. Container tracks (MP4 ec-3 and Matroska A_EAC3) with 6-block E-AC-3 (48 kHz stereo)
    for ext in ["mp4", "mkv"]:
        path = OUT / f"stereo-48k.{ext}"
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=gray:s=16x16:r=50:d=0.064",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s=48000:d=0.064",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "libvpx-vp9" if ext == "mkv" else "libx264",
                *(["-lossless", "1"] if ext == "mkv" else ["-crf", "0", "-pix_fmt", "yuv420p"]),
                "-threads",
                "1",
                "-c:a",
                "eac3",
                "-b:a",
                "192k",
                "-t",
                "0.064",
                *(["-movflags", "+faststart"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # Container 5.1 E-AC-3 (MP4 and MKV)
    for ext in ["mp4", "mkv"]:
        path = OUT / f"surround-51.{ext}"
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.3*sin(2*PI*440*t)|0.25*sin(2*PI*550*t)|0.2*sin(2*PI*660*t)|0.3*sin(2*PI*80*t)|0.15*sin(2*PI*880*t)|0.1*sin(2*PI*1100*t):s=48000:d=0.064",
                "-vn",
                "-ch_layout",
                "5.1",
                "-c:a",
                "eac3",
                "-b:a",
                "448k",
                "-t",
                "0.064",
                *(["-movflags", "+faststart"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # Container with 2-block syncframes (numblkscod = 1) in MKV and MP4
    for ext in ["mp4", "mkv"]:
        path = OUT / f"blocks-2.{ext}"
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.032",
                "-vn",
                "-c:a",
                "eac3",
                "-b:a",
                "3024k",
                *(["-movflags", "+faststart", "-use_editlist", "0"] if ext == "mp4" else []),
                str(path),
            ],
            check=True,
        )
        files[path.name] = path.read_bytes()

    # 2. Elementary .ec3 streams for 1, 2, 3, and 6 blocks per syncframe
    for blocks, bitrate in [(6, "192k"), (3, "1536k"), (2, "3024k"), (1, "6144k")]:
        name = f"blocks-{blocks}.ec3"
        path = OUT / name
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s=48000:d=0.064",
                "-c:a",
                "eac3",
                "-b:a",
                bitrate,
                str(path),
            ],
            check=True,
        )
        raw = path.read_bytes()
        numblkscod = (raw[4] >> 4) & 3
        assert [1, 2, 3, 6][numblkscod] == blocks
        ref = subprocess.check_output(
            ["ffmpeg", "-v", "error", "-i", str(path), "-f", "f32le", "-"]
        )
        ref_name = f"blocks-{blocks}.f32le"
        (OUT / ref_name).write_bytes(ref)
        files[name] = raw
        files[ref_name] = ref
        cases.append(
            {
                "name": f"blocks-{blocks}",
                "file": name,
                "reference": ref_name,
                "blocks": blocks,
                "sample_rate": 48000,
                "channels": 2,
                "pcm_frames": len(ref) // 8,
            }
        )

    # 3. Reduced sample rates (fscod = 3, fscod2 = 0 -> 24000, 1 -> 22050, 2 -> 16000)
    # Derived from a 2-frame (6 blocks each) stereo stream at matching base rate (48000, 44100, 32000 Hz)
    # Header probes only: packed mantissas are NOT reallocated for half rate.
    # Valid owned PCM acceptance is authored by generate_eac3_reduced_allocation.py.
    for fscod2, base_rate, rate in [(0, 48000, 24000), (1, 44100, 22050), (2, 32000, 16000)]:
        name = f"reduced-{rate}.ec3"
        base_raw = subprocess.check_output(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                f"aevalsrc=0.25*sin(2*PI*440*t)|0.2*sin(2*PI*660*t):s={base_rate}:d=0.064",
                "-frames:a",
                "2",
                "-c:a",
                "eac3",
                "-b:a",
                "192k",
                "-f",
                "eac3",
                "-",
            ]
        )
        data = mutate_reduced_rate(base_raw, fscod2, fscod2)
        (OUT / name).write_bytes(data)
        files[name] = data
        cases.append(
            {
                "name": f"reduced-{rate}",
                "file": name,
                "blocks": 6,
                "sample_rate": rate,
                "channels": 2,
                "pcm_frames": 2 * 1536,
            }
        )

    # 4. Block switching (blkswe = 1) and Spectral Extension boundary attenuation (spxattene = 1)
    blksw_raw = build_blksw_frame([(1, 0), (1, 1), (0, 1), (1, 0), (0, 0), (1, 1)], absexp=8) * 2
    blksw_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=blksw_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "blksw.ec3").write_bytes(blksw_raw)
    (OUT / "blksw.f32le").write_bytes(blksw_ref)
    files["blksw.ec3"] = blksw_raw
    files["blksw.f32le"] = blksw_ref
    cases.append(
        {
            "name": "blksw",
            "file": "blksw.ec3",
            "reference": "blksw.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(blksw_ref) // 8,
        }
    )

    spx_no_atten_raw = build_spx_atten_frame(None) * 2
    spx_no_atten_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=spx_no_atten_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "spx-no-atten.ec3").write_bytes(spx_no_atten_raw)
    (OUT / "spx-no-atten.f32le").write_bytes(spx_no_atten_ref)
    files["spx-no-atten.ec3"] = spx_no_atten_raw
    files["spx-no-atten.f32le"] = spx_no_atten_ref
    cases.append(
        {
            "name": "spx-no-atten",
            "file": "spx-no-atten.ec3",
            "reference": "spx-no-atten.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(spx_no_atten_ref) // 8,
        }
    )

    spx_atten_raw = build_spx_atten_frame([15, 31]) * 2
    spx_atten_ref = subprocess.run(
        ["ffmpeg", "-v", "error", "-f", "eac3", "-i", "pipe:0", "-f", "f32le", "-"],
        input=spx_atten_raw,
        capture_output=True,
        check=True,
    ).stdout
    (OUT / "spx-atten.ec3").write_bytes(spx_atten_raw)
    (OUT / "spx-atten.f32le").write_bytes(spx_atten_ref)
    files["spx-atten.ec3"] = spx_atten_raw
    files["spx-atten.f32le"] = spx_atten_ref
    cases.append(
        {
            "name": "spx-atten",
            "file": "spx-atten.ec3",
            "reference": "spx-atten.f32le",
            "blocks": 6,
            "sample_rate": 48000,
            "channels": 2,
            "pcm_frames": len(spx_atten_ref) // 8,
        }
    )

    # 5. Container rate mismatch control & reproducer (48 kHz E-AC-3 labeled as 32 kHz)
    mkv_48 = files["stereo-48k.mkv"]
    old_rate = b"\xb5\x88" + struct.pack(">d", 48000.0)
    new_rate = b"\xb5\x88" + struct.pack(">d", 32000.0)
    assert mkv_48.count(old_rate) == 1
    mismatch_mkv = mkv_48.replace(old_rate, new_rate)
    (OUT / "mismatch-rate.mkv").write_bytes(mismatch_mkv)
    files["mismatch-rate.mkv"] = mismatch_mkv

    mp4_48 = bytearray(files["stereo-48k.mp4"])
    mdhd_at = mp4_48.index(b"mdhd\x00\x00\x00\x00", mp4_48.index(b"soun") - 200)
    assert struct.unpack_from(">I", mp4_48, mdhd_at + 16)[0] == 48000
    struct.pack_into(">I", mp4_48, mdhd_at + 16, 32000)
    at = mp4_48.index(b"ec-3") + 28
    assert struct.unpack_from(">I", mp4_48, at)[0] == 48000 << 16
    struct.pack_into(">I", mp4_48, at, 32000 << 16)
    mismatch_mp4 = bytes(mp4_48)
    (OUT / "mismatch-rate.mp4").write_bytes(mismatch_mp4)
    files["mismatch-rate.mp4"] = mismatch_mp4

    manifest = {
        "provenance": (
            "Synthetic E-AC-3 (Dolby Digital Plus) regression fixtures covering MP4 (ec-3), "
            "Matroska (A_EAC3), elementary .ec3 streams, 1/2/3/6-block syncframes (numblkscod 0..3), "
            "and reduced sample rates (fscod=3, fscod2=0..2: 24000, 22050, 16000 Hz). "
            "FFmpeg is used only during offline fixture authoring; ordinary tests load saved files."
        ),
        "cases": cases,
        "sha256": {k: hashlib.sha256(v).hexdigest() for k, v in sorted(files.items())},
    }
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode()
    (OUT / "manifest.json").write_bytes(manifest_bytes)
    files["manifest.json"] = manifest_bytes
    return files


if __name__ == "__main__":
    generate()

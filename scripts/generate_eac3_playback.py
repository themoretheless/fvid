"""Shared E-AC-3 bitstream helpers; FFmpeg reference generation lives in benchmark_eac3_playback_reference.py."""

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

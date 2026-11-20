#!/usr/bin/env python3
"""Explicit offline ISO spatial decoder audit on the same LD5151 core PCM.

External reference sources/media are never copied into repository fixtures.
Builds the original spatial parser and processing chain, with raw-float file IO.
"""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_SHA = 'b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
WAVE_SHA = 'a2732ae529d50f0ce7e6e6258911bf2985846e02df19ce67939b3131252c2e9f'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def compare(a, b):
    x, y = array.array('f'), array.array('f')
    x.frombytes(a.read_bytes())
    y.frombytes(b.read_bytes())
    if len(x) != len(y) or len(x) != 2820*512*6:
        raise ValueError('spatial output geometry differs')
    total, maximum = [0.]*6, [0.]*6
    for i, (u, v) in enumerate(zip(x, y)):
        if not math.isfinite(u) or not math.isfinite(v):
            raise ValueError('nonfinite spatial output')
        delta, channel = u-v, i % 6
        total[channel] += delta*delta
        maximum[channel] = max(maximum[channel], abs(delta))
    return dict(scalars=len(x), per_channel=[dict(
        rmse=math.sqrt(total[c]/(len(x)//6)), max_abs_error=maximum[c])
        for c in range(6)])


def transport(records, syntax):
    raw = records.read_bytes()
    at = 8

    def payload():
        nonlocal at
        n = struct.unpack_from('<I', raw, at)[0]
        at += 4
        value = raw[at:at+n]
        at += n
        return value

    bits = ''.join(f'{v:08b}' for v in payload())
    size = int(bits[34:42], 2)
    if not 0 < size < 127:
        raise ValueError('source-specific SSC header geometry')
    ssc = int(bits[42:42+size*8], 2).to_bytes(size, 'big')
    count = struct.unpack_from('<I', raw, at)[0]
    at += 4
    rows = [json.loads(s) for s in syntax.read_text().splitlines()]
    frames = []
    for row in rows:
        bits = ''.join(f'{v:08b}' for v in payload())
        pos = row['core_bits']
        end = pos+(len(bits)-pos)//8*8
        before = len(frames)
        while pos < end:
            kind = int(bits[pos:pos+4], 2)
            pos += 4
            if kind == 0:
                break
            if kind == 1:
                if bits[pos:pos+4] != '0000' or any(
                        bits[i:i+8] != '10100101' for i in range(pos+4, end, 8)):
                    raise ValueError('unexpected source fill-data suffix')
                break
            if kind != 3:
                raise ValueError('expected length-delimited spatial extension')
            length = int(bits[pos:pos+4], 2)
            pos += 4
            if length == 15:
                add = int(bits[pos:pos+8], 2)
                pos += 8
                length += add
                if add == 255:
                    length += int(bits[pos:pos+16], 2)
                    pos += 16
            child = bits[pos:pos+length*8]
            pos += length*8
            if len(child) != length*8 or child[:8] != '10010011':
                raise ValueError('expected complete frame-only LD spatial message')
            frames.append(int(child[8:], 2).to_bytes(length-1, 'big'))
        if len(frames) != before+1:
            raise ValueError('expected exactly one spatial frame per source AU')
    if at != len(raw) or count != len(frames) or count != 2820:
        raise ValueError('source-specific spatial AU geometry differs')
    return bytes([size])+ssc, struct.pack('<I', count)+b''.join(
        struct.pack('<I', len(f))+f for f in frames)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for arg in ['archive', 'records', 'syntax', 'core-pcm', 'owned-real',
                'owned-complex', 'reference-wav', 'output-dir', 'report']:
        p.add_argument('--'+arg, type=Path, required=True)
    a = p.parse_args()
    if sys.byteorder != 'little':
        raise ValueError('reference harness requires little-endian float transport')
    if sha(a.archive) != ARCHIVE_SHA or sha(a.reference_wav) != WAVE_SHA:
        raise ValueError('external reference pin differs')
    inputs = [a.archive, a.records, a.syntax, a.core_pcm,
              a.owned_real, a.owned_complex, a.reference_wav]
    pins = {str(v.resolve()): sha(v) for v in inputs}
    out = a.output_dir.resolve()
    if (out.is_relative_to(ROOT/'tests/fixtures')
            or a.report.resolve().is_relative_to(ROOT/'tests/fixtures')
            or a.report.resolve() in [v.resolve() for v in inputs]
            or any(v.resolve().is_relative_to(out) for v in inputs)):
        raise ValueError('research outputs must be separate from originals and fixtures')
    core = array.array('f')
    core.frombytes(a.core_pcm.read_bytes())
    if len(core) != 2820*512 or any(not math.isfinite(v) for v in core):
        raise ValueError('core PCM geometry differs')
    out.mkdir(parents=True, exist_ok=True)
    cases, members = [], {}
    driver = ROOT/'scripts/support/aac_ld_mps_original_spatial_probe.c'
    with tempfile.TemporaryDirectory(prefix='fvid-original-spatial-') as tmp:
        work = Path(tmp)
        # Validate the original and all parsed core boundaries against the
        # qualified bit-prefix transform before extracting ancillary messages.
        subprocess.run([sys.executable, str(ROOT/'scripts/isolate_aac_ld_mps_core.py'),
                        str(a.records), str(a.syntax), str(work/'core.records'),
                        '--report', str(work/'isolation.json')], check=True)
        header, frames = transport(a.records, a.syntax)
        (work/'ssc.bin').write_bytes(header)
        (work/'frames.bin').write_bytes(frames)
        with zipfile.ZipFile(a.archive) as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/', 'mp4spatialdec/include/')) and name.endswith(('.c', '.h')):
                    if '..' in Path(name).parts:
                        raise ValueError('archive traversal')
                    data = z.read(name)
                    dest = work/name
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(data)
                    members[name] = hashlib.sha256(data).hexdigest()
        src = work/'mp4spatialdec/src'
        (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n'
            'AFILE *AFopnWrite(const char*,int,int,long,double,void*);\n'
            'void AFclose(AFILE*);\nlong AFfWriteData(AFILE*,const float*,long);\n')
        (work/'decl.h').write_text('void myexit(char*);\n')
        (work/'precise.c').write_text('#include "sac_sbrconst.h"\n#undef PI\n'
            '#define PI 3.141592653589793238462643383279502884\n'
            '#include "sac_polyphase.c"\n')
        for control in ['original', 'precise']:
            sources = sorted(src.glob('sac_*.c'))
            if control == 'precise':
                sources.remove(src/'sac_polyphase.c')
                sources.append(work/'precise.c')
            exe = work/'probe'
            command = ['cc', '-O2', '-DLD_MPS', '-include', str(work/'decl.h'),
                       '-ffunction-sections', '-fdata-sections',
                       '-Wl,-dead_strip' if sys.platform == 'darwin' else '-Wl,--gc-sections',
                       '-I'+str(work), '-I'+str(src), '-I'+str(src.parent/'include'),
                       str(driver), *map(str, sources), '-lm', '-pthread', '-o', str(exe)]
            build = subprocess.run(command, capture_output=True, text=True, timeout=60)
            (out/(control+'-build.log')).write_text(build.stdout+build.stderr)
            build.check_returncode()
            for number, mode in enumerate(['real', 'complex']):
                pcm = out/(control+'-'+mode+'.f32le')
                run = subprocess.run([str(exe), str(work/'ssc.bin'),
                    str(work/'frames.bin'), str(a.core_pcm.resolve()), str(pcm), str(number)],
                    capture_output=True, text=True, timeout=60)
                (out/(control+'-'+mode+'.log')).write_text(run.stdout+run.stderr)
                run.check_returncode()
                own = a.owned_real if mode == 'real' else a.owned_complex
                metrics = compare(pcm, own)
                wave = subprocess.run([sys.executable,
                    str(ROOT/'scripts/compare_aac_reference_pcm.py'),
                    str(a.reference_wav), str(pcm)], capture_output=True, text=True, check=True)
                cases.append(dict(control=control, mode=mode, vs_owned=metrics,
                                  vs_original_wave=json.loads(wave.stdout), output_sha256=sha(pcm)))
                print(control, mode, 'vs_owned worst RMS',
                      max(c['rmse'] for c in metrics['per_channel']), flush=True)
    if any(sha(Path(path)) != digest for path, digest in pins.items()):
        raise ValueError('research inputs changed during audit')
    report = dict(conformance_verdict=None, complete=True, inputs=pins,
        driver_sha256=sha(driver), script_sha256=sha(Path(__file__)),
        reference_members=members, cases=cases, explicit_lag=0, gain_fit=False,
        scope='Same supplied mono core PCM through unmodified ISO spatial parser, '
        'matrix assembly, temporal processing, decorrelation and QMF chain. '
        'Raw float file adapter only. Separate precise-PI polyphase control. '
        'Not independent AAC core decoding or complete-profile conformance.')
    a.report.parent.mkdir(parents=True, exist_ok=True)
    a.report.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()

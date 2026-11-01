#!/usr/bin/env python3
"""Offline independent ISO AAC-LD core decoding of pinned isolated LD5151.

External reference build, never a production fallback or ordinary test.
Requires make, a C/C++ compiler and the separately fetched original archive.
"""
import argparse
import array
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import math
import os
from pathlib import Path
import shlex
import struct
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_SHA = 'b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
CORE_RECORD_SHA = '11df174bf1aa8387c583c487c51c4159f5a58b059b1d0dbf3a9009d532cd3053'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pack(bits):
    return int(bits+'0'*(-len(bits) % 8), 2).to_bytes((len(bits)+7)//8, 'big')


def latm(raw):
    at = 8

    def word():
        nonlocal at
        value = struct.unpack_from('<I', raw, at)[0]
        at += 4
        return value

    def payload():
        nonlocal at
        n = word()
        value = raw[at:at+n]
        at += n
        return value

    # Pinned mono LD ep0 config: strip only the disabled 0x7cc extension and
    # padding. Version1 carries the exact 22 core ASC bits. Legacy reference
    # transport otherwise skips ASC remainder twice; no AU bits are changed.
    asc = ''.join(f'{v:08b}' for v in payload())[:22]
    count = word()
    output = bytearray(b'.mp4\nASS\0')  # ISO bit-file wrapper, not an MP4 container.
    for index in range(count):
        packet = payload()
        bits = '0' if index == 0 else '1'
        if index == 0:
            bits += ('10'+'00'+f'{0:08b}'+'1'+f'{0:06b}'+f'{0:04b}'
                     +f'{0:03b}'+'00'+f'{len(asc):08b}'+asc+'000'+f'{255:08b}'+'00')
        n = len(packet)
        while n >= 255:
            bits += f'{255:08b}'
            n -= 255
        bits += f'{n:08b}'+''.join(f'{v:08b}' for v in packet)
        body = pack(bits)
        if len(body) >= 8192:
            raise ValueError('LOAS admission bound')
        output += pack(f'{0x2b7:011b}'+f'{len(body):013b}')+body
    if at != len(raw) or count != 2820:
        raise ValueError('pinned core geometry differs')
    return output


def run(command, cwd, log, env=None):
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True,
                            text=True, timeout=60)
    log.write_text(result.stdout+result.stderr)
    result.check_returncode()
    return result.stdout


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['archive', 'core-records', 'owned-core', 'output-dir', 'report']:
        p.add_argument('--'+name, type=Path, required=True)
    a = p.parse_args()
    if sys.byteorder != 'little':
        raise ValueError('little-endian float research transport required')
    if sha(a.archive) != ARCHIVE_SHA or sha(a.core_records) != CORE_RECORD_SHA:
        raise ValueError('external reference/input pin differs')
    inputs = [a.archive, a.core_records, a.owned_core]
    pins = {str(v.resolve()): sha(v) for v in inputs}
    out = a.output_dir.resolve()
    if (out.is_relative_to(ROOT/'tests/fixtures')
            or a.report.resolve().is_relative_to(ROOT/'tests/fixtures')
            or a.report.resolve() in [v.resolve() for v in inputs]
            or any(v.resolve().is_relative_to(out) for v in inputs)):
        raise ValueError('research outputs must be separate from inputs and fixtures')
    out.mkdir(parents=True, exist_ok=True)
    stream = out/'core-reference.ass'
    stream.write_bytes(latm(a.core_records.read_bytes()))
    adapter = ROOT/'scripts/support/aac_ld_original_core_io.c'
    with tempfile.TemporaryDirectory(prefix='fvid-iso-aac-core-') as tmp:
        work = Path(tmp)
        with zipfile.ZipFile(a.archive) as outer:
            with zipfile.ZipFile(io.BytesIO(outer.read('audio.zip'))) as inner:
                for item in inner.infolist():
                    if item.is_dir() or not item.filename.startswith('audio/natural/'):
                        continue
                    if '..' in Path(item.filename).parts:
                        raise ValueError('archive traversal')
                    dest = work/item.filename
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    dest.write_bytes(inner.read(item))
        natural = work/'audio/natural'
        source = natural/'mp4AudVm_Rewrite'
        # Ask the original makefiles for their decoder source/flag selection.
        # Compile those commands directly; the old GNU-sed linker-path helper
        # does not work with BSD sed. No archived source is edited.
        opts = ['ISOMP4=0', 'USE_AFSP=0', 'EPTOOLLIB=1', 'SBR=1', 'PAR=0',
                'LPC=0', 'MPEG12=0', 'AAC_ELD=0', 'PS=0',
                'OBJS_STREAMFILE=streamfile.o streamfile_helper.o '
                'streamfile_diagnose.o streamfile_fl4.o streamfile_latm.o '
                'streamfile_raw.o ep_convert.o']
        dry = run(['make', '-n', 'mp4audec', *opts], source, out/'make-plan.log')
        commands = []
        for line in dry.splitlines():
            if not line.startswith(('gcc ', 'cc ')) or ' -c ' not in line:
                continue
            args = shlex.split(line)
            ci = args.index('-c')
            candidates = [v for v in args[ci+1:] if v.endswith('.c')]
            if len(candidates) != 1 or not candidates[0].startswith('src_'):
                continue  # epTool is built separately with its original makefile.
            Path(source/args[args.index('-o')+1]).parent.mkdir(parents=True, exist_ok=True)
            args[0] = 'cc'
            commands.append(args)
        if len(commands) < 100:
            raise ValueError('incomplete original AAC decoder source selection')

        def compile_one(item):
            index, cmd = item
            run(cmd, source, out/f'compile-{index:03}.log')

        with ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(compile_one, enumerate(commands)))
        run(['make', '-j2', 'libeptool.a'], natural/'epTool', out/'eptool-build.log')
        # Reuse the original frontend's compilation flags for its generic
        # stream wrapper and the IO adapter; AAC decoder objects are unchanged.
        template = next(cmd for cmd in commands if 'src_frame/mp4audec.c' in cmd)
        def extra(src, obj, flags=()):
            cmd = template.copy()
            cmd[cmd.index('src_frame/mp4audec.c')] = str(src)
            cmd[cmd.index('-o')+1] = str(obj)
            run([*cmd, *flags], source, out/(obj.stem+'-build.log'))
        extra(source/'src_frame/mp4ifc.c', work/'mp4ifc.o')
        extra(source/'src_frame/audio.c', work/'audio-capture.o',
              ['-DAudioWriteData=OriginalAudioWriteData'])
        extra(adapter, work/'capture.o')
        extra(adapter, work/'no-capture.o', ['-DFVID_RESEARCH_NO_CAPTURE'])
        objects = [source/cmd[cmd.index('-o')+1] for cmd in commands
                   if not any(v == 'src_frame/audio.c' for v in cmd)]
        libraries = list((natural/'lib').glob('**/libeptool.a'))
        if len(libraries) != 1:
            raise ValueError('ambiguous original epTool library')
        exe = work/'aac-core-reference'
        run(['c++', *map(str, objects), str(work/'mp4ifc.o'),
             str(work/'audio-capture.o'), str(work/'capture.o'), str(libraries[0]),
             '-lm', '-o', str(exe)], source, out/'link.log')
        original_audio = next(source/cmd[cmd.index('-o')+1] for cmd in commands
                              if 'src_frame/audio.c' in cmd)
        baseline = work/'aac-core-file-reference'
        run(['c++', *map(str, objects), str(work/'mp4ifc.o'), str(original_audio),
             str(work/'no-capture.o'), str(libraries[0]), '-lm', '-o', str(baseline)],
            source, out/'baseline-link.log')
        run([str(baseline), '-d', '1', '-o', str(out/'baseline.au'), str(stream)],
            source, out/'baseline-decode.log')
        pcm = out/'original-core.f32le'
        env = dict(os.environ, FVID_RESEARCH_CORE_PCM=str(pcm))
        run([str(exe), '-d', '1', '-o', str(out/'original-core.au'), str(stream)],
            source, out/'decode.log', env)
        log = (out/'decode.log').read_text()
        if 'WARNING:' in log or 'ERROR' in log or 'EOF' not in log:
            raise ValueError('original decoder did not finish cleanly')
        if (out/'baseline.au').read_bytes() != (out/'original-core.au').read_bytes():
            raise ValueError('float capture changed original file output')
        wave = (out/'baseline.au').read_bytes()
        magic, offset, size, encoding, rate, channels = struct.unpack('>6I', wave[:24])
        if (magic != 0x2e736e64 or encoding != 3 or rate != 48000 or channels != 1
                or len(wave)-offset != 2820*512*2):
            raise ValueError('original PCM16 file geometry differs')
        native, original = array.array('f'), array.array('f')
        native.frombytes(a.owned_core.read_bytes())
        original.frombytes(pcm.read_bytes())
        if len(native) != 2820*512 or len(original) != len(native):
            raise ValueError('whole-stream core geometry differs')
        squared, maximum = 0., 0.
        for x, y in zip(native, original):
            if not math.isfinite(x) or not math.isfinite(y):
                raise ValueError('nonfinite core PCM')
            delta = x-y
            squared += delta*delta
            maximum = max(maximum, abs(delta))
        report = dict(complete=True, conformance_verdict=None, inputs=pins,
            archive_sha256=ARCHIVE_SHA, adapter_sha256=sha(adapter),
            script_sha256=sha(Path(__file__)), translation_units=len(commands),
            executable_sha256=sha(exe), original_pcm_sha256=sha(pcm),
            file_output_unchanged=True, original_file_sha256=sha(out/'baseline.au'),
            scalars=len(native), rmse=math.sqrt(squared/len(native)),
            max_abs_error=maximum, explicit_lag=0, gain_fit=False,
            scope='Unmodified ISO AAC-LD core and generic LATM frontend on exact '
            'isolated core AUs; mono float capture before original file rounding. '
            'No MP4, LPC or parametric decoding in this research build. '
            'Numerical control, not whole-profile conformance.')
    if any(sha(Path(path)) != digest for path, digest in pins.items()):
        raise ValueError('inputs changed during audit')
    a.report.parent.mkdir(parents=True, exist_ok=True)
    a.report.write_text(json.dumps(report, indent=2)+'\n')
    print('original AAC-LD core RMS', report['rmse'], 'max', maximum)


if __name__ == '__main__':
    main()

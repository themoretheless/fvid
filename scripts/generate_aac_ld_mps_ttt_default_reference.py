#!/usr/bin/env python3
"""Explicit ISO spatial reference for authored predictive-default movies.

Archive supplied externally; no network or foreign media. Ordinary tests read
the committed numeric oracle and never run this reference decoder.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT/'tests/fixtures/playback-errors/aac-ld-mps-ttt-default'
ARCHIVE_SHA = 'b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    args = parser.parse_args()
    if sha(args.archive) != ARCHIVE_SHA or sys.byteorder != 'little':
        raise ValueError('reference archive/transport pin')
    m = json.loads((OUT/'manifest.json').read_text())
    driver = ROOT/'scripts/support/aac_ld_mps_ttt_default_reference_probe.c'
    result, count_control, cases, members, inputs = bytearray(), bytearray(), [], {}, {}
    with tempfile.TemporaryDirectory(prefix='fvid-ttt-default-iso-') as tmp:
        work = Path(tmp)
        with zipfile.ZipFile(args.archive) as z:
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
        (work/'decl.h').write_text('void myexit(char*);\n')
        (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n'
            'AFILE *AFopnWrite(const char*,int,int,long,double,void*);\n'
            'void AFclose(AFILE*);\nlong AFfWriteData(AFILE*,const float*,long);\n')
        exe = work/'probe'
        command = ['cc', '-O2', '-DLD_MPS', '-include', str(work/'decl.h'),
            '-ffunction-sections', '-fdata-sections',
            '-Wl,-dead_strip' if sys.platform == 'darwin' else '-Wl,--gc-sections',
            '-I'+str(work), '-I'+str(src), '-I'+str(src.parent/'include'),
            str(driver), *map(str, sorted(src.glob('sac_*.c'))), '-lm', '-pthread',
            '-o', str(exe)]
        build = subprocess.run(command, capture_output=True, timeout=60)
        if build.returncode:
            raise RuntimeError(build.stderr.decode())
        # A diagnostic variant isolates the original TREE_525 last-region
        # count overwrite. Preserve the complete unmodified output separately.
        bitdec = src/'sac_bitdec.c'
        original = bitdec.read_bytes()
        start = original.index(b'  case TREE_525:', original.index(b'self->numDecorSignals'))
        end = original.index(b'  case TREE_7271:', start)
        block = original[start:end]
        old = b'self->numDecorSignals = 2;'
        replacement = b'self->numDecorSignals = self->tttConfig[0][0].mode == 0 ? 3 : 2;'
        if block.count(old) != 1:
            raise ValueError('TREE_525 diagnostic source pin')
        changed = original[:start]+block.replace(old, replacement)+original[end:]
        bitdec.write_bytes(changed)
        diagnostic = work/'count-control'
        command[-1] = str(diagnostic)
        build = subprocess.run(command, capture_output=True, timeout=60)
        bitdec.write_bytes(original)
        if build.returncode:
            raise RuntimeError(build.stderr.decode())
        patch = dict(file='mp4spatialdec/src/sac_bitdec.c',
            original_sha256=hashlib.sha256(original).hexdigest(),
            changed_sha256=hashlib.sha256(changed).hexdigest(),
            before=old.decode(), after=replacement.decode(),
            scope='TREE_525 case only; diagnostic, not a normative oracle')
        for c in m['cases']:
            config = bytes.fromhex(c['ssc'])
            if len(config) >= 127:
                raise ValueError('reference header bound')
            (work/'ssc.bin').write_bytes(bytes([len(config)])+config)
            core = ROOT/f"tests/fixtures/playback-errors/aac-eld-mps-core/tree{c['tree']}.pcm"
            if len(core.read_bytes()) != 3*512*2*4:
                raise ValueError('reference input core geometry')
            inputs[str(core.relative_to(ROOT))] = sha(core)
            for mode, name in enumerate(['real', 'complex']):
                outputs = []
                for variant in ['default', 'control']:
                    frames = [bytes.fromhex(v) for v in c[variant]['spatial']]
                    wire = struct.pack('<I', len(frames))+b''.join(
                        struct.pack('<I', len(f))+f for f in frames)
                    (work/'frames.bin').write_bytes(wire)
                    run = subprocess.run([str(exe), str(work/'ssc.bin'),
                        str(work/'frames.bin'), str(core), str(work/'out.f32le'), str(mode)],
                        capture_output=True, timeout=30)
                    if run.returncode:
                        raise RuntimeError(run.stderr.decode())
                    outputs.append((work/'out.f32le').read_bytes())
                if outputs[0] != outputs[1]:
                    raise ValueError('original default and coded unity differ: '+c['name'])
                if len(outputs[0]) != 1536*c['channels']*4:
                    raise ValueError('reference output geometry')
                value = outputs[0]
                # Original 7272 output is LF/RF/C/LFE/LS/RS/AL/AR.
                # Native AacFrame uses WAVE mask0x63f ordering instead.
                if c['tree'] == 4:
                    value = b''.join(value[at+ch*4:at+ch*4+4]
                        for at in range(0, len(value), 32)
                        for ch in [0, 1, 2, 3, 6, 7, 4, 5])
                entry = dict(name=c['name'], mode=name, offset=len(result),
                    bytes=len(outputs[0]), default_control_byte_identical=True)
                affected = c['tree'] == 2 and c.get('split') is not None and c['mode'] == 0
                if affected:
                    controls = []
                    for variant in ['default', 'control']:
                        frames = [bytes.fromhex(v) for v in c[variant]['spatial']]
                        (work/'frames.bin').write_bytes(struct.pack('<I', len(frames))+b''.join(
                            struct.pack('<I', len(f))+f for f in frames))
                        run = subprocess.run([str(diagnostic), str(work/'ssc.bin'),
                            str(work/'frames.bin'), str(core), str(work/'count.f32le'), str(mode)],
                            capture_output=True, timeout=30)
                        if run.returncode:
                            raise RuntimeError(run.stderr.decode())
                        controls.append((work/'count.f32le').read_bytes())
                    if controls[0] != controls[1] or len(controls[0]) != len(value):
                        raise ValueError('count diagnostic output/default equivalence')
                    if controls[0] == value:
                        raise ValueError('count diagnostic did not reproduce the discrepancy')
                    entry['count_control_offset'] = len(count_control)
                    entry['unmodified_reference_count_overwrite_reproducer'] = True
                    count_control += controls[0]
                cases.append(entry)
                result += value
    (OUT/'reference.f32le').write_bytes(result)
    (OUT/'count-control.f32le').write_bytes(count_control)
    (OUT/'oracle.json').write_text(json.dumps(dict(cases=cases, scope='Full original '
        'spatial decoder on authored core PCM and authored SSC/frames; both '
        'decorrelators, default and explicit-unity outputs byte-identical. '
        '7272 outputs explicitly reordered to WAVE mask0x63f. '
        'Four TREE_525 low0/high1 outputs additionally have a separately labelled '
        'decorrelator-count source diagnostic; original outputs remain intact. '
        'Not an AAC core oracle or complete-profile conformance.'), indent=2)+'\n')
    report = dict(count_control_source_patch=patch,
        count_control_sha256=sha(OUT/'count-control.f32le'), archive_sha256=ARCHIVE_SHA, reference_members=members,
        driver_sha256=sha(driver), generator_sha256=sha(Path(__file__)),
        manifest_sha256=sha(OUT/'manifest.json'), input_core_sha256=inputs,
        pcm_sha256=sha(OUT/'reference.f32le'), cases=len(cases),
        default_control_byte_identical=True, conformance_verdict=None)
    (ROOT/'docs/validation/aac-ld-mps-ttt-default-reference-2026-11-01.json').write_text(json.dumps(report, indent=2)+'\n')
    print('original default/unity identical:', len(cases), 'cases;', len(result)//4, 'PCM scalars')


if __name__ == '__main__':
    main()

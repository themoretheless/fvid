"""Explicit original/reference-initialization control, outside ordinary tests."""
import hashlib, json, struct, subprocess, sys, tempfile
from pathlib import Path
from zipfile import ZipFile
ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / 'tests/fixtures/playback-errors/aac-ld-mps-parameters'
def sha(b): return hashlib.sha256(b).hexdigest()
def run(archive):
    known = json.loads((ROOT / 'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text())
    archive_hash = sha(archive.read_bytes())
    assert archive_hash == known['archive_sha256']
    manifest = json.loads((CORPUS / 'manifest.json').read_text())
    packets = (CORPUS / 'packets.bin').read_bytes()
    reports, members = [], {}
    with tempfile.TemporaryDirectory(prefix='fvid-parameter-reference-') as tmp:
        work = Path(tmp)
        with ZipFile(archive) as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/', 'mp4spatialdec/include/', 'general/')) and name.endswith(('.c', '.h')):
                    assert '..' not in Path(name).parts
                    data = z.read(name); p = work / name
                    p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(data); members[name] = sha(data)
        src = work / 'mp4spatialdec/src'
        original = (src / 'sac_bitdec.c').read_bytes()
        before = b'int i1, i2, x1, xi, x2;'
        after = b'int i1=0, i2, x1, xi, x2;'
        assert original.count(before) == 1
        control = original.replace(before, after)
        (work / 'libtsp.h').write_text('typedef struct AFILE AFILE;\n')
        driver = ROOT / 'scripts/support/aac_ld_mps_parameters_reference_probe.c'
        for version in ['original-noninterpolated', 'initialized-anchor-control']:
            (work / 'parameter-reference.c').write_bytes(original if version.startswith('original') else control)
            records, expected, names = bytearray(), [], []
            for c in manifest['cases']:
                for i, f in enumerate(c['frames']):
                    # The original source has undefined behavior for a leading
                    # interpolation set. Do not execute that path or invent a
                    # deterministic original result. The next block defaults
                    # both histories before any temporal prediction.
                    if version.startswith('original') and 2 in f['modes']: continue
                    p = packets[f['offset']:f['offset'] + f['bytes']] + b'\x5a\xa5'
                    records.extend(struct.pack('<8I', {'CLD':0,'ICC':1,'CPC':2}[c['kind']], c['bands'], c['default'], len(f['slots']), f['independent'], len(p), i == 0, (23-c['bands'])//2))
                    records.extend(p); expected.append(f); names.append(c['name'] + ':' + str(i))
            (work / 'records.bin').write_bytes(records)
            exe = work / 'probe'
            args = ['cc', '-O2', '-DLD_MPS', '-ffunction-sections', '-fdata-sections', '-I', str(work), '-I', str(src), '-I', str(work / 'mp4spatialdec/include'), str(driver), str(src / 'sac_nlc_dec.c'), str(src / 'sac_bitinput.c')]
            args += [str(src / f'sac_nodes_{k}.c') for k in ['cld','icc','cpc','pt0']]
            args += ['-Wl,-dead_strip'] if sys.platform == 'darwin' else ['-Wl,--gc-sections']
            build = subprocess.run(args + ['-lm', '-o', str(exe)], capture_output=True, timeout=60)
            if build.returncode: raise RuntimeError(build.stderr.decode())
            result = subprocess.run([str(exe), str(work / 'records.bin')], capture_output=True, text=True, check=True, timeout=30)
            actual = [json.loads(s) for s in result.stdout.splitlines()]
            assert len(actual) == len(expected)
            for name, got, want in zip(names, actual, expected):
                assert got['bits'] == want['bits'] and got['rows'] == want['expected'] and got['parsed'] == want['parsed'], (version, name, got, want)
            reports.append(dict(control=version, matched=len(actual), input_sha256=sha(records), stdout_sha256=sha(result.stdout.encode()), names_sha256=sha(json.dumps(names).encode())))
    report = dict(archive_sha256=archive_hash, reference_members=members, driver_sha256=sha(driver.read_bytes()), original_sha256=sha(original), initialized_control_sha256=sha(control), initialization=dict(before=before.decode(), after=after.decode(), semantics_source=json.loads((ROOT / 'docs/validation/aac-ld-mps-parameters-later-source-2026-10-30.json').read_text()), note='Later original-author source initializes mapIndexData left anchor to zero. This harness does not include or compile FDK code.'), results=reports, scope='Stateful parameter indices only. No spatial PCM or playback/seek acceptance. Original leading-interpolation undefined behavior is not executed; all other blocks use unmodified ISO sources.')
    (ROOT / 'docs/validation/aac-ld-mps-parameters-reference-2026-10-30.json').write_text(json.dumps(report, indent=2) + '\n')
    print([(r['control'],r['matched']) for r in reports])
if __name__ == '__main__': run(Path(sys.argv[1]))

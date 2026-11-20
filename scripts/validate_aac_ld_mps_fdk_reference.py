#!/usr/bin/env python3
"""Optional, offline FDK cross-check of the two public ISO LD-QMF inputs.

Requires a separately obtained pinned AOSP archive, fetched ISO vectors, CMake,
clang++, and the owned aac_spatial_reference example. Never run by ordinary tests.
No source patches, concealment, delay fitting or conformance verdicts are used.
"""
import argparse
import array
import ast
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import struct
import sys
import tarfile
import tempfile

from compare_aac_reference_pcm import pcm16_wave
from fdk_reference_source import COMMIT, verify_source_archive

ROOT = Path(__file__).resolve().parents[1]
NAMES = ['er_eld_ldmps_ldqmf_1', 'er_eld_ldmps_ldqmf_2']
LIBRARIES = ['libAACdec', 'libPCMutils', 'libFDK', 'libSYS', 'libMpegTPDec',
             'libSBRdec', 'libArithCoding', 'libDRCdec', 'libSACdec']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args, log):
    with log.open('w') as output:
        result = subprocess.run([str(v) for v in args], stdout=output, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f'command failed; inspect {log}')


def native_floats(path, kind):
    result = array.array(kind, path.read_bytes())
    if sys.byteorder != 'little':
        result.byteswap()
    return result


def window_tables(source):
    text = (source/'libFDK/src/FDK_tools_rom.cpp').read_text()
    tables = {}
    for size in [320, 640]:
        body = re.search(r'const FIXP_PFT qmf_mpsldfb_'+str(size)+r'\[[^]]+\]\s*=\s*\{(.*?)\};', text, re.S).group(1)
        packed = [float(v) for v in re.findall(r'QTMFLLD\(([-+0-9.eE]+)\)', body)]
        if len(packed) != size:
            raise ValueError('FDK reference window size differs')
        table = [0.0]*size
        for k in range(size//5):
            for part in range(5):
                table[k+(size//5)*part] = packed[k*5+part]
        tables[size] = table
    path = ROOT/'crates/fvid-media/src/owned_aac/aac_ld_qmf_window.rs'
    native = ast.literal_eval('['+path.read_text().split('= [', 1)[1].split('];', 1)[0]+']')
    if len(native) != 640:
        raise ValueError('native reference window size differs')
    interpolation = max(abs(tables[320][i]-(tables[640][2*i]+tables[640][2*i+1])/2) for i in range(320))
    error64 = max(abs(native[i]-tables[640][639-i]) for i in range(640))
    error32 = max(abs((native[2*i]+native[2*i+1])/2-tables[320][319-i]) for i in range(320))
    # Only scalar-table rounding equivalence; no PCM conformance threshold.
    if interpolation >= 1e-7 or error64 >= 1e-10 or error32 >= 1e-7:
        raise ValueError('reference/native window coefficient mismatch')
    return dict(fdk320_coefficients=320, fdk640_coefficients=640,
                max_interpolation_error=interpolation,
                max_native64_vs_reversed_fdk640_error=error64,
                max_native32_vs_reversed_fdk320_error=error32,
                native_window_sha256=sha(path))


def metrics(a, b, channels, start):
    if len(a) != len(b) or len(a) % channels or start*channels >= len(a):
        raise ValueError('incompatible reference output geometry')
    error = energy = maximum = 0.0
    for i in range(start*channels, len(a)):
        if not math.isfinite(a[i]) or not math.isfinite(b[i]):
            raise ValueError('non-finite reference comparison')
        delta = a[i]-b[i]
        error += delta*delta
        energy += b[i]*b[i]
        maximum = max(maximum, abs(delta))
    count = len(a)-start*channels
    return dict(from_frame=start, compared_scalars=count, rmse=math.sqrt(error/count),
                max_abs_error=maximum,
                snr_db=10*math.log10(energy/error) if energy and error else None)


def admission_controls(executable, records, work):
    data = records.read_bytes()
    asc_length = struct.unpack_from('<I', data, 8)[0]
    prefix = data[:12+asc_length]
    invalid_au = prefix + struct.pack('<II', 1, 1) + b'\0'
    cases = [
        ('magic', b'WRONGMAG'+data[8:], 'invalid FVIDAAC1 magic'),
        ('empty-asc', b'FVIDAAC1'+struct.pack('<I', 0), 'admission bound'),
        ('truncated', data[:-1], 'truncated FVIDAAC1 payload'),
        ('trailing', data+b'X', 'trailing/error bytes'),
        ('decode-error', invalid_au, 'decode error=')]
    results = []
    for name, wire, reason in cases:
        path = work/(name+'.records')
        path.write_bytes(wire)
        result = subprocess.run([str(executable), str(path), str(work/(name+'.pcm'))],
                                capture_output=True, text=True)
        if result.returncode == 0 or reason not in result.stderr:
            raise ValueError(f'reference refusal failed for {name}: {result.stderr}')
        results.append(dict(name=name, status=result.returncode, stderr=result.stderr))
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-archive', type=Path, required=True)
    parser.add_argument('--vectors-dir', type=Path, required=True)
    parser.add_argument('--native-probe', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    source_pin = verify_source_archive(args.source_archive)
    # Verify public inputs against the inventory already qualified by the ISO
    # fetch tool. No private media or substitute codec parameter sets are used.
    inventory = json.loads((ROOT/'docs/validation/aac-alignment-iso-fetch-2026-10-31.json').read_text())
    hashes = {Path(row['member']).name: row['sha256'] for row in inventory['files']}
    for stem in NAMES:
        for suffix in ['.mp4', '.wav']:
            path = args.vectors_dir/(stem+suffix)
            if sha(path) != hashes[path.name]:
                raise ValueError('ISO conformance member checksum mismatch')
    out = args.output_dir.resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = dict(conformance_verdict=None, source_commit=COMMIT,
                  source_archive_sha256=source_pin['archive_sha256'],
                  source_git_tree=source_pin, fdk_qmf_requested='complex', source_files={}, cases=[])
    probe = args.native_probe.resolve()
    with tempfile.TemporaryDirectory(prefix='fdk-reference-', dir=out) as scratch:
        work = Path(scratch)
        source = work/'source'
        source.mkdir()
        with tarfile.open(args.source_archive) as archive:
            archive.extractall(source, filter='data')
        report['window_tables'] = window_tables(source)
        project = work/'project'
        project.mkdir()
        cmake = 'cmake_minimum_required(VERSION 3.18)\nproject(fvid_fdk_reference LANGUAGES CXX)\nset(CMAKE_CXX_STANDARD 11)\n'
        for lib in LIBRARIES:
            cmake += f'file(GLOB {lib}_SOURCES "${{FVID_FDK_SOURCE_DIR}}/{lib}/src/*.cpp")\n'
        cmake += 'add_library(fdk_reference STATIC\n' + ''.join(f'  ${{{lib}_SOURCES}}\n' for lib in LIBRARIES) + ')\n'
        for inc in sorted(source.glob('lib*/include')):
            cmake += f'target_include_directories(fdk_reference PUBLIC "${{FVID_FDK_SOURCE_DIR}}/{inc.parent.name}/include")\n'
        cmake += 'target_compile_definitions(fdk_reference PRIVATE SUPPRESS_BUILD_DATE_INFO)\n'
        cmake += 'add_executable(fdk_probe "${FVID_FDK_PROBE_SOURCE}")\ntarget_link_libraries(fdk_probe PRIVATE fdk_reference)\n'
        (project/'CMakeLists.txt').write_text(cmake)
        build = work/'build'
        run(['cmake', '-S', project, '-B', build, '-DCMAKE_BUILD_TYPE=Release',
             '-DCMAKE_CXX_COMPILER=clang++', f'-DFVID_FDK_SOURCE_DIR={source}',
             f'-DFVID_FDK_PROBE_SOURCE={ROOT}/scripts/support/aac_ld_mps_fdk_reference_probe.cpp'], out/'configure.log')
        run(['cmake', '--build', build, '-j', '4'], out/'build.log')
        for name in ['libFDK/src/qmf.cpp', 'libFDK/src/FDK_tools_rom.cpp',
                     'libFDK/src/FDK_qmf_domain.cpp', 'libAACdec/src/aacdecoder.cpp',
                     'libSACdec/src/sac_dec.cpp']:
            report['source_files'][name] = sha(source/name)
        for stem in NAMES:
            movie = args.vectors_dir/(stem+'.mp4')
            records = out/(stem+'.au-records')
            actual = out/(stem+'.native.f32le')
            external = out/(stem+'.fdk.s16le')
            run([probe, 'packets', movie, records], out/(stem+'.extract.log'))
            run([probe, 'complex', movie, actual], out/(stem+'.native.log'))
            run([build/'fdk_probe', records, external, '--qmf=complex'], out/(stem+'.fdk.log'))
            channels, rate, reference_pcm = pcm16_wave((args.vectors_dir/(stem+'.wav')).read_bytes())
            reference = [v/32768 for v in reference_pcm]
            fdk = [v/32768 for v in native_floats(external, 'h')]
            native = native_floats(actual, 'f')
            comparisons = []
            for label, a, b in [('FVid-vs-FDK', native, fdk), ('FDK-vs-ISO', fdk, reference)]:
                for start in [0, 512, 1024, 2048, 4096]:
                    comparisons.append(dict(pair=label, **metrics(a, b, channels, start)))
            report['cases'].append(dict(name=stem, rate=rate, channels=channels,
                input_mp4_sha256=sha(movie), records_sha256=sha(records),
                native_sha256=sha(actual), fdk_sha256=sha(external),
                external_status=(out/(stem+'.fdk.log')).read_text(), comparisons=comparisons))
            print(stem, 'decoded by both implementations', flush=True)
        report['protocol_admission'] = admission_controls(build/'fdk_probe', records, work)
    report['limitations'] = [
        'FDK is a separately built external fixed-point cross-check, not a normative whole-codec oracle.',
        'All windows use zero comparison lag. Later windows are diagnostic observations, not discarded errors in a pass/fail test.',
        'The large FDK/ISO single-rate discrepancy also needs a verified normative phase/timing procedure.',
        'This optional tool is not linked by production or invoked by ordinary tests.']
    (out/'report.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()

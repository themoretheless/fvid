#!/usr/bin/env python3
"""Owned mixed/deep ACT and non-zero slice-offset qualification streams.
Explicit SCM binaries required; ordinary tests never invoke these tools.
For --slice-offsets, apply scm_act_slice_offsets.patch to the encoder only.
For depth fixtures, apply scm_scc14_config.patch to the encoder configuration
validation only. The decoder remains the unmodified official SCM reference.
"""
import argparse, ast, hashlib, json, subprocess, tempfile
from pathlib import Path
from hevc_fixture_mp4 import mux


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--scm-encoder', type=Path, required=True)
    p.add_argument('--scm-decoder', type=Path, required=True)
    p.add_argument('--slice-offsets', action='store_true')
    a = p.parse_args()
    root = Path(__file__).resolve().parents[1]
    fixtures = root / 'tests/fixtures/playback-errors'
    tree = ast.parse((root/'scripts/generate_hevc_tiles_sample.py').read_text())
    base = next(ast.literal_eval(n.value) for n in tree.body if isinstance(n, ast.Assign)
                and any(isinstance(t, ast.Name) and t.id == 'config_text' for t in n.targets))
    base = base.replace('MaxCUChromaQpAdjustmentDepth : 0', 'MaxCUChromaQpAdjustmentDepth : -1').replace('SAO : 0', 'SAO : 1').replace('LoopFilterDisable : 1', 'LoopFilterDisable : 0')
    base += 'IntraBlockCopyEnabled : 0\nPaletteMode : 0\nColourTransform : 1\nHashBasedME : 0\n'
    pairs = [(8,8),(10,10),(8,10),(10,8)] if a.slice_offsets else [(8,10),(10,8),(14,14),(14,10),(10,14)]
    results = []
    for y_depth, c_depth in pairs:
        for parallel in [False, True]:
            width, height = (128,96) if parallel else (64,64)
            with tempfile.TemporaryDirectory(prefix='fvid-scc-act-depth-') as d:
                tmp = Path(d)
                cfg, src, stream, recon, oracle = [tmp/x for x in ['owned.cfg','owned.yuv','owned.hevc','recon.yuv','oracle.yuv']]
                deep = max(y_depth,c_depth)>10
                profile = 'high-throughput-SCC' if deep else 'main-SCC'
                config = base.replace('Profile : main-RExt',f'Profile : {profile}')
                if deep: config += 'MaxBitDepthConstraint : 14\nMaxChromaFormatConstraint : 444\nSCCHighThroughputFlag : 1\n'
                if parallel or deep: config += 'WaveFrontSynchro : 1\n'
                cfg.write_text(config)
                def sample(x,y,c):
                    x %= width
                    return ((x*3+y*5+((x//8+y//8)%2)*31)&127)+32+c*4
                src.write_bytes(bytes(min(255,sample(x-f*8,y,c)+f*7*((x//16+y//16)%3))
                                for f in range(3) for c in range(3) for y in range(height) for x in range(width)))
                subprocess.run([str(a.scm_encoder.resolve()),'-c',str(cfg),'-i',str(src),'-b',str(stream),'-o',str(recon),'-wdt',str(width),'-hgt',str(height),'-fr','25','-f','3','--InputBitDepth=8',f'--InternalBitDepth={y_depth}',f'--InternalBitDepthC={c_depth}','--InputChromaFormat=444'],check=True)
                # Uniform 16-bit storage retains each component's original precision.
                subprocess.run([str(a.scm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={y_depth}',f'--OutputBitDepthC={c_depth}','--SEIDecodedPictureHash=0'],check=True)
                assert recon.read_bytes() == oracle.read_bytes()
                storage = 2 if max(y_depth,c_depth)>8 else 1
                assert len(oracle.read_bytes()) == 3*width*height*3*storage
                stem = f'hevc-scc-act-{"slice" if a.slice_offsets else "depth"}-y{y_depth}-c{c_depth}'+('-parallel' if parallel else '')
                mp4 = mux(stream.read_bytes(),y_depth,width,height,chroma_format=3,chroma_depth=c_depth)
                (fixtures/f'{stem}.mp4').write_bytes(mp4)
                (fixtures/f'{stem}.yuv').write_bytes(oracle.read_bytes())
                results.append(dict(stem=stem,depths=[y_depth,c_depth],mp4_sha256=hashlib.sha256(mp4).hexdigest(),oracle_sha256=hashlib.sha256(oracle.read_bytes()).hexdigest()))
    report = dict(reference='HM-16.20+SCM-8.8',revision='cae35dba3dcf8e1b6121a70b397000af36301045',encoder_sha256=hashlib.sha256(a.scm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(a.scm_decoder.read_bytes()).hexdigest(),fixtures=results)
    if not a.slice_offsets: report['encoder_configuration_patch_sha256'] = hashlib.sha256((root/'scripts/scm_scc14_config.patch').read_bytes()).hexdigest()
    if a.slice_offsets: report['encoder_configuration_patch_sha256'] = hashlib.sha256((root/'scripts/scm_act_slice_offsets.patch').read_bytes()).hexdigest()
    (fixtures/f'hevc-scc-act-{"slice" if a.slice_offsets else "depth"}-oracle.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__ == '__main__': main()

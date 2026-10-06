#!/usr/bin/env python3
"""Owned SCC profile fixtures with SCC coding tools disabled.
HM generates base 4:4:4 samples and its oracle;
only PTL and inert SPS/PPS extension syntax are rewritten. No entropy changes.
Explicit generation only; ordinary tests consume saved bytes.
"""
import argparse, ast, re, subprocess, tempfile, hashlib, json
from pathlib import Path
from hevc_fixture_mp4 import mux

def unescape(data):
    return data.replace(b'\x00\x00\x03',b'\x00\x00')

def escape(data):
    out=bytearray();zeros=0
    for v in data:
        if zeros==2 and v<=3:out.append(3);zeros=0
        out.append(v);zeros=zeros+1 if v==0 else 0
    return bytes(out)

def rewrite(nal,empty_initializers=False,reserved_motion=False,boundary_disabled=False,current_capability=False):
    kind=(nal[0]>>1)&63
    if kind not in [32,33,34]:return nal
    raw=bytearray(unescape(nal[2:]))
    if kind in [32,33]:
        offset=4 if kind==32 else 1
        raw[offset]=(raw[offset]&0xe0)|9
        raw[offset+1:offset+5]=(1<<(31-9)).to_bytes(4,'big')
        if current_capability: raw[offset+6] |= 4  # max_14bit_constraint_flag for profile 9
    if kind in [33,34]:
        bits=''.join(f'{v:08b}' for v in raw);stop=bits.rfind('1');body=bits[:stop]
        assert body[-1]=='0','source must have no extensions'
        # range/multilayer/3D/SCC flags and four reserved bits.
        if kind==33:
            extension='10000' if current_capability else ('00110' if reserved_motion else ('00001' if boundary_disabled else '00000'))
        else:
            extension='0011' if empty_initializers else '000'
        body=body[:-1]+'1'+'00010000'+extension+'1'
        body+='0'*(-len(body)%8)
        raw=bytes(int(body[i:i+8],2) for i in range(0,len(body),8))
    return nal[:2]+escape(raw)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--hm-encoder',type=Path,required=True);p.add_argument('--hm-decoder',type=Path,required=True)
    p.add_argument('--boundary-disabled-only',action='store_true',help='Requires HM tools built with hm_scc_boundary_oracle.patch; generate boundary-disabled SCC streams only')
    p.add_argument('--current-capability-only',action='store_true',help='Generate inert SPS capability with current-picture references disabled in PPS')
    args=p.parse_args()
    if args.current_capability_only and args.boundary_disabled_only: p.error('select one fixture group')
    root=Path(__file__).resolve().parents[1]
    provenance=[]
    tree=ast.parse((root/'scripts/generate_hevc_tiles_sample.py').read_text())
    config=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='config_text' for t in n.targets))
    config=config.replace('MaxCUChromaQpAdjustmentDepth : 0','MaxCUChromaQpAdjustmentDepth : -1').replace('SAO : 0','SAO : 1').replace('LoopFilterDisable : 1','LoopFilterDisable : 0')
    for depth in [8,10]:
        for suffix,width,height in ([('',64,64),('-parallel',128,96)] if args.boundary_disabled_only or args.current_capability_only else [('',64,64),('-parallel',128,96),('-empty-initializers',64,64)]):
            with tempfile.TemporaryDirectory(prefix='fvid-scc-base-') as directory:
                tmp=Path(directory);cfg=tmp/'owned.cfg';src=tmp/'owned.yuv';stream=tmp/'owned.hevc';recon=tmp/'recon.yuv';oracle=tmp/'oracle.yuv'
                cfg.write_text(config+('WaveFrontSynchro : 1\n' if suffix=='-parallel' else ''))
                src.write_bytes(bytes(24+plane*19+(x*3+y*5+f*7)%112+(((x+f*2)//8+y//8)%2)*64 for f in range(3) for plane in range(3) for y in range(height) for x in range(width)))
                subprocess.run([str(args.hm_encoder.resolve()),'-c',str(cfg),'-i',str(src),'-b',str(stream),'-o',str(recon),'-wdt',str(width),'-hgt',str(height),'-fr','25','-f','3','--InputBitDepth=8',f'--InternalBitDepth={depth}','--InputChromaFormat=444'],check=True)
                subprocess.run([str(args.hm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True)
                assert oracle.read_bytes()==recon.read_bytes()
                nals=[n for n in re.split(b'\x00\x00\x00?\x01',stream.read_bytes()) if n]
                patched=b''.join(b'\x00\x00\x00\x01'+rewrite(n,suffix=='-empty-initializers',boundary_disabled=args.boundary_disabled_only,current_capability=args.current_capability_only) for n in nals)
                if args.current_capability_only:
                    stream.write_bytes(patched)
                    subprocess.run([str(args.hm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True)
                    assert oracle.read_bytes()==recon.read_bytes()
                stem=f'hevc-scc-{"current-capability" if args.current_capability_only else ("boundary-disabled" if args.boundary_disabled_only else "base")}{suffix}-rext{depth}';fixtures=root/'tests/fixtures/playback-errors'
                (fixtures/f'{stem}.mp4').write_bytes(mux(patched,depth,width,height,chroma_format=3))
                (fixtures/f'{stem}.yuv').write_bytes(oracle.read_bytes())
                provenance.append(dict(stem=stem,reference_sha256=hashlib.sha256(oracle.read_bytes()).hexdigest(),mp4_sha256=hashlib.sha256((fixtures/f'{stem}.mp4').read_bytes()).hexdigest()))
                if not args.boundary_disabled_only and not args.current_capability_only and depth==8 and suffix=='':
                    invalid=b''.join(b'\x00\x00\x00\x01'+rewrite(n,reserved_motion=True) for n in nals)
                    (fixtures/'hevc-scc-reserved-motion.mp4').write_bytes(mux(invalid,depth,width,height,chroma_format=3))
    if args.current_capability_only:
        report=dict(encoder_sha256=hashlib.sha256(args.hm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(args.hm_decoder.read_bytes()).hexdigest(),fixtures=provenance)
        (root/'tests/fixtures/playback-errors/hevc-scc-current-capability-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
    if args.boundary_disabled_only:
        report=dict(patch_sha256=hashlib.sha256((root/'scripts/hm_scc_boundary_oracle.patch').read_bytes()).hexdigest(),encoder_sha256=hashlib.sha256(args.hm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(args.hm_decoder.read_bytes()).hexdigest(),fixtures=provenance)
        (root/'tests/fixtures/playback-errors/hevc-scc-boundary-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()

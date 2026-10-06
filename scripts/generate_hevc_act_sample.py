#!/usr/bin/env python3
"""Owned SCC adaptive colour transform fixtures and official SCM reference pixels.
Explicit generation only. SCM-8.8 tag cae35dba (full revision in provenance).
Ordinary tests consume saved bytes without invoking reference tools.
"""
import argparse,ast,hashlib,json,subprocess,tempfile
from pathlib import Path
from hevc_fixture_mp4 import mux

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--scm-encoder',type=Path,required=True);p.add_argument('--scm-decoder',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1];fixtures=root/'tests/fixtures/playback-errors'
    tree=ast.parse((root/'scripts/generate_hevc_tiles_sample.py').read_text())
    config=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='config_text' for t in n.targets))
    config=config.replace('Profile : main-RExt','Profile : main-SCC').replace('MaxCUChromaQpAdjustmentDepth : 0','MaxCUChromaQpAdjustmentDepth : -1').replace('SAO : 0','SAO : 1').replace('LoopFilterDisable : 1','LoopFilterDisable : 0')
    config+='IntraBlockCopyEnabled : 0\nPaletteMode : 0\nColourTransform : 1\n'
    results=[]
    for depth in [8,10]:
        for name,control,hash_me,width,height,fractional in [('intra',0,0,64,64,False),('inter',0,0,64,64,False),('parallel',0,0,128,96,False),('offsets',0,0,64,64,False),('bypass',0,0,64,64,False)]:
            with tempfile.TemporaryDirectory(prefix='fvid-scc-act-') as d:
                tmp=Path(d);cfg=tmp/'owned.cfg';src=tmp/'owned.yuv';stream=tmp/'owned.hevc';recon=tmp/'recon.yuv';oracle=tmp/'oracle.yuv'
                cfg.write_text((config.replace('IntraPeriod : -1','IntraPeriod : 1').replace('DecodingRefreshType : 2','DecodingRefreshType : 0').replace('Frame1 : B','Frame1 : I') if name=='intra' else config)+('ActQpYOffset : -2\nActQpCbOffset : -7\nActQpCrOffset : 1\n' if name=='offsets' else '')+('TransquantBypassEnable : 1\nCUTransquantBypassFlagForce : 1\n' if name=='bypass' else '')+f'MotionVectorResolutionControlIdc : {control}\nHashBasedME : {hash_me}\n'+('WaveFrontSynchro : 1\n' if 'parallel' in name else ''))
                def sample(x,y,c):
                    x%=width
                    return ((x*3+y*5+((x//8+y//8)%2)*31)&127)+32+c*4
                raw=bytearray()
                for f in range(3):
                    offset=f if fractional else f*32
                    for c in range(3):
                        for y in range(height):
                            for x in range(width):
                                q=x*4-offset;xx,phase=divmod(q,4)
                                raw.append(min(255,((4-phase)*sample(xx,y,c)+phase*sample(xx+1,y,c)+2)//4+f*7*((x//16+y//16)%3)))
                src.write_bytes(raw)
                subprocess.run([str(a.scm_encoder.resolve()),'-c',str(cfg),'-i',str(src),'-b',str(stream),'-o',str(recon),'-wdt',str(width),'-hgt',str(height),'-fr','25','-f','3','--InputBitDepth=8',f'--InternalBitDepth={depth}','--InputChromaFormat=444'],check=True)
                subprocess.run([str(a.scm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True)
                assert recon.read_bytes()==oracle.read_bytes()
                assert len(oracle.read_bytes())==3*width*height*3*(1 if depth==8 else 2)
                stem=f'hevc-scc-act-{name}-rext{depth}'
                mp4=mux(stream.read_bytes(),depth,width,height,chroma_format=3)
                (fixtures/f'{stem}.mp4').write_bytes(mp4);(fixtures/f'{stem}.yuv').write_bytes(oracle.read_bytes())
                results.append(dict(stem=stem,mp4_sha256=hashlib.sha256(mp4).hexdigest(),oracle_sha256=hashlib.sha256(oracle.read_bytes()).hexdigest()))
    report=dict(reference='HM-16.20+SCM-8.8',revision='cae35dba3dcf8e1b6121a70b397000af36301045',encoder_sha256=hashlib.sha256(a.scm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(a.scm_decoder.read_bytes()).hexdigest(),fixtures=results)
    (fixtures/'hevc-scc-act-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()

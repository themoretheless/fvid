#!/usr/bin/env python3
"""Owned SCC current-picture prediction fixtures and official SCM reference pixels.
Explicit generation only. SCM-8.8 tag cae35dba (full revision in provenance).
Ordinary tests consume saved bytes without invoking reference tools.
Apply scm_ibc_probe.patch only to the probe decoder: it logs actual IBC
predictions without modifying samples. Compare with the unmodified decoder.
"""
import argparse,ast,hashlib,json,subprocess,tempfile
from pathlib import Path
from hevc_fixture_mp4 import mux

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--scm-encoder',type=Path,required=True);p.add_argument('--scm-decoder',type=Path,required=True);p.add_argument('--scm-probe-decoder',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1];fixtures=root/'tests/fixtures/playback-errors'
    tree=ast.parse((root/'scripts/generate_hevc_tiles_sample.py').read_text())
    config=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id=='config_text' for t in n.targets))
    config=config.replace('Profile : main-RExt','Profile : main-SCC').replace('MaxCUChromaQpAdjustmentDepth : 0','MaxCUChromaQpAdjustmentDepth : -1').replace('SAO : 0','SAO : 1').replace('LoopFilterDisable : 1','LoopFilterDisable : 0')
    config=config.replace('IntraPeriod : -1','IntraPeriod : 1').replace('DecodingRefreshType : 2','DecodingRefreshType : 0').replace('Frame1 : B','Frame1 : I')
    config+='IntraBlockCopyEnabled : 1\nHashBasedIntraBlockCopySearchEnabled : 1\nPaletteMode : 0\nColourTransform : 0\n'
    results=[]
    for depth in [8,10]:
        for name,control,hash_me,width,height,fractional in [('intra',0,0,64,64,False),('parallel',0,0,128,96,False),('weighted',0,0,64,64,False),('weighted-parallel',0,0,128,96,False),('420-odd',0,0,64,64,False),('420-odd-parallel',0,0,128,96,False),('inter',0,0,64,64,False),('inter-parallel',0,0,128,96,False),('bidir',0,0,64,64,False),('bidir-parallel',0,0,128,96,False),('tiles',0,0,128,96,False),('dependent',0,0,128,96,False)]:
            with tempfile.TemporaryDirectory(prefix='fvid-scc-ibc-') as d:
                tmp=Path(d);cfg=tmp/'owned.cfg';src=tmp/'owned.yuv';stream=tmp/'owned.hevc';recon=tmp/'recon.yuv';oracle=tmp/'oracle.yuv'
                chroma_format=1 if name.startswith('420') else 3
                stream_config=config
                if name.startswith(('inter','bidir')):
                    stream_config=stream_config.replace('IntraPeriod : 1','IntraPeriod : -1').replace('Frame1 : I','Frame1 : B' if name.startswith('bidir') else 'Frame1 : P')
                if name in ('tiles','dependent'):
                    stream_config+='NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\nLFCrossTileBoundaryFlag : 0\n'
                    if name=='dependent': stream_config+='SliceSegmentMode : 1\nSliceSegmentArgument : 1\n'
                cfg.write_text(stream_config+('WeightedPredP : 1\n' if name.startswith('weighted') else '')+f'MotionVectorResolutionControlIdc : {control}\nHashBasedME : {hash_me}\n'+('WaveFrontSynchro : 1\n' if 'parallel' in name else ''))
                def sample(x,y,c):
                    x%=17 if name.startswith("420") and c==0 else (8 if chroma_format==1 and c!=0 else 16); y%=16
                    return (x*37+y*23+((x//8+y//8)%2)*83+c*17)&255
                raw=bytearray()
                for f in range(3):
                    offset=f if fractional else f*32
                    for c in range(3):
                        for y in range(height//2 if chroma_format==1 and c!=0 else height):
                            for x in range(width//2 if chroma_format==1 and c!=0 else width):
                                q=x*4-(offset//2 if chroma_format==1 and c!=0 else offset);xx,phase=divmod(q,4)
                                raw.append(((4-phase)*sample(xx,y,c)+phase*sample(xx+1,y,c)+2)//4)
                src.write_bytes(raw)
                subprocess.run([str(a.scm_encoder.resolve()),'-c',str(cfg),'-i',str(src),'-b',str(stream),'-o',str(recon),'-wdt',str(width),'-hgt',str(height),'-fr','25','-f','3','--InputBitDepth=8',f'--InternalBitDepth={depth}',f'--InputChromaFormat={420 if chroma_format==1 else 444}'],check=True)
                subprocess.run([str(a.scm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True)
                assert recon.read_bytes()==oracle.read_bytes()
                probe=tmp/'probe.yuv'
                checked=subprocess.run([str(a.scm_probe_decoder.resolve()),'-b',str(stream),'-o',str(probe),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True,capture_output=True)
                active=checked.stderr.count(b'FVID_IBC_BLOCK')
                assert active>0, 'fixture never used current-picture prediction'
                assert probe.read_bytes()==oracle.read_bytes()
                assert len(oracle.read_bytes())==3*width*height*(3 if chroma_format==3 else 3/2)*(1 if depth==8 else 2)
                stem=f'hevc-scc-ibc-{name}-rext{depth}'
                mp4=mux(stream.read_bytes(),depth,width,height,chroma_format=chroma_format)
                (fixtures/f'{stem}.mp4').write_bytes(mp4);(fixtures/f'{stem}.yuv').write_bytes(oracle.read_bytes())
                results.append(dict(stem=stem,active_current_picture_predictions=active,mp4_sha256=hashlib.sha256(mp4).hexdigest(),oracle_sha256=hashlib.sha256(oracle.read_bytes()).hexdigest()))
    report=dict(reference='HM-16.20+SCM-8.8',revision='cae35dba3dcf8e1b6121a70b397000af36301045',encoder_sha256=hashlib.sha256(a.scm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(a.scm_decoder.read_bytes()).hexdigest(),probe_decoder_sha256=hashlib.sha256(a.scm_probe_decoder.read_bytes()).hexdigest(),probe_patch_sha256=hashlib.sha256((root/'scripts/scm_ibc_probe.patch').read_bytes()).hexdigest(),fixtures=results)
    (fixtures/'hevc-scc-ibc-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()

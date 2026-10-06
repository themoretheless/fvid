#!/usr/bin/env python3
"""Owned SCC palette fixtures and official SCM reference pixels.
Explicit generation only. SCM-8.8 tag cae35dba (full revision in provenance).
Apply scm_scc14_config.patch to encoder configuration validation for deep cases.
Ordinary tests consume saved bytes without invoking reference tools.
"""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from hevc_fixture_mp4 import mux

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--scm-encoder',type=Path,required=True);p.add_argument('--scm-decoder',type=Path,required=True);p.add_argument('--scm-probe-decoder',type=Path,required=True)
    a=p.parse_args();root=Path(__file__).resolve().parents[1];fixtures=root/'tests/fixtures/playback-errors'
    reference_config=a.scm_encoder.resolve().parent.parent/'cfg/encoder_intra_main_scc.cfg'
    config=reference_config.read_text()
    config+='ColourTransform : 0\nIntraBlockCopyEnabled : 0\nPaletteMode : 1\n'
    results=[]
    for depth in [8,10,12,14]:
        for name,control,hash_me,width,height,fractional in [('intra',0,0,64,64,False),('parallel',0,0,128,96,False),('initializers',0,0,64,64,False),('initializers-parallel',0,0,128,96,False),('pps-initializers',0,0,64,64,False),('pps-initializers-parallel',0,0,128,96,False),('tiles',0,0,128,128,False),('dependent',0,0,128,128,False),('420',0,0,64,64,False),('420-parallel',0,0,128,96,False),('422',0,0,64,64,False),('422-parallel',0,0,128,96,False),('mono',0,0,64,64,False),('mono-parallel',0,0,128,96,False),('escape',0,0,64,64,False),('escape-parallel',0,0,128,96,False),('escape-bypass',0,0,64,64,False),('escape-bypass-parallel',0,0,128,96,False),('420-escape', 0, 0, 64, 64, False),('420-escape-parallel', 0, 0, 128, 96, False),('420-escape-bypass', 0, 0, 64, 64, False),('420-escape-bypass-parallel', 0, 0, 128, 96, False),('422-escape', 0, 0, 64, 64, False),('422-escape-parallel', 0, 0, 128, 96, False),('422-escape-bypass', 0, 0, 64, 64, False),('422-escape-bypass-parallel', 0, 0, 128, 96, False),('mono-escape', 0, 0, 64, 64, False),('mono-escape-parallel', 0, 0, 128, 96, False),('mono-escape-bypass', 0, 0, 64, 64, False),('mono-escape-bypass-parallel', 0, 0, 128, 96, False),('precision-escape', 0, 0, 64, 64, False),('precision-escape-bypass', 0, 0, 64, 64, False),('mono-precision-escape', 0, 0, 64, 64, False),('mono-precision-escape-bypass', 0, 0, 64, 64, False),('420-precision-escape', 0, 0, 64, 64, False),('420-precision-escape-bypass', 0, 0, 64, 64, False),('422-precision-escape', 0, 0, 64, 64, False),('422-precision-escape-bypass', 0, 0, 64, 64, False)]:
            # SCM SCC high-throughput requires WPP; tiles+WPP is forbidden here.
            if 'precision' in name and depth<12:
                continue
            if depth>=12 and name in ('tiles','dependent'):
                continue
            with tempfile.TemporaryDirectory(prefix='fvid-scc-palette-') as d:
                tmp=Path(d);cfg=tmp/'owned.cfg';src=tmp/'owned.yuv';stream=tmp/'owned.hevc';recon=tmp/'recon.yuv';oracle=tmp/'oracle.yuv'
                chroma=0 if name.startswith('mono') else 1 if name.startswith('420') else 2 if name.startswith('422') else 3
                extra=''
                if depth>=12:
                    extra+='Profile : high-throughput-SCC\nMaxBitDepthConstraint : 14\nMaxChromaFormatConstraint : 444\nSCCHighThroughputFlag : 1\nWaveFrontSynchro : 1\n'
                if 'escape' in name:
                    extra+='PaletteMaxSize : 4\n'
                    if 'bypass' in name:
                        extra+='TransquantBypassEnable : 1\nCUTransquantBypassFlagForce : 1\n'
                if name in ('tiles','dependent'):
                    extra+='NumTileColumnsMinus1 : 1\nNumTileRowsMinus1 : 0\nTileUniformSpacing : 1\nLFCrossTileBoundaryFlag : 0\n'
                    if name=='dependent': extra+='SliceSegmentMode : 1\nSliceSegmentArgument : 1\n'
                cfg.write_text((config.replace('IntraPeriod : -1','IntraPeriod : 1').replace('DecodingRefreshType : 2','DecodingRefreshType : 0').replace('Frame1 : B','Frame1 : I') if name=='intra' else config)+('PalettePredInSPSEnabled : 1\n' if name.startswith('initializers') else '')+('PalettePredInPPSEnabled : 1\n' if name.startswith('pps-initializers') else '')+extra+f'MotionVectorResolutionControlIdc : {control}\nHashBasedME : {hash_me}\n'+('WaveFrontSynchro : 1\n' if 'parallel' in name else ''))
                def sample(x,y,c):
                    x%=width
                    if 'escape' in name and (x+3*y)%29==0:
                        return (16+(x*13+y*19+c*23)%224)*(1<<(depth-8))+1+(x+y+c)%((1<<(depth-8))-1) if 'precision' in name else 16+(x*13+y*19+c*23)%224
                    value=[32,80,144,208][(x//8+y//8)%4]+c*7
                    return value*(1<<(depth-8))+3+c if 'precision' in name else value
                raw=bytearray()
                for f in range(3):
                    offset=f if fractional else f*32
                    for c in range(1 if chroma==0 else 3):
                        for y in range(height//2 if c!=0 and chroma==1 else height):
                            for x in range(width//2 if c!=0 and chroma!=3 else width):
                                q=x*4-offset;xx,phase=divmod(q,4)
                                value=((4-phase)*sample(xx,y,c)+phase*sample(xx+1,y,c)+2)//4
                                raw.extend(value.to_bytes(2,'little') if 'precision' in name else bytes([min(255,value)]))
                src.write_bytes(raw)
                subprocess.run([str(a.scm_encoder.resolve()),'-c',str(cfg),'-i',str(src),'-b',str(stream),'-o',str(recon),'-wdt',str(width),'-hgt',str(height),'-fr','25','-f','3',f'--InputBitDepth={depth if "precision" in name else 8}',f'--InternalBitDepth={depth}',f'--InputChromaFormat={400 if chroma==0 else 420 if chroma==1 else 422 if chroma==2 else 444}'],check=True)
                subprocess.run([str(a.scm_decoder.resolve()),'-b',str(stream),'-o',str(oracle),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True)
                assert recon.read_bytes()==oracle.read_bytes()
                if 'precision' in name:
                    pixels=oracle.read_bytes()
                    samples=[int.from_bytes(pixels[i:i+2],'little') for i in range(0,len(pixels),2)]
                    assert any(value&15 for value in samples), 'oracle lost all low bits'
                    if 'bypass' in name:
                        assert src.read_bytes()==oracle.read_bytes(), 'lossless precision fixture changed input samples'
                probe=tmp/'probe.yuv'
                checked=subprocess.run([str(a.scm_probe_decoder.resolve()),'-b',str(stream),'-o',str(probe),f'--OutputBitDepth={depth}',f'--OutputBitDepthC={depth}','--SEIDecodedPictureHash=0'],check=True,capture_output=True)
                active=checked.stderr.count(b'FVID_PALETTE_BLOCK')
                assert active>0, 'fixture never used palette mode'
                escape_lossy=checked.stderr.count(b'FVID_PALETTE_ESCAPE_LOSSY')
                escape_bypass=checked.stderr.count(b'FVID_PALETTE_ESCAPE_BYPASS')
                transpose_escapes=checked.stderr.count(b'FVID_PALETTE_TRANSPOSE_ESCAPE')
                if 'escape' in name:
                    assert (escape_bypass if 'bypass' in name else escape_lossy)>0, 'fixture never decoded intended escape syntax'
                    assert transpose_escapes>0, 'fixture never decoded transposed escapes'
                assert probe.read_bytes()==oracle.read_bytes()
                assert len(oracle.read_bytes())==3*width*height*({0:1,1:1.5,2:2,3:3}[chroma])*(1 if depth==8 else 2)
                stem=f'hevc-scc-palette-{name}-rext{depth}'
                mp4=mux(stream.read_bytes(),depth,width,height,chroma_format=chroma)
                (fixtures/f'{stem}.mp4').write_bytes(mp4);(fixtures/f'{stem}.yuv').write_bytes(oracle.read_bytes())
                results.append(dict(stem=stem,active_palette_blocks=active,escape_lossy_samples=escape_lossy,escape_bypass_samples=escape_bypass,transpose_escape_samples=transpose_escapes,mp4_sha256=hashlib.sha256(mp4).hexdigest(),oracle_sha256=hashlib.sha256(oracle.read_bytes()).hexdigest()))
    report=dict(encoder_configuration_patch_sha256=hashlib.sha256((root/'scripts/scm_scc14_config.patch').read_bytes()).hexdigest(),reference='HM-16.20+SCM-8.8',reference_config_sha256=hashlib.sha256(reference_config.read_bytes()).hexdigest(),revision='cae35dba3dcf8e1b6121a70b397000af36301045',encoder_sha256=hashlib.sha256(a.scm_encoder.read_bytes()).hexdigest(),decoder_sha256=hashlib.sha256(a.scm_decoder.read_bytes()).hexdigest(),probe_decoder_sha256=hashlib.sha256(a.scm_probe_decoder.read_bytes()).hexdigest(),probe_patch_sha256=hashlib.sha256((root/'scripts/scm_palette_probe.patch').read_bytes()).hexdigest(),fixtures=results)
    (fixtures/'hevc-scc-palette-oracle.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()

#!/usr/bin/env python3
"""Own multichannel/LFE spectra, scalar PCM, and authored implicit-SBR multiplexing.
Offline own region helper only; no foreign codec, private source or network.
"""
import json,struct,subprocess
from pathlib import Path
from generate_aac_main_tools_fixtures import channel,ics,Filterbank,SEQUENCES
from generate_aac_ssr_fixtures import DEST,field,frequency,packed
from generate_he_aac_packet_fixtures import video_fixture
from generate_adts_multiblock_fixtures import transport

def pce(prefix,channels):
    # Front FC+CPE0, back CPE1 for 5.1; front FC+CPE0, side CPE1,
    # back CPE2 for 7.1. All include LFE0, no comment or mix metadata.
    bits=prefix+'0000'+'01'+frequency(24000)+field(2,4)+field(channels==8,4)+field(1,4)+'01'+'000'+'0000'+'000'
    bits+='0'+'0000'+'1'+'0000'
    if channels==8:bits+='1'+'0001'
    bits+='1'+field(2 if channels==8 else 1,4)+'0000'
    return bits+'0'*(-len(bits)%8)+'00000000'
def fixed(asc,payload,configuration):
    size=len(payload)+9
    return packed(field(0xfff,12)+'0'+'00'+'0'+'01'+frequency(24000)+'0'+field(configuration,3)+'0000'+field(size,13)+field(0x7ff,11)+'00').hex()
def main():
    blob=bytearray();sources=[]
    for layout,channels,explicit in [('51',6,False),('71-wide',8,False),('51-pce',6,True),('71-side-pce',8,True)]:
        cfg=0 if explicit else (6 if channels==6 else 7)
        prefix=field(2,5)+frequency(24000)+field(cfg,4)+'000'
        asc=packed(pce(prefix,channels) if explicit else prefix)
        mapping=[2,0,1,4,5,3] if channels==6 else ([2,0,1,6,7,4,5,3] if explicit else [2,6,7,0,1,4,5,3])
        elements=[(0,0),(1,0),(1,1)]+([(1,2)] if channels==8 else [])+[(3,0)]
        banks=[Filterbank() for _ in range(channels)];pcm=bytearray();rows=[]
        for frame,seq in enumerate(SEQUENCES):
            wire=pce('101',channels) if explicit and frame in (0,11) else '';coding=0;signals=[]
            for kind,tag in elements:
                wire+=field(kind,3)+field(tag,4)+('0' if kind==1 else '')
                for _ in range(2 if kind==1 else 1):
                    shape=(frame+coding)%2;values=[[(-1 if (frame+coding+w)%2 else 1)*v for v in [1,-1,0,1]] for w in range(8 if seq==2 else 1)]
                    raw=channel(seq,[1],values,info=ics(seq,1,False,shape=shape));raw=field(140+4*coding,8)+raw[8:];wire+=raw
                    signals.append(banks[coding].run(seq,[[float(v*1024*2**coding) for v in row] for row in values],shape));coding+=1
            assert coding==channels
            payload=packed(wire+'111');rows.append(dict(offset=len(blob),bytes=len(payload),header=fixed(asc,payload,cfg)));blob.extend(payload)
            for sample in zip(*signals):
                for index in range(channels):pcm+=struct.pack('<f',sample[mapping.index(index)])
        name='lc-'+layout;file='adts-layout-sbr-'+name+'-pcm.bin';(DEST/file).write_bytes(pcm)
        sources.append(dict(name=name,asc=asc.hex(),channels=channels,core_channels=channels,frames=rows,sample_rate=24000,samples=12288,pcm_file=file,slots=16,bands=32,container_rate=24000,container_frame_samples=1024,pcm_offset=0))
    implicit=json.loads((DEST/'he-aac-implicit-sbr.json').read_text())
    first=next(c for c in implicit['cases'] if c['slots']==16 and c['bands']==64)
    for name,case,packetfile,ch in [('he-mono',first,'he-aac-sbr-packets.bin',1),('he-stereo',implicit['stereo']['case'],'he-aac-sbr-stereo.bin',2),('he-delayed',implicit['delayed']['case'],'he-aac-missing-sbr.bin',1)]:
        src=(DEST/packetfile).read_bytes();rows=[]
        for i in range(12):
            row=case['frames'][i%len(case['frames'])];payload=src[row['offset']:row['offset']+row['bytes']]
            rows.append(dict(offset=len(blob),bytes=len(payload),header=fixed(bytes.fromhex(case['asc']),payload,ch)));blob.extend(payload)
        sources.append(dict(name=name,asc=case['asc'],channels=ch,core_channels=ch,frames=rows,sample_rate=48000,samples=24576,slots=16,bands=64,container_rate=48000,container_frame_samples=2048,pcm_offset=0))
    subprocess.run(['cargo','build','--offline','--quiet','--no-default-features','--example','adts_crc_regions'],capture_output=True,check=True,cwd=DEST.parents[2])
    meta=json.loads(subprocess.run(['cargo','metadata','--offline','--no-deps','--format-version','1'],capture_output=True,text=True,check=True,cwd=DEST.parents[2]).stdout)
    inputs=[dict(asc=c['asc'],payload=blob[r['offset']:r['offset']+r['bytes']].hex()) for c in sources for r in c['frames']]
    spans=iter(json.loads(subprocess.run([str(Path(meta['target_directory'])/'debug/examples/adts_crc_regions')],input=json.dumps(inputs),capture_output=True,text=True,check=True).stdout))
    cases=[]
    for source in sources:
        for row in source['frames']:row['regions']=next(spans)
        for counts in ([2]*6,[3]*4,[4]*3,[1,2,1,4,4]):
            name=source['name']+'-'+('mixed' if len(set(counts))>1 else str(counts[0]));case=dict(source,name=name,counts=counts)
            case['video']=video_fixture([case],blob,channels=case['channels'],filename='adts-layout-sbr-'+name+'-synthetic.mp4');case['adts']=[]
            for crc in (False,True):
                data=bytearray();at=0
                for count in counts:data+=transport(case['frames'][at:at+count],blob,crc);at+=count
                file='adts-layout-sbr-'+name+('-crc' if crc else '')+'-synthetic.aac';(DEST/file).write_bytes(data);case['adts'].append(file)
            cases.append(case)
    (DEST/'adts-layout-sbr-packets.bin').write_bytes(blob)
    (DEST/'adts-layout-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='authored spectra and PCE, direct filterbank PCM; existing authored SBR syntax; independent GF(2) CRC'),indent=2)+'\n')
    print(len(cases),'own layout/SBR videos')
if __name__=='__main__':main()

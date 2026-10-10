#!/usr/bin/env python3
"""Own ER multi-element SBR region larger than an ordinary FIL; offline only."""
import json
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_sbr_data_fixtures import word
from generate_aac_sbr_dsp_fixtures import header
def sbr(gains,frame,nhigh):
    width=len(gains)
    envelope=field(60,7)+''.join(word(1,15 if k%2 else -15) for k in range(nhigh-1))
    data='0'+('0' if width==2 else '')+'00101'*width+'0'*(6*width)+'00'*width+envelope*(4*width)+field(7,5)*(2*width)+'0'*(width+1)
    return packed(field(13,4)+field(frame==0,1)+(header(0,True) if frame==0 else '')+data)
from generate_aac_sbr_frequency_oracles import tables

def main():
    blob=bytearray();cases=[]
    def store(data):
        row=dict(offset=len(blob),bytes=len(data));blob.extend(data);return row
    _,high,_,_=tables(10,27,0,False,0,0)
    channel=field(100,8)+'0000'+'000000'+'0'+'000'
    for aot in (17,19):
      for slots in (15,16):
        rows=[];controls=[];bad=[]
        for frame in range(3):
            cores=[];extensions=[];ordinary=''
            for tag,gains in [(1,[0]),(2,[1,2])]:
                kind=int(len(gains)==2)
                core=field(tag,4)+('0'+channel*2 if kind else channel)
                raw=sbr(gains,frame,len(high)-1)
                assert len(raw)<269
                ext=''.join(field(v,8) for v in raw)
                cores.append(core);extensions.append(ext)
                ordinary+=field(kind,3)+core+'110'+'1111'+field(len(raw)-14,8)+ext
            assert sum(len(e)//8 for e in extensions)>269
            wire=''.join(cores)+''.join(extensions)
            rows.append(store(packed(wire)));controls.append(store(packed(ordinary+'111')))
            # Truncate inside the second element's envelope Huffman data.
            bad.append(store(packed(wire[:-800])))
        name=f'{aot}-{slots}'
        asc=lambda core:packed(field(5,5)+frequency(24000)+field(3,4)+frequency(48000)+field(core,5)+field(slots==15,1)+'00'+('00' if core in (17,19) else '')).hex()
        c=dict(name=name,aot=aot,asc=asc(aot),control_asc=asc(2),frames=rows,control_frames=controls,bad_frames=bad,channels=3,bands=64,slots=slots,pcm_offset=0,samples=3*slots*128,container_rate=48000,container_frame_samples=slots*128)
        for key,frames,conf,suffix in [('video',rows,c['asc'],''),('control_video',controls,c['control_asc'],'-control'),('bad_video',bad,c['asc'],'-truncated')]:
            c[key]=video_fixture([dict(c,frames=frames,asc=conf)],blob,channels=3,filename=f'aac-er-large-sbr-{name}{suffix}-synthetic.mp4')
        cases.append(c)
    (DEST/'aac-er-large-sbr-packets.bin').write_bytes(blob)
    (DEST/'aac-er-large-sbr.json').write_text(json.dumps(dict(cases=cases,provenance='Own ER LC/LTP 960/1024 three-channel core and two SBR elements with four envelopes and alternating signed Huffman frequency deltas. Each element fits ordinary FIL; combined ER region exceeds 269 bytes. Matched ordinary HE-AAC controls and truncations. No private media, FFmpeg, foreign encoder or network.'),indent=2)+'\n')
if __name__=='__main__':main()

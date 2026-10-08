#!/usr/bin/env python3
"""Original HE-AAC missing-FIL packets and direct noise/QMF PCM oracle."""
import json,struct,hashlib,re
from decimal import Decimal as D,localcontext
from generate_he_aac_packet_fixtures import DEST,asc,packet,packed,field,video_fixture
from generate_aac_sbr_dsp_fixtures import payload,synthesis
from generate_aac_sbr_frequency_oracles import tables

def main():
    window=[float(v) for v in re.findall(r'-?\d+\.\d+', (DEST.parents[2]/'crates/fvid-media/src/owned_aac/aac_sbr_qmf_window.rs').read_text().split('= [',1)[1])]
    noise_raw=(DEST/'aac-sbr-noise-protocol.f64le').read_bytes()
    noise=[complex(*struct.unpack_from('<dd',noise_raw,16*i)) for i in range(512)]
    _,high,_,_=tables(10,27,0,False,0,0);nhigh=len(high)-1
    silent=packed('000'+'0000'+field(100,8)+'0'+'00'+'0'+'000000'+'0'+'000'+'111')
    blob=bytearray();pcm=bytearray();cases=[]
    with localcontext() as context:
        context.prec=80
        gain=float((D(128)*D('.5')/D('1.5')).sqrt()*D('1.584893192'))
        for slots in [15,16]:
            for bands in [32,64]:
                for pattern in [[True,False,True],[False,True,False],[False,False,False]]:
                    frames=[];hf=[];noise_column=0;active=0
                    for enabled in pattern:
                        raw=packet(payload(nhigh,3,True,0 if active==0 else 2)) if enabled else silent
                        frames.append(dict(offset=len(blob),bytes=len(raw),sbr=enabled));blob.extend(raw)
                        for _ in range(2*slots):
                            hf.append([gain*noise[(noise_column*17+b+1)%512] for b in range(17)] if enabled else [0j]*17)
                            if enabled:noise_column+=1
                        active+=enabled
                    result=synthesis(hf,bands,window)
                    cases.append(dict(slots=slots,bands=bands,signalling='explicit',asc=asc(24000,48000 if bands==64 else 24000,slots,'explicit').hex(),frames=frames,pcm_offset=len(pcm),samples=len(result),pattern=pattern))
                    pcm.extend(struct.pack('<'+str(len(result))+'d',*result))
    (DEST/'he-aac-missing-sbr.bin').write_bytes(blob);(DEST/'he-aac-missing-sbr.f64le').write_bytes(pcm)
    video=video_fixture(cases,blob,1,'he-aac-missing-sbr.mp4')
    (DEST/'he-aac-missing-sbr.json').write_text(json.dumps(dict(cases=cases,video=video,packet_sha256=hashlib.sha256(blob).hexdigest(),pcm_sha256=hashlib.sha256(pcm).hexdigest()),separators=(',',':'))+'\n')
    print(len(cases),'missing-FIL sequences;',len(blob),'packet bytes')
if __name__=='__main__':main()

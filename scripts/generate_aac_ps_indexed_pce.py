"""Own tagged PS packets with indexed mono ASC and confirming PCE."""
import json
from generate_he_aac_packet_fixtures import DEST,field,frequency,packed,video_fixture
from generate_aac_ps_pce_fixtures import packet

def generate():
    source=json.loads((DEST/'aac-ps-absence-oracles.json').read_text())
    reference=json.loads((DEST/'aac-ps-pce-oracles.json').read_text())
    raw=(DEST/'he-aac-ps-absence-packets.bin').read_bytes();blob=bytearray();rows={}
    for c in source['cases']:
        if c['name']!='late-ps':continue
        frames=[]
        for row in c['frames']:
            wire=packet(raw[row['offset']:row['offset']+row['bytes']])
            frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire)
        rows[c['slots']]=frames
    cases=[]
    for original in reference['cases']:
        kind=original['kind'];slots=original['slots']
        for rate,mode in [(24000,'Core'),(48000,'Double')]:
            ga=field(slots==15,1)+'00'
            bits=field(2,5)+frequency(24000)+'0001'+ga if kind=='LC' else field(29 if kind=='PS'else 5,5)+frequency(24000)+'0001'+frequency(rate)+field(2,5)+ga
            name=f'{kind.lower()}-{slots*64}-{rate}'
            c=dict(pcm_offset=0,name=name,asc=packed(bits).hex(),kind=kind,slots=slots,bands=32 if mode=='Core'else 64,mode=mode,frames=rows[slots],pcm=original['pcm'][mode],container_rate=rate,container_frame_samples=slots*(64 if mode=='Core'else 128),samples=original['pcm'][mode][0][1])
            c['video']=video_fixture([c],blob,channels=2,filename='aac-ps-indexed-pce-'+name+'-synthetic.mp4')
            cases.append(c)
    invalid=[]
    for c in cases:
        for label,start,end,replacement in [('profile',7,9,field(0,2)),('rate',9,13,frequency(48000))]:
            first=c['frames'][0];wire=bytes(blob[first['offset']:first['offset']+first['bytes']])
            bits=''.join(field(b,8)for b in wire);assert bits[:3]=='101'
            wire=packed(bits[:start]+replacement+bits[end:])
            row=dict(first,offset=len(blob),bytes=len(wire));blob.extend(wire)
            bad=dict(c,name=c['name']+'-'+label,frames=[row]+c['frames'][1:],control=c['name'])
            bad['video']=video_fixture([bad],blob,channels=2,filename='aac-ps-indexed-pce-invalid-'+bad['name']+'-synthetic.mp4')
            invalid.append(bad)
    (DEST/'aac-ps-indexed-pce-packets.bin').write_bytes(blob)
    (DEST/'aac-ps-indexed-pce.json').write_text(json.dumps(dict(provenance='Authored tagged mono PCE/PS packets and scalar stereo PCM; indexed ASC only. Own synthetic AVC, offline, no external encoder.',cases=cases,invalid=invalid),indent=2)+'\n')
if __name__=='__main__':generate()

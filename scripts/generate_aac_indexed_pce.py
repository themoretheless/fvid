"""Own indexed AAC config with matching repeated in-band PCE, offline."""
import json
from generate_aac_ssr_fixtures import DEST,field,frequency,packed
from generate_he_aac_packet_fixtures import video_fixture
from generate_aac_pce_profile_fixtures import program

def generate():
    source=json.loads((DEST/'aac-pce-profile.json').read_text())
    blob=(DEST/'aac-pce-profile-packets.bin').read_bytes();cases=[];invalid=[]
    for original in source['cases']:
        c=dict(original);obj=c['object_type'];channels=c['channels']
        c['asc']=packed(field(obj,5)+frequency(24000)+field(channels,4)+'000').hex()
        c['video']=video_fixture([c],blob,channels=channels,filename='aac-indexed-pce-'+c['name']+'-synthetic.mp4')
        cases.append(c)
        for label,outer,rate in [('profile',2 if obj!=2 else 1,24000),('rate',obj,48000)]:
            bad=dict(c,name=c['name']+'-'+label,container_rate=rate)
            bad['asc']=packed(field(outer,5)+frequency(rate)+field(channels,4)+'000').hex()
            bad['video']=video_fixture([bad],blob,channels=channels,filename='aac-indexed-pce-invalid-'+bad['name']+'-synthetic.mp4')
            invalid.append(bad)
    extended=bytearray(blob)
    for original in list(cases):
        if original['name'].startswith('main-cce-'):continue
        obj=original['object_type'];channels=original['channels']
        prefix=program('101',obj,channels);old=packed(prefix)
        assert prefix[38:42]=='0000'
        for tag in [3,15]:
            new=packed(prefix[:38]+field(tag,4)+prefix[42:])
            c=dict(original,name=original['name']+f'-tag{tag}',frames=[],tag=tag,late=False)
            for row in original['frames']:
                packet=bytes(blob[row['offset']:row['offset']+row['bytes']])
                has_pce=packet.startswith(old)
                offset=len(old) if has_pce else 0
                payload=packet[offset:]
                wire=''.join(field(b,8)for b in payload)
                assert wire[3:7]=='0000'
                payload=packed(wire[:3]+field(tag,4)+wire[7:])
                packet=(new if has_pce else b'')+payload
                c['frames'].append(dict(row,offset=len(extended),bytes=len(packet)))
                extended.extend(packet)
            c['video']=video_fixture([c],extended,channels=channels,filename='aac-indexed-pce-'+c['name']+'-synthetic.mp4')
            cases.append(c)
            # Keep the final confirming PCE but omit the initial one: adoption
            # must preserve overlap/predictor state already built from indexed ASC.
            first=c['frames'][0];packet=bytes(extended[first['offset']:first['offset']+first['bytes']])
            assert packet.startswith(new)
            packet=packet[len(new):]
            row=dict(first,offset=len(extended),bytes=len(packet));extended.extend(packet)
            late=dict(c,name=c['name']+'-late',frames=[row]+c['frames'][1:],late=True)
            late['video']=video_fixture([late],extended,channels=channels,filename='aac-indexed-pce-'+late['name']+'-synthetic.mp4')
            cases.append(late)
    (DEST/'aac-indexed-pce-packets.bin').write_bytes(extended)
    (DEST/'aac-indexed-pce.json').write_text(json.dumps(dict(provenance='Authored packets/PCM with matching PCE, indexed ASC, tags0/3/15 and delayed adoption. No private media/network/external encoder.',cases=cases,invalid=invalid),indent=2)+'\n')
if __name__=='__main__':generate()

"""Own exact-bit EP class recovery vectors; generation is explicit and offline."""
import json
from pathlib import Path
from generate_he_aac_packet_fixtures import field,packed
from generate_aac_ep_playback_fixtures import protected_header,crc
from generate_aac_ep_class_codes_fixtures import chain
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ep-class-boundaries.json'
def generate():
    vectors=[]
    for total in range(65):
        lengths=[total//3,0,total-total//3]
        for absent in [False,True]:
            # The middle class distinguishes present-empty from absent-optional.
            config=field(1,8)+'00'+'001'+'001'+field(3,6)
            rates=[total%25,(total+3)%25,(total+11)%25]
            widths=[4,0,16]
            for index,(n,rate,width)in enumerate(zip(lengths,rates,widths)):
                config+='000'+'00'+field(index==2,1)+field(index==1,1)+field(n,16)+field(rate,5)+field(width,5)
            config+='0'+'0'
            source=[''.join(str((j*j+index*3+j//3+total)%2)for j in range(n))for index,n in enumerate(lengths)]
            coded=chain([bits+crc(bits,width)for bits,width in zip(source,widths)if bits],
                        [rate for bits,rate in zip(source,rates)if bits],
                        [index==2 for index,bits in enumerate(source)if bits])
            selector=field(absent,1)*3
            # Three attribute bits use the independent shortened BCH oracle.
            stuffing=(-len(selector)-len(protected_header('000',False))-sum(map(len,coded)))%8
            wire=selector+protected_header(field(stuffing,3),False)+''.join(coded)+'0'*stuffing
            assert len(wire)%8==0
            vectors.append(dict(total=total,config=packed(config).hex(),packet=packed(wire).hex(),
                classes=[dict(class_index=index,bits=bits)for index,bits in enumerate(source)if index!=1 or not absent]))
    return (json.dumps(dict(vectors=vectors,provenance='Own exact-bit information, independent CRC/SRCPC polynomial arithmetic and BCH headers. Includes zero-bit and absent optional classes; no codec, process, network or private media.'),indent=2)+'\n').encode()
if __name__=='__main__':OUT.write_bytes(generate())

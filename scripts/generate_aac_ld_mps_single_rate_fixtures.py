"""Owned shared LD-SBR/MPS synthesis convention videos; offline generation."""
import cmath,json,math,struct
from pathlib import Path
from generate_aac_eld_fixtures import generate,mkv
from generate_aac_ld_mps_entropy_fixtures import pcm as code_pcm
from generate_aac_eld_spatial_sbr_fixtures import analysis
from generate_aac_eld_sbr_fixtures import reconstruct,sbr_bits
from generate_aac_sbr_dsp_fixtures import header
from generate_aac_sbr_frequency_oracles import tables
from generate_aac_ld_mps_transport_fixtures import envelope,length
from generate_he_aac_packet_fixtures import field,frequency,packed,video_fixture
ROOT=Path(__file__).resolve().parents[1];FIX=ROOT/'tests/fixtures/playback-errors';OUT=FIX/'aac-ld-mps-single-rate'
def synthesis(rows,bands,window):
    # Shared single-rate AAC LD-QMF N uses2n-31; dual-rate uses2n-255.
    # Direct absolute-time superposition,not the production shifting history.
    c=window if bands==64 else[(a+b)/2 for a,b in zip(window[::2],window[1::2])]
    shift=31 if bands==32 else 255
    m=[[cmath.exp(1j*math.pi/(2*bands)*(k+.5)*(2*n-shift))/64 for k in range(bands)]for n in range(2*bands)]
    v=[[sum((a*b).real for a,b in zip(row,mn))for mn in m]for row in rows]
    return[sum((v[t-2*p][k]*c[2*bands*p+k]if t>=2*p else 0)+(v[t-2*p-1][bands+k]*c[2*bands*p+bands+k]if t>=2*p+1 else 0)for p in range(5))for t in range(len(rows))for k in range(bands)]
def main():
    OUT.mkdir(exist_ok=True);core=generate(True,[(n,24000,1,'base',0,0,False)for n in[480,512]],frame_count=8)
    noise=[complex(*v)for v in struct.iter_unpack('<dd',(FIX/'aac-sbr-noise-protocol.f64le').read_bytes())]
    window=json.loads((FIX/'aac-ld-qmf/window.json').read_text());_,high,low,_=tables(10,27,0,False,0,0)
    blob=bytearray();cases=[]
    for base in json.loads(core['manifest.json'])['cases']:
        n=base['n'];slots=n//32;pcm=[v[0]for v in struct.iter_unpack('<f',core[base['name']+'.f32le'])]
        qmf=reconstruct(analysis(pcm,window,10),slots,noise)
        for double in[False,True]:
          for switched in[False,True]:
            bands=64 if double else 32;rate=24000*(1+double);name=f'n{n}-x{1+double}'+('-gain'if switched else'')
            # Own tree212: four bands,explicit CLD0dB,ICC1,one endpoint,
            # no smoothing/residual. In-band clip gain changes reset synthesis.
            ssc_bits=field(3 if double else 6,4)+field(slots-1,5)+field(7,3)+field(7,4)+'00'+'0'+'000'+'0000'
            ssc=packed(ssc_bits)
            asc=packed(field(31,5)+field(7,6)+frequency(24000)+field(1,4)+field(n==480,1)+'0001'+field(double,1)+'1'+header(0,True)+'0010'+length(len(ssc))+''.join(field(b,8)for b in ssc)+'000000')
            # Independent first-frame interpolation of both M1 and M2.
            rows=[[v/32768/math.sqrt(2)*(((t+1)/slots)**2 if t<slots else 1)for v in row[:bands]]for t,row in enumerate(qmf)]
            if switched:
                samples=[]
                for first,last,gain in[(0,3,1.),(3,6,1.414213),(6,8,1.)]:
                    samples+=synthesis([[v*gain for v in row]for row in rows[first*slots:last*slots]],bands,window)
            else:samples=synthesis(rows,bands,window)
            gold=b''.join(struct.pack('<ff',v,v)for v in samples)
            packets=[];frames=[]
            for i,r in enumerate(base['frames']):
                spatial=packed('001'+'11'+'00001'+code_pcm([15]*4,31)+'00'+'00')
                changed=switched and i in[3,6]
                if changed:
                    current=packed(ssc_bits[:19]+field(2 if i==3 else 0,3)+ssc_bits[22:])
                    spatial=bytes([len(current)])+current+spatial
                bits=r['bits']+sbr_bits(1,len(high)-1,len(low)-1,slots,i,True)+envelope(9,1 if changed else 0,3,spatial,True)
                wire=packed(bits);frames.append(dict(offset=len(blob),bytes=len(wire)));blob.extend(wire);packets.append(wire)
            case=dict(name=name,n=n,channels=2,rate=rate,asc=asc.hex(),frames=frames,slots=slots,bands=bands,samples=8*n*(1+double),container_rate=rate,container_frame_samples=n*(1+double),pcm_offset=0,video=name+'.mp4',file=name+'.mkv',golden=name+'.f32le',gain_switch=switched,edges=[n*(1+double),3*n*(1+double),6*n*(1+double),7*n*(1+double)])
            (OUT/case['golden']).write_bytes(gold);(OUT/case['video']).write_bytes(video_fixture([case],blob,2,write=False)['data']);(OUT/case['file']).write_bytes(mkv(asc,packets,rate,n*(1+double),2));cases.append(case)
    (OUT/'packets.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(cases=cases,exports=[dict(c,name=c['name']+'-matroska',video=c['file'])for c in cases]),indent=2)+'\n')
    print('8 authored480/512 single/dual-rate programmes;16 containers;gain-reset controls')
if __name__=='__main__':main()

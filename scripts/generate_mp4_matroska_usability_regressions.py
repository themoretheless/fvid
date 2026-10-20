"""Authored paired AVC tracks for effective usability preservation in re-mux."""
import json,hashlib,struct
from pathlib import Path
from generate_avc_mbaff_inter_component_regressions import generate as avc
from generate_mp4_enabled_track_regressions import boxes,rewrite_track,canonical_movie,sample_track
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-matroska-usability'
def audiovisual(source):
    base=dict(boxes(source['y14-c8-field-cavlc-b-bi.mp4']));movie=dict(boxes(base[b'moov']));ftyp=atom(b'ftyp',base[b'ftyp']);payloads=[base[b'mdat']]
    def movie_duration(data,duration):
        result=b''
        for kind,body in boxes(data):
            if kind==b'tkhd':body=bytearray(body);body[20:24]=word(duration)
            result+=atom(kind,body)
        return result
    tracks=atom(b'trak',movie_duration(rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False),2880))
    bits='000'+'0000'+f'{100:08b}'+'0'+'00'+'0'+'000000'+'0'+'000'+'111'
    bits+='0'*(-len(bits)%8);packet=int(bits,2).to_bytes(len(bits)//8,'big')
    def descriptor(tag,data):return bytes([tag,len(data)])+data
    esds=bytes(4)+descriptor(3,b'\0\2\0'+descriptor(4,b'\x40\x15'+bytes(11)+descriptor(5,b'\x11\x88'))+descriptor(6,b'\2'))
    entry=bytearray(28);entry[6:8]=b'\0\1';entry[16:20]=struct.pack('>HH',1,16);entry[24:28]=word(48000<<16);entry=entry+atom(b'esds',esds)
    for number,flags in [(2,6),(3,3)]:
        track=sample_track(number,flags,b'soun',b'mp4a',entry,[packet]*3,len(ftyp)+8+sum(map(len,payloads)),48000,1024)
        tracks+=atom(b'trak',movie_duration(dict(boxes(track))[b'trak'],3072));payloads.append(packet*3)
    header=canonical_movie(movie[b'mvhd'],4);header[12:20]=word(48000,3072)
    return ftyp+atom(b'mdat',b''.join(payloads))+atom(b'moov',atom(b'mvhd',header)+tracks)
def generate():
    source,cases=avc();names=['y8-c8-frame-cavlc-b-bi.mp4','y14-c8-field-cavlc-b-bi.mp4'];parts=[dict(boxes(source[n])) for n in names];payloads=[p[b'mdat'] for p in parts];ftyp=atom(b'ftyp',parts[0][b'ftyp']);movie=canonical_movie(dict(boxes(parts[0][b'moov']))[b'mvhd'],3);original=[dict(boxes(p[b'moov']))[b'trak'] for p in parts]
    files={};records=[]
    for name,flags in [('disabled-first',[6,3]),('legacy-zero',[0,0]),('legacy-disabled',[6,6]),('both-enabled',[3,3])]:
        tracks=b''.join(atom(b'trak',rewrite_track(t,i+1,flags[i],len(ftyp)+8+sum(map(len,payloads[:i])),False)) for i,t in enumerate(original));data=ftyp+atom(b'mdat',b''.join(payloads))+atom(b'moov',atom(b'mvhd',movie)+tracks);files[name+'.mp4']=data;records.append({'file':name+'.mp4','flags':flags,'effective_enabled':[False,True] if name=='disabled-first' else [True,True]})
    files['audiovisual.mp4']=audiovisual(source)
    files['manifest.json']=(json.dumps({'provenance':'Four authored60ms dual AVC B-picture tracks plus64ms AVC/two mono AAC-LC silence tracks. Component arrays/bitstreams derive from owned synthetic writers; no private data/encoder/network.','cases':records,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)

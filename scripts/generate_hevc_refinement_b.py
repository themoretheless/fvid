"""Owned mixed-role B-picture depth refinement, offline native acceptance author."""
import json
from functools import lru_cache
import generate_hevc_texture_iv_b as b
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_cqt import reference,texture_amp_bi,OUT
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_layered_reconstruction import parameter_ids
from generate_hevc_layered_slices import header_pps
from hevc_fixture_mp4 import mux

@lru_cache(None)
def parameters(enabled,weighted,scale,precision=0):
    movie=control_movie(enabled,camera=(scale,0,precision))
    config=movie[movie.index(b'hvcC')+4:];at=23;units=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big');at+=2
            nal=config[at:at+length];at+=length
            if weighted and ((int.from_bytes(nal[:2],'big')>>9)&63)==34 and ((int.from_bytes(nal[:2],'big')>>3)&63)==5:
                nal=parameter_ids(header_pps(5,0,True),0,[2,2])
            units.append(nal)
    return units

def header(layer,poc,data,enabled,weighted=False,explicit=False,list_entries=None,illumination=None,ic_disabled_merge_zero=False,merge_extra=1):
    kind=0 if layer==5 and poc==1 else 2
    value='1'+('0'if poc==0 else'')+b.ue({3:1,5:2}[layer])+b.ue(kind)+'1'+f'{poc:08b}'
    if poc:
        negative=[2]if poc==2 else[1];positive=[]if poc==2 else[1]
        value+='0'+b.ue(len(negative))+b.ue(len(positive))
        for distances in [negative,positive]:
            previous=0
            for distance in distances:value+=b.ue(distance-previous-1)+'1';previous=distance
        value+=b.ue(0)+b.ue(0)+'0'
    if layer==5:
        value+=str(int(poc==1)) # sole texture inter-layer reference
        if enabled:value+=str(int(poc==1)) # in-component prediction
    value+='0'if layer==3 else'00'
    if kind==0:
        value+='1'+b.ue(2)+b.ue(2)
        for entries in (list_entries if list_entries is not None else [None,None]):
            value+='0' if entries is None else '1'+''.join(f'{entry:02b}'for entry in entries)
        value+='0'+'0'
        if weighted:
            value+=b.ue(1)+b.se(0)
            if explicit:
                from generate_hevc_texture_iv_chroma_b import CHROMA
            for list,entries in enumerate(b.WEIGHTS):
                value+='111'+('111'if explicit else'000')
                for ref,(weight,offset) in enumerate(entries):
                    value+=b.se(weight-2)+b.se(offset)
                    if explicit:
                        for weight,offset in CHROMA[list][ref]:value+=b.se(weight-2)+b.se(offset-128+64*weight)
        else:value+='0' if illumination is None else '1'+str(int(ic_disabled_merge_zero)) # IC enabled, merge-zero presence control
        value+=b.ue(merge_extra) # max five from available Annex I candidates
    value+=b.se(0)+b.ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(layer<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def movie(enabled,weighted,mode,scale,precision=0,colored=False,explicit=False):
    past=b.pcm();future=b.pcm(97);depth,_=reference(32,5,False)
    if colored:
        from generate_hevc_texture_iv_chroma_b import pcm
        past=pcm(0,0);future=pcm(0,97)
    units=[]
    for poc in [0,2,1]:
        source=(past if poc!=2 else future)if poc!=1 or mode=='intra'else texture_amp_bi(32,top=int(mode[3:]),vertical_partition=True)if mode.startswith('amp')else b.source(mode)
        units.append([
            b.header(0,poc,source,0 if poc==1 and mode!='intra'else 2,
                negative=[2]if poc==2 else[1]if poc==1 else[],positive=[1]if poc==1 else[]),
            header(3,poc,depth,enabled),
            header(5,poc,b.target(0 if mode=='intra'or mode.startswith('amp')else 1)if poc==1 else pcm(5,97 if poc==2 else 0)if colored else future if poc==2 else past,enabled,weighted,explicit)])
    stream=b''.join(b'\0\0\0\1'+n for n in parameters(enabled,weighted,scale,precision)+[n for au in units for n in au])
    return mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def generate():
    files={};cases=[]
    for enabled in [False,True]:
        for weighted in [False,True]:
            for mode in ['intra','bi','l0','l1','amp8','amp24']:
                for scale in [1,-1,4,-4,128]:
                    name=f'texture-refinement-b-{int(enabled)}-{int(weighted)}-{mode}-{scale}.mp4'
                    files[name]=movie(enabled,weighted,mode,scale)
                    cases.append(dict(file=name,enabled=enabled,weighted=weighted,mode=mode,scale=scale,
                        disparity=(scale*254+64)>>7 if enabled else 0))
    files['texture-refinement-b.json']=(json.dumps(dict(
        provenance='Owned mixed-role PCM/CABAC B pictures, camera parameters and MP4. No external codec, private media, network or test-time generation.',
        decode_order=[0,2,1],display_order=[0,1,2],cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)

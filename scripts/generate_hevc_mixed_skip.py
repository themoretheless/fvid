"""Own 10-bit PCM gradient followed by skip-intra CTUs, including parsed headers."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import vps,sps,fmt,ue,packed,escape
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT
def generate():
    files={};target=fmt(chroma=0,depth=(10,10),size=(64,64));target['crop']=[0]*4
    flags=[False]*14;flags[-1]=True
    files['mixed-skip.vps']=vps(target,depth_layer=True)[0]
    files['mixed-skip.sps']=sps(multilayer=True,three_d=payload(flags),pcm=True,pcm_blocks=(5,5))
    files['mixed-skip.pps']=header_pps(3,0,False,False)
    for base in [0,300]:
      for final_mode in range(4):
          b=Encoder(0,26)
          for value in [185,137,139]:
              pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
              b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
          b.decision(5,False);b.decision(3,False)
          first=b.finish() # pcm_flag terminal one, followed by zero alignment
          pcm=packed(''.join(f'{base+x+2*y:010b}'for y in range(32)for x in range(32)))
          # PCM restarts arithmetic state but preserves adapted context banks.
          b.low=0;b.range=510;b.width=9
          b.range-=2 # first end_of_slice_segment_flag is zero after PCM restart
          for index,mode in enumerate([2,3,final_mode]):
              b.decision(5,False);b.decision(3,True);b.decision(4,mode!=0)
              if mode:b.bypass(mode>1)
              if mode>1:b.bypass(mode==3)
              if index<2:
                  b.range-=2
                  while b.range<256:b.low*=2;b.range*=2;b.width+=1
          entropy=first+pcm+b.finish()
          bits='10'+ue(0)+ue(2)+'1'+'00000000'+'0'+se(0)+ue(0)+'1'
          bits+='0'*((-len(bits))%8)
          suffix='-high'if base else ''
          files[f'mixed-skip-mode{final_mode}{suffix}.nal']=((19<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(bits)+entropy)
          values=[base+(x+2*y if x<32 and y<32 else 63 if y<32 else 78 if x<32 else (63 if final_mode in [0,3]else 78))for y in range(64)for x in range(64)]
          files[f'mixed-skip-mode{final_mode}{suffix}.y16']=struct.pack('<4096H',*values)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)

"""Own in_comp_pred_flag witnesses with complete skip-intra entropy payloads."""
from generate_hevc_skip_picture import generate as skips
from generate_hevc_layered_sps import sps,ue,packed,escape
from generate_hevc_colour_plane_fixtures import unescape
from generate_hevc_layered_pps import se
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT
def generate():
    files={};original=skips()['skip-64-mode2.nal'];raw=unescape(original[2:])
    flags=[False]*14;flags[9]=flags[13]=True
    files['component-gate.sps']=sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,6))
    for enabled in [False,True]:
        bits='10'+ue(0)+ue(2)+'1'+'00000000'+str(int(enabled))+'0'+se(0)+ue(0)+'1'
        bits+='0'*((-len(bits))%8);assert len(bits)==24
        files[f"component-gate-{'on'if enabled else'off'}.nal"]=original[:2]+escape(packed(bits)+raw[3:])
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)

"""Offline normative I-135..I-139 metadata oracle, not a coded-tool fixture."""
import json
from generate_hevc_wedge_tables import OUT
LAYOUTS=[[["A1",1],["B1",2],["B0",1],["A0",3],["Other",4],["Other",5]],
         [["A1",1],["B1",2],["B0",1],["B2",3],["Other",4],["Other",5]]]
def specials(mask):
    return dict(T=1 if mask&1 else None,IV=(1 if mask&8 else 2)if mask&2 else None,
        IVShift=(1 if mask&8 else 2)if mask&4 and mask&16 else(9 if mask&4 else None),
        DI=(1 if mask&4 else 3)if mask&8 else None,DIShift=9 if mask&16 else None,VSP=bool(mask&32))
def expected(initial,mask,depth,left,maximum):
    s=specials(mask);out=[]
    def item(kind):return next(((i,m)for i,(k,m)in enumerate(initial)if k==kind),None)
    if s['T']is not None:out.append(6)
    if s['IV']is not None and(s['T']is None or s['T']!=s['IV']):out.append(7)
    compared=s['T']if depth else s['IV']
    for kind in ['A1','B1']:
        p=item(kind)
        if p is not None and(compared is None or p[1]!=compared):out.append(p[0])
    if s['VSP']and(item('A1')is None or not left)and len(out)<maximum:out.append(11)
    p=item('B0')
    if p is not None:out.append(p[0])
    if s['DI']is not None and all(item(k)is None or item(k)[1]!=s['DI']for k in ['A1','B1'])and len(out)<maximum:out.append(9)
    for kind in ['A0','B2']:
        p=item(kind)
        if p is not None and len(out)<maximum:out.append(p[0])
    if s['IVShift']is not None and len(out)<maximum and(s['IV']is None or s['IV']!=s['IVShift']):out.append(8)
    if s['DIShift']is not None and s['IVShift']is None and len(out)<maximum:out.append(10)
    j=0
    while len(out)<maximum:
        assert j<len(initial)
        if initial[j][0]=='Other':out.append(j)
        j+=1
    out=out[:maximum]
    def selected(sequence):
        return [11 if i<6 and initial[i][0]=='A1'and s['VSP']and left else i for i in sequence]
    return out,selected(out),selected(list(range(maximum)))
def generate():
    cases=[]
    for layout,entries in enumerate(LAYOUTS):
        for maximum in range(1,7):
            for mask in range(64):
                for depth in [False,True]:
                    for left in [False,True]:
                        cases.append([layout,maximum,mask,depth,left,*expected(entries[:maximum],mask,depth,left,maximum)])
    return {'extended-merge-order-oracles.json':(json.dumps(dict(provenance='Independent full-list pseudocode execution for I-135..I-139 with 3072 metadata cases. Inputs are prederived candidate motions, not coded IV/DI/VSP playback acceptance. No external codec, private media, network or test-time generation.',layouts=LAYOUTS,cases=cases),separators=(',',':'))+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)

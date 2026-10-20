"""Owned AVC weight-table syntax and independent scalar clipping helpers."""
PROFILES={
 'mixed':{'denoms':[1,2],'entries':[[(3,-5),(3,7),(-2,64)]]},
 'extreme':{'denoms':[0,0],'entries':[[(-1,127),(2,-128),(-1,127)]]},
 'default':{'denoms':[7,7],'entries':[[(128,0)]*3]},
}
B_PROFILES={
 'mixed':{'denoms':[1,2],'entries':[[(3,-7),(1,5),(-2,64)],[(-1,9),(3,-4),(4,-32)]]},
 'extreme':{'denoms':[0,0],'entries':[[(127,127),(-128,-128),(0,64)],[(-128,-128),(127,127),(0,-64)]]},
 'default':{'denoms':[7,7],'entries':[[(128,0)]*3,[(128,0)]*3]},
}
def table(b,profile):
    dy,dc=profile['denoms'];b.ue(dy);b.ue(dc)
    for entry in profile['entries']:
        luma=entry[0]!=(1<<dy,0);b.u(int(luma))
        if luma:b.se(entry[0][0]);b.se(entry[0][1])
        chroma=any(v!=(1<<dc,0) for v in entry[1:]);b.u(int(chroma))
        if chroma:
            for weight,offset in entry[1:]:b.se(weight);b.se(offset)
def uni(source,depths,profile,list_index=0):
    out=[];clipped=0;offset=0
    for component,depth in enumerate(depths):
        count=2048 if component==0 else 1024;denom=profile['denoms'][int(component>0)];w,o=profile['entries'][list_index][component]
        for sample in source[offset:offset+count]:
            value=((w*sample+(1<<(denom-1) if denom else 0))>>denom)+o*(1<<(depth-8))
            clipped+=int(value<0 or value>(1<<depth)-1);out.append(max(0,min((1<<depth)-1,value)))
        offset+=count
    return out,clipped

def bi(a,z,depths,profile):
    out=[];clipped=0;offset=0
    for component,depth in enumerate(depths):
        count=2048 if component==0 else 1024;denom=profile['denoms'][int(component>0)]
        wa,oa=profile['entries'][0][component];wz,oz=profile['entries'][1][component]
        # Scale each coded offset to component precision before averaging;
        # an odd sum at >8 bits distinguishes this from early rounding.
        add=((oa+oz)*(1<<(depth-8))+1)//2
        for left,right in zip(a[offset:offset+count],z[offset:offset+count]):
            value=((wa*left+wz*right+(1<<denom))>>(denom+1))+add
            clipped+=int(value<0 or value>(1<<depth)-1);out.append(max(0,min((1<<depth)-1,value)))
        offset+=count
    return out,clipped

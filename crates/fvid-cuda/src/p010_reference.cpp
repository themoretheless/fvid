#include <cassert>
#include <climits>
int main() {
    unsigned short y[96], uv[48];
    for (int i=0; i<96; ++i) y[i] = (((i*17)%1024)<<6)|37;
    for (int i=0; i<48; ++i) uv[i] = (((i*11)%1024)<<6)|19;
    for (unsigned int h=0; h<2; ++h) for (unsigned int v=0; v<2; ++v) {
        unsigned short dy[24]={}, duv[12]={};
        unsigned int p[10]={24,24,12,12,2,2,4,4,h,v};
        for (unsigned int row=0; row<4; ++row) for (unsigned int col=0; col<4; ++col) {
            threadIdx={col,row};
            fvid_nv12_transform(y,uv,dy,duv,p);
        }
        // Materialize transformed codes first, then blur that independent image.
        unsigned int cropped[16];
        for (int row=0; row<4; ++row) for (int col=0; col<4; ++col)
            cropped[row*4+col] = y[((v?3-row:row)+2)*12+(h?3-col:col)+2]>>6;
        for (int row=0; row<4; ++row) for (int col=0; col<4; ++col) {
            unsigned int sum=0;
            for (int j=-1; j<=1; ++j) for (int i=-1; i<=1; ++i) {
                int x=col+i, z=row+j;
                if (x<0) x=0; if (x>3) x=3;
                if (z<0) z=0; if (z>3) z=3;
                sum+=cropped[z*4+x];
            }
            assert(dy[row*6+col]==(sum/9)<<6);
        }
        for (int row=0; row<2; ++row) for (int col=0; col<2; ++col) for (int c=0; c<2; ++c) {
            int sx=(h?1-col:col)+1, sy=(v?1-row:row)+1;
            assert(duv[row*6+col*2+c]==(uv[sy*12+sx*2+c]&~63));
            FvidSampler sampler{y,uv,p,(unsigned int)c+1};
            assert(sample(sampler,col,row)==(unsigned int)(uv[sy*12+sx*2+c]>>6));
        }
        for (int row=0; row<4; ++row) assert(dy[row*6+4]==0 && dy[row*6+5]==0);
        for (int row=0; row<2; ++row) assert(duv[row*6+4]==0 && duv[row*6+5]==0);
        for (unsigned int plane=0; plane<3; ++plane) {
            FvidSampler sampler{y,uv,p,plane};
            unsigned int max=plane==0?3:1;
            assert(sample(sampler,LLONG_MIN,LLONG_MIN)==sample(sampler,0,0));
            assert(sample(sampler,LLONG_MAX,LLONG_MAX)==sample(sampler,max,max));
        }
    }
}

#include "bos32.h"
int main(int argc,char **argv){
    unsigned duration=argc>1?number(argv[1]):1500,start=ticks_ms();
    volatile unsigned work=0;
    while(ticks_ms()-start<duration){for(unsigned i=0;i<65536;i++)work+=i;}
    print("spin finished\n");return 0;
}

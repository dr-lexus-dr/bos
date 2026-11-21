#include "bos32.h"
int main(int argc,char **argv){
    unsigned limit=argc>1?number(argv[1]):5;
    for(unsigned i=1;i<=limit;i++){print("counter ");decimal(i);print("\n");sleep_ms(150);}
    return 0;
}

#include "bos32.h"
int main(int argc,char **argv){
    unsigned limit=argc>1?number(argv[1]):100;
    if(limit>100000){print("Maximum limit is 100000\n");return 1;}
    unsigned char *used=sbrk(limit+1);
    if(used==(void*)-1)return 1;
    for(unsigned n=2;n<=limit;n++)if(!used[n]){
        decimal(n);print(" ");
        if(n<=limit/n)for(unsigned m=n*n;m<=limit;m+=n)used[m]=1;
    }
    print("\n");return 0;
}

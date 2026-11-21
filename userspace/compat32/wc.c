#include "bos32.h"
int main(int argc,char **argv){
    if(argc<2){print("Usage: wc FILE\n");return 2;}
    int fd=open(argv[1]);if(fd<0){print("Cannot open file\n");return 1;}
    unsigned lines=0,words=0,bytes=0,inside=0;char buf[256];int n;
    while((n=read(fd,buf,sizeof(buf)))>0){bytes+=n;for(int i=0;i<n;i++){
        unsigned ch=(unsigned char)buf[i];lines+=ch=='\n';words+=ch>32&&!inside;inside=ch>32;
    }}close(fd);if(n<0)return 1;
    decimal(lines);print(" ");decimal(words);print(" ");decimal(bytes);print(" ");print(argv[1]);print("\n");return 0;
}

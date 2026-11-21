#include "bos32.h"
int main(int argc,char **argv){
    print("Hello from user space! PID "); decimal(getpid()); print("\n");
    for(int i=0;i<argc;i++){print("argv[");decimal(i);print("] = ");print(argv[i]);print("\n");}
    return 0;
}

#include "bos32.h"
int main(void){
    unsigned target=1+ticks_ms()%100;char line[16];
    print("Guess a number from 1 to 100.\n");
    for(;;){print("Guess: ");int n=read(0,line,sizeof(line)-1);if(n<=0)return 1;
        line[n]=0;unsigned guess=number(line);if(guess==target){print("Correct!\n");return 0;}
        print(guess<target?"Higher\n":"Lower\n");
    }
}

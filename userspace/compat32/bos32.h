#ifndef BOS32_H
#define BOS32_H
/* Freestanding i386 syscall ABI. Return values are negative on failure. */
enum bos_call { BOS_EXIT, BOS_WRITE, BOS_READ, BOS_OPEN, BOS_CLOSE, BOS_GETPID,
                BOS_SLEEP, BOS_TICKS, BOS_SBRK, BOS_YIELD, BOS_GETKEY, BOS_STAT };
static inline int bos(int n,int a,int b,int c) {
    int result;
    __asm__ volatile("int $0x80":"=a"(result):"a"(n),"b"(a),"c"(b),"d"(c):"memory","cc");
    return result;
}
static inline int write(int fd,const void *buf,unsigned len){return bos(BOS_WRITE,fd,(int)buf,len);}
static inline int read(int fd,void *buf,unsigned len){return bos(BOS_READ,fd,(int)buf,len);}
static inline int open(const char *path){return bos(BOS_OPEN,(int)path,0,0);}
static inline int close(int fd){return bos(BOS_CLOSE,fd,0,0);}
static inline int getpid(void){return bos(BOS_GETPID,0,0,0);}
static inline void sleep_ms(unsigned ms){bos(BOS_SLEEP,ms,0,0);}
static inline unsigned ticks_ms(void){return bos(BOS_TICKS,0,0,0);}
static inline void *sbrk(int bytes){return (void*)bos(BOS_SBRK,bytes,0,0);}
static inline void yield(void){bos(BOS_YIELD,0,0,0);}
static inline int getkey(void){return bos(BOS_GETKEY,0,0,0);}
static inline int stat_size(const char *path){return bos(BOS_STAT,(int)path,0,0);}
static inline unsigned length(const char *s){unsigned n=0;while(s[n])++n;return n;}
static inline void print(const char *s){write(1,s,length(s));}
static inline unsigned number(const char *s){unsigned n=0;while(*s>='0'&&*s<='9')n=n*10+(*s++-'0');return n;}
static inline void decimal(unsigned n){char out[12];unsigned i=sizeof(out);do{out[--i]='0'+n%10;n/=10;}while(n);write(1,out+i,sizeof(out)-i);}
#endif

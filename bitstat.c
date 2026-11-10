#include <stdio.h>
#include <string.h>
#include <sys/utsname.h>

int main(void)
{
    struct utsname os;
    unsigned bits;

    if (uname(&os) == -1) {
        perror("bitstat: uname");
        return 1;
    }

    if (strcmp(os.machine, "x86_64") == 0) {
        bits = 64;
    } else if (strlen(os.machine) == 4 && os.machine[0] == 'i' &&
               os.machine[1] >= '3' && os.machine[1] <= '6' &&
               strcmp(os.machine + 2, "86") == 0) {
        bits = 32;
    } else {
        fprintf(stderr, "bitstat: unsupported architecture: %s\n", os.machine);
        return 1;
    }

    printf("%s: %u-bit (%s)\n", os.sysname, bits, os.machine);
    return 0;
}

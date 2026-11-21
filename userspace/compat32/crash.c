/* Exercise normal user-fault isolation by executing an undefined instruction. */
int main(void){__asm__ volatile("ud2");return 1;}

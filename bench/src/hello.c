#include <unistd.h>
#include <stdlib.h>
int main(int c,char**v){ if(c>1) sleep(atoi(v[1])); return 0; }

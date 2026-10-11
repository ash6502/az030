/* hello.c -- compile with: cc -o hello hello.c */
#include <stdio.h>

int main(int argc, char **argv)
{
    int i;
    printf("hello, world\n");
    for (i = 1; i < argc; i++)
        printf("argument %d: %s\n", i, argv[i]);
    return 0;
}

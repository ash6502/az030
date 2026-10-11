/* primes.c -- the sieve of Eratosthenes. cc -o primes primes.c && ./primes 100 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    int n = argc > 1 ? atoi(argv[1]) : 100;
    char *composite;
    int i, j, count = 0;
    if (n < 2) {
        fprintf(stderr, "usage: primes N (N >= 2)\n");
        return 1;
    }
    composite = calloc(n + 1, 1);
    for (i = 2; i * i <= n; i++)
        if (!composite[i])
            for (j = i * i; j <= n; j += i)
                composite[j] = 1;
    for (i = 2; i <= n; i++)
        if (!composite[i]) {
            printf("%d%c", i, ++count % 10 ? ' ' : '\n');
        }
    printf("\n%d primes up to %d\n", count, n);
    free(composite);
    return 0;
}

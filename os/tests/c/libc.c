/* Library behaviour: compared between the host libc and AZ/UX's. */
#include <ctype.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

static int cmp_int(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }
static int cmp_str(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static jmp_buf jb;
static void jump(int n) { longjmp(jb, n); }

int main(void)
{
    int a[20], i;
    char buf[100], *words[] = {"pear", "apple", "fig", "banana", "cherry"};
    unsigned seed = 12345;

    for (i = 0; i < 20; i++) {
        seed = seed * 1103515245u + 12345u;
        a[i] = (int)(seed >> 16) % 1000;
    }
    qsort(a, 20, sizeof a[0], cmp_int);
    for (i = 0; i < 20; i++)
        printf("%d ", a[i]);
    printf("\n");
    qsort(words, 5, sizeof words[0], cmp_str);
    for (i = 0; i < 5; i++)
        printf("%s ", words[i]);
    printf("\n");
    {
        int key = a[7];
        int *found = bsearch(&key, a, 20, sizeof a[0], cmp_int);
        printf("bsearch: %d\n", found ? *found : -1);
    }

    strcpy(buf, "Hello");
    strcat(buf, ", World");
    printf("%s %d %d %d\n", buf, (int)strlen(buf), strcmp("abc", "abd") < 0, strncmp("abcx", "abcy", 3));
    printf("%s|%s|%s\n", strchr(buf, 'o'), strrchr(buf, 'o'), strstr(buf, "Wor"));
    printf("%d %d\n", (int)strspn("aaab", "a"), (int)strcspn("hello", "l"));
    {
        char t[] = "a,b,,c";
        char *tok = strtok(t, ",");
        while (tok) {
            printf("[%s]", tok);
            tok = strtok(0, ",");
        }
        printf("\n");
    }
    printf("%d %d %d %c %c\n", isalpha('x'), isdigit('5') != 0, isspace('\t') != 0, toupper('q'), tolower('Q'));
    printf("%ld %lu %d %g %g\n", strtol("-123abc", 0, 10), strtoul("ff", 0, 16), atoi("  42"), strtod("1.5e3", 0), atof("-0.25"));
    printf("%lld\n", strtoll("9000000000", 0, 10));

    {
        char *p = malloc(10);
        char *q;
        strcpy(p, "123456789");
        q = realloc(p, 1000);
        printf("realloc: %s\n", q);
        free(q);
        for (i = 0; i < 200; i++) {
            void *x = malloc(i * 37 % 500 + 1);
            void *y = calloc(i % 13 + 1, 8);
            memset(x, i, i * 37 % 500 + 1);
            free(x);
            if (i % 3)
                free(y);
        }
        printf("malloc ok\n");
    }

    {
        int r = setjmp(jb);
        if (r == 0) {
            jump(7);
            printf("not reached\n");
        } else {
            printf("longjmp: %d\n", r);
        }
    }

    printf("math: %.6f %.6f %.6f %.6f\n", sqrt(2.0), sin(1.0), cos(1.0), atan2(1.0, -1.0));
    printf("math2: %.6f %.6f %.6f %.6f\n", exp(1.0), log(10.0), pow(2.0, 10.5), fmod(10.0, 3.0));
    printf("math3: %g %g %g %g %g\n", floor(-2.5), ceil(-2.5), round(2.5), fabs(-3.0), trunc(-7.9));
    {
        int e;
        double m = frexp(48.0, &e);
        printf("frexp: %g %d %g\n", m, e, ldexp(0.75, 4));
    }

    {
        time_t t = 1000000000;
        struct tm *tm = gmtime(&t);
        strftime(buf, sizeof buf, "%Y-%m-%d %H:%M:%S %a %b %j", tm);
        printf("time: %s %ld\n", buf, (long)timegm(tm));
    }
    {
        char out[64];
        snprintf(out, 8, "%s", "truncated string");
        printf("snprintf: %s %d\n", out, snprintf(0, 0, "%d", 123456));
    }
    return 0;
}

/* Harder cases for the code generator. */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

struct big { int a[10]; char tag; double d; };
struct node { int val; struct node *next; };
struct pair { int64_t k; int v; };

static struct big make_big(int seed)
{
    struct big b;
    int i;
    for (i = 0; i < 10; i++)
        b.a[i] = seed * i;
    b.tag = (char)('A' + seed);
    b.d = seed / 4.0;
    return b;
}

static int sum_big(struct big b)
{
    int s = 0, i;
    for (i = 0; i < 10; i++)
        s += b.a[i];
    return s + b.tag + (int)(b.d * 100);
}

static int many(int a, int b, int c, int d, int e, int f, int g, int h, int i, int j)
{
    int x1 = a * b, x2 = c * d, x3 = e * f, x4 = g * h, x5 = i * j;
    int y1 = x1 + x2, y2 = x3 - x4, y3 = x5 ^ x1, y4 = x2 | x3;
    return (y1 * y2 + y3 - y4) % 100003 + (a ? b / a : 0);
}

/* recursive descent calculator */
static const char *p;
static long expr(void);
static long primary(void)
{
    long v;
    while (*p == ' ')
        p++;
    if (*p == '(') {
        p++;
        v = expr();
        p++;
        return v;
    }
    if (*p == '-') {
        p++;
        return -primary();
    }
    v = 0;
    while (isdigit((unsigned char)*p))
        v = v * 10 + (*p++ - '0');
    return v;
}
static long term(void)
{
    long v = primary();
    for (;;) {
        while (*p == ' ')
            p++;
        if (*p == '*') {
            p++;
            v *= primary();
        } else if (*p == '/') {
            p++;
            v /= primary();
        } else if (*p == '%') {
            p++;
            v %= primary();
        } else
            return v;
    }
}
static long expr(void)
{
    long v = term();
    for (;;) {
        while (*p == ' ')
            p++;
        if (*p == '+') {
            p++;
            v += term();
        } else if (*p == '-') {
            p++;
            v -= term();
        } else
            return v;
    }
}
static long calc(const char *s)
{
    p = s;
    return expr();
}

static int static_val = 42;
static int *static_ptr = &static_val;
static const char *const msgs[] = {"zero", "one", "two"};
static const char *msgp = "xone" + 1;
static int grid[3][3] = {{1, 2, 3}, {4, 5, 6}, {7, 8, 9}};
static struct pair pairs[] = {{5, 1}, {-3, 2}, {1LL << 35, 3}};

static int cmp_pair(const void *a, const void *b)
{
    const struct pair *x = a, *y = b;
    return x->k < y->k ? -1 : x->k > y->k;
}

int main(void)
{
    int i, j;
    {
        struct big b = make_big(3), c;
        struct big *pb = &c;
        *pb = b;
        pb->a[2] += 100;
        printf("big: %d %d %c %g\n", sum_big(b), sum_big(c), c.tag, c.d);
    }
    printf("many: %d %d\n", many(1, 2, 3, 4, 5, 6, 7, 8, 9, 10), many(-7, 13, 0, 99, 1000, -1000, 3, 3, 17, 4));
    printf("calc: %ld %ld %ld\n", calc("1 + 2 * 3"), calc("(10 - 4) * (3 + -2) / 2"), calc("100 % 7 - -3"));
    {
        struct node *head = 0, *n;
        int total = 0;
        for (i = 0; i < 50; i++) {
            n = malloc(sizeof *n);
            n->val = i * i;
            n->next = head;
            head = n;
        }
        for (n = head; n; n = n->next)
            total += n->val % 7;
        while (head) {
            n = head->next;
            free(head);
            head = n;
        }
        printf("list: %d\n", total);
    }
    printf("statics: %d %s %s %d %lld\n", *static_ptr, msgp, msgs[2], grid[2][1] + grid[0][2], (long long)pairs[2].k);
    qsort(pairs, 3, sizeof pairs[0], cmp_pair);
    printf("pairs: %d %d %d\n", pairs[0].v, pairs[1].v, pairs[2].v);
    {
        int a[8] = {5, 3, 8, 1, 9, 2, 7, 4};
        int *q = a, *e = a + 8, mx = *q;
        while (++q < e)
            mx = *q > mx ? *q : mx;
        a[a[3]] += a[a[5]] *= 2;
        printf("arr: %d %d %d %d\n", mx, a[1], a[2], (int)(e - a));
    }
    {
        int64_t x = -1000000000000LL, y = 7;
        uint64_t u = 1;
        int sh;
        printf("ll: %lld %lld %lld %lld\n", (long long)(x / y), (long long)(x % y), (long long)(x / -y), (long long)(-x % -y));
        for (sh = 0; sh < 64; sh += 13)
            printf("%llx ", (unsigned long long)(u << sh));
        printf("\n");
        printf("llcmp: %d %d %d %d\n", x < y, (uint64_t)x > (uint64_t)y, x == -1000000000000LL, (x >> 20) == -953675);
        printf("llconv: %d %lld %g %lld\n", (int)x, (long long)(int)-5, (double)x, (long long)1e15);
    }
    {
        unsigned char c = 200;
        signed char sc = -100;
        unsigned u = 3;
        int k = -1;
        c += 100;
        sc -= 100;
        printf("wrap: %d %d %d %u\n", c, sc, u > k, u - 5);
        printf("chararith: %d %d\n", 'a' + 1, (char)('z' - 'a') * 3);
    }
    {
        double d = 3.75;
        float f = -2.5f;
        int iv = (int)d;
        unsigned uv = (unsigned)3000000000.5;
        double back = uv;
        printf("fconv: %d %d %u %.1f %d %g\n", iv, (int)f, uv, back, (int)(f * d), (float)1 / 3);
        printf("fops: %g %g %g %d\n", d * f, d / f, -d + f * 2, d > f && f < 0);
    }
    {
        int n = 0;
    again:
        n++;
        if (n < 5)
            goto again;
        printf("goto: %d\n", n);
    }
    {
        struct pair *pp = &(struct pair){.v = 9, .k = 77};
        int *arr = (int[]){10, 20, 30};
        printf("compound: %lld %d %d\n", (long long)pp->k, pp->v, arr[1] + arr[2]);
    }
    {
        char buf[32];
        int len = 0;
        for (i = 0; i < 5; i++)
            for (j = 0; j < i; j++)
                buf[len++] = (char)('0' + (i * j) % 10);
        buf[len] = 0;
        printf("nested: %s %d\n", buf, len);
    }
    {
        int x = 5, y = 10, z;
        z = x++ + ++y;
        z += (x, y) * 2;
        z -= x > y ? x : y > 8 ? 100 : 200;
        printf("ops: %d %d %d\n", x, y, z);
    }
    {
        static char text[] = "The quick brown fox";
        char *w = strtok(text, " ");
        int count = 0;
        while (w) {
            count += (int)strlen(w);
            w = strtok(0, " ");
        }
        printf("words: %d\n", count);
    }
    return 0;
}

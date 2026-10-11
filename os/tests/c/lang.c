/* C language features: compared between the host compiler and azcc. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdarg.h>
#include <stdlib.h>

struct point { int x, y; };
struct rect { struct point a, b; char name[8]; };
union u { int32_t i; unsigned char b[4]; float f; };
struct bits { unsigned a : 3; signed b : 5; unsigned c : 12; unsigned d : 1; };
enum color { RED, GREEN = 5, BLUE };
typedef int (*binop)(int, int);

static int add(int a, int b) { return a + b; }
static int sub(int a, int b) { return a - b; }
static int counter(void) { static int n; return ++n; }
static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
static struct point mkpoint(int x, int y) { struct point p; p.x = x; p.y = y; return p; }
static int area(struct rect r) { return (r.b.x - r.a.x) * (r.b.y - r.a.y); }

static int sum(int n, ...)
{
    va_list ap;
    int s = 0, i;
    va_start(ap, n);
    for (i = 0; i < n; i++)
        s += va_arg(ap, int);
    va_end(ap);
    return s;
}

static double dsum(int n, ...)
{
    va_list ap;
    double s = 0;
    int i;
    va_start(ap, n);
    for (i = 0; i < n; i++)
        s += va_arg(ap, double);
    va_end(ap);
    return s;
}

int table[5] = {1, 2, 3};
const char *names[] = {"zero", "one", "two"};
struct point origin = {0, 0}, corner = {.y = 7, .x = 3};
char greeting[] = "hello";
int matrix[3][4];
int64_t bigs[] = {1, -1, 1LL << 40, -(1LL << 50)};

int main(void)
{
    int i, j;
    unsigned u = 0xF0000000u;
    int32_t s = -7;
    int64_t ll = 123456789012345LL;
    uint64_t ull = 0xFEDCBA9876543210ULL;
    char c = -3;
    unsigned char uc = 250;
    short sh = -1234;
    unsigned short us = 65000;

    printf("arith: %d %d %d %d %d\n", 17 / 5, -17 / 5, 17 % 5, -17 % 5, 17 % -5);
    printf("unsigned: %u %u %u %x\n", u / 3, u % 7, u >> 4, u << 1);
    printf("signed shift: %d %d\n", s >> 1, s << 3);
    printf("chars: %d %d %d %d\n", c, uc, (char)(uc + 10), (unsigned char)c);
    printf("shorts: %d %d %d\n", sh, us, (short)(us + 1000));
    printf("conv: %d %u %d\n", (int)3.99, (unsigned)4000000000.0, (int)-2.5);
    printf("ll: %lld %lld %lld %lld\n", (long long)ll, (long long)(ll * 3), (long long)(ll / 7), (long long)(ll % 1000));
    printf("ull: %llx %llu %llx %llx\n", (unsigned long long)ull, (unsigned long long)(ull / 10), (unsigned long long)(ull >> 12), (unsigned long long)(ull << 8));
    printf("ll ops: %lld %lld %d %d\n", (long long)-ll, (long long)(ll ^ 0xFFFF), ll > 1000000LL, ll < -5LL);
    for (i = 0; i < 4; i++)
        printf("big[%d]=%lld ", i, (long long)bigs[i]);
    printf("\n");

    printf("fib: %d\n", fib(20));
    printf("table: %d %d %d %d %d (%d)\n", table[0], table[1], table[2], table[3], table[4], (int)(sizeof table / sizeof table[0]));
    printf("names: %s %s %s\n", names[0], names[1], names[2]);
    printf("points: %d,%d %d,%d\n", origin.x, origin.y, corner.x, corner.y);
    printf("greeting: %s %d\n", greeting, (int)sizeof greeting);

    for (i = 0; i < 3; i++)
        for (j = 0; j < 4; j++)
            matrix[i][j] = i * 10 + j;
    printf("matrix: %d %d %d\n", matrix[0][3], matrix[2][1], *(*(matrix + 1) + 2));

    {
        struct rect r = {{1, 2}, {11, 7}, "box"};
        struct rect r2 = r;
        struct point p = mkpoint(4, 5);
        r2.a.x = 6;
        printf("struct: %d %d %s %d %d %d\n", area(r), area(r2), r2.name, p.x, p.y, (int)sizeof(struct rect));
    }
    {
        union u x;
        x.i = 0x01020304;
        x.b[0] = x.b[3] = 9;
        printf("union: %x\n", (unsigned)(x.i & 0x00FFFF00));
        x.f = 1.0f;
        printf("float bits: %x\n", (unsigned)x.i);
    }
    {
        struct bits b;
        memset(&b, 0, sizeof b);
        b.a = 5;
        b.b = -9;
        b.c = 4000;
        b.d = 1;
        b.a += 3;
        printf("bits: %u %d %u %u\n", b.a, b.b, b.c, b.d);
    }
    printf("enum: %d %d %d\n", RED, GREEN, BLUE);
    {
        binop ops[2] = {add, sub};
        printf("fnptr: %d %d\n", ops[0](7, 3), (*ops[1])(7, 3));
    }
    i = counter();
    j = counter();
    printf("static: %d %d %d\n", i, j, counter());
    printf("varargs: %d %g\n", sum(5, 1, 2, 3, 4, 5), dsum(3, 0.5, 1.25, 2.0));
    {
        int x = 10, *p = &x, **pp = &p;
        **pp += 5;
        printf("ptr: %d %d\n", x, (int)(&table[4] - &table[1]));
    }
    {
        int k = 0, n = 0;
        while (k < 100) {
            k += 7;
            if (k % 3 == 0)
                continue;
            if (k > 80)
                break;
            n++;
        }
        do n += 100; while (n < 300);
        printf("loops: %d %d\n", k, n);
    }
    {
        int v, total = 0;
        for (v = 0; v < 12; v++) {
            switch (v) {
            case 0: total += 1; break;
            case 1: case 2: total += 10; break;
            case 5: total += 100;
            case 6: total += 1000; break;
            case 11: total += 5; break;
            default: total += 3;
            }
        }
        printf("switch: %d\n", total);
    }
    {
        int a = 5, b = 0;
        int r = (a > 3 && b == 0) || (a / b > 1);
        printf("logic: %d %d %d %d\n", r, !a, a ? 11 : 22, (b, a));
    }
    {
        double d = 3.0;
        float f = 2.5f;
        printf("float: %.3f %.2f %g %e %f\n", d / 7, f * f, 1.0 / 3, 12345.678, -0.0001);
        printf("float2: %g %g %g %.10g %5.1f|%-8.3e|\n", 100000.0, 1000000.0, 0.0001, 3.14159265358979, 2.25, 0.000123);
        printf("cmp: %d %d %d\n", d > f, d == 3, f < 2.4);
    }
    {
        char buf[64];
        int n = sprintf(buf, "%5d|%-5d|%05d|%x|%#o|%c|%s|%.2s", 42, 42, 42, 255, 8, 'Z', "str", "abc");
        printf("sprintf: %s (%d)\n", buf, n);
        {
            int a, b2;
            char word[16];
            double dv;
            int got = sscanf("12 0x1f hello 2.5", "%d %i %15s %lf", &a, &b2, word, &dv);
            printf("sscanf: %d %d %d %s %g\n", got, a, b2, word, dv);
        }
    }
    return 0;
}

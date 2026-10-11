/* Math functions written in C (the rest use the FPU directly in math.s). */

#include <errno.h>
#include <math.h>

typedef union {
    double d;
    struct {
        unsigned long hi, lo;
    } w;
} dbits;

int __fpclassify(double x)
{
    dbits b;
    unsigned long e;
    b.d = x;
    e = (b.w.hi >> 20) & 0x7FF;
    if (e == 0x7FF)
        return ((b.w.hi & 0xFFFFF) | b.w.lo) ? FP_NAN : FP_INFINITE;
    if (e == 0)
        return ((b.w.hi & 0xFFFFF) | b.w.lo) ? FP_SUBNORMAL : FP_ZERO;
    return FP_NORMAL;
}

int __signbit(double x)
{
    dbits b;
    b.d = x;
    return (int)(b.w.hi >> 31);
}

double copysign(double x, double y)
{
    dbits a, b;
    a.d = x;
    b.d = y;
    a.w.hi = (a.w.hi & 0x7FFFFFFFUL) | (b.w.hi & 0x80000000UL);
    return a.d;
}

double nan(const char *s)
{
    (void)s;
    return 0.0 / 0.0;
}

double floor(double x)
{
    double t = trunc(x);
    if (x < 0 && t != x)
        t -= 1.0;
    return t;
}

double ceil(double x)
{
    double t = trunc(x);
    if (x > 0 && t != x)
        t += 1.0;
    return t;
}

double round(double x)
{
    if (x < 0)
        return -floor(-x + 0.5);
    return floor(x + 0.5);
}

long lround(double x) { return (long)round(x); }
long lrint(double x) { return (long)rint(x); }

double modf(double x, double *ip)
{
    double t = trunc(x);
    *ip = t;
    if (isinf(x))
        return copysign(0.0, x);
    return x - t;
}

double fmin(double a, double b)
{
    if (isnan(a))
        return b;
    if (isnan(b))
        return a;
    return a < b ? a : b;
}

double fmax(double a, double b)
{
    if (isnan(a))
        return b;
    if (isnan(b))
        return a;
    return a > b ? a : b;
}

double fdim(double a, double b) { return a > b ? a - b : 0.0; }

double hypot(double x, double y)
{
    x = fabs(x);
    y = fabs(y);
    if (x < y) {
        double t = x;
        x = y;
        y = t;
    }
    if (x == 0.0)
        return 0.0;
    y /= x;
    return x * sqrt(1.0 + y * y);
}

double cbrt(double x)
{
    double r;
    if (x == 0.0 || isnan(x) || isinf(x))
        return x;
    r = exp(log(fabs(x)) / 3.0);
    /* one Newton step */
    r = r - (r * r * r - fabs(x)) / (3.0 * r * r);
    return x < 0 ? -r : r;
}

double asinh(double x) { return log(x + sqrt(x * x + 1.0)); }
double acosh(double x) { return log(x + sqrt(x * x - 1.0)); }

double pow(double x, double y)
{
    double iy;
    if (y == 0.0)
        return 1.0;
    if (x == 1.0)
        return 1.0;
    if (isnan(x) || isnan(y))
        return 0.0 / 0.0;
    if (x == 0.0) {
        if (y < 0) {
            errno = ERANGE;
            return 1.0 / 0.0;
        }
        return 0.0;
    }
    /* integral exponents: repeated squaring, exact for small values */
    if (modf(y, &iy) == 0.0 && fabs(y) < 2147483647.0) {
        long n = (long)(y < 0 ? -y : y);
        double r = 1.0, b = x;
        while (n) {
            if (n & 1)
                r *= b;
            b *= b;
            n >>= 1;
        }
        return y < 0 ? 1.0 / r : r;
    }
    if (x < 0) {
        errno = EDOM;
        return 0.0 / 0.0;
    }
    return exp(y * log(x));
}

float powf(float x, float y) { return (float)pow(x, y); }
float floorf(float x) { return (float)floor(x); }
float ceilf(float x) { return (float)ceil(x); }

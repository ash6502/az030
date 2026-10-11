#ifndef _MATH_H
#define _MATH_H

#define M_E 2.7182818284590452354
#define M_LOG2E 1.4426950408889634074
#define M_LOG10E 0.43429448190325182765
#define M_LN2 0.69314718055994530942
#define M_LN10 2.30258509299404568402
#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.78539816339744830962
#define M_1_PI 0.31830988618379067154
#define M_2_PI 0.63661977236758134308
#define M_2_SQRTPI 1.12837916709551257390
#define M_SQRT2 1.41421356237309504880
#define M_SQRT1_2 0.70710678118654752440

#define HUGE_VAL (1.0 / 0.0)
#define HUGE_VALF ((float)HUGE_VAL)
#define INFINITY ((float)HUGE_VAL)
#define NAN (0.0f / 0.0f)

#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4

double sqrt(double x);
double cbrt(double x);
double hypot(double x, double y);
double sin(double x);
double cos(double x);
double tan(double x);
double asin(double x);
double acos(double x);
double atan(double x);
double atan2(double y, double x);
double sinh(double x);
double cosh(double x);
double tanh(double x);
double asinh(double x);
double acosh(double x);
double atanh(double x);
double exp(double x);
double exp2(double x);
double expm1(double x);
double log(double x);
double log10(double x);
double log2(double x);
double log1p(double x);
double pow(double x, double y);
double fabs(double x);
double floor(double x);
double ceil(double x);
double round(double x);
double trunc(double x);
double rint(double x);
double nearbyint(double x);
long lround(double x);
long lrint(double x);
double fmod(double x, double y);
double remainder(double x, double y);
double modf(double x, double *ip);
double frexp(double x, int *e);
double ldexp(double x, int e);
double scalbn(double x, int e);
double fmin(double a, double b);
double fmax(double a, double b);
double fdim(double a, double b);
double copysign(double x, double y);
double nan(const char *s);

float sqrtf(float x);
float sinf(float x);
float cosf(float x);
float tanf(float x);
float atanf(float x);
float expf(float x);
float logf(float x);
float fabsf(float x);
float powf(float x, float y);
float floorf(float x);
float ceilf(float x);

int __fpclassify(double x);
int __signbit(double x);
#define fpclassify(x) __fpclassify(x)
#define isnan(x) (fpclassify(x) == FP_NAN)
#define isinf(x) (fpclassify(x) == FP_INFINITE)
#define isfinite(x) (fpclassify(x) >= FP_ZERO)
#define isnormal(x) (fpclassify(x) == FP_NORMAL)
#define signbit(x) __signbit(x)

#endif

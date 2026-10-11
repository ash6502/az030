/*
 * Run-time support for long long, called by compiled code (the leading
 * underscore of each C name plus the compiler's own prefix gives the
 * assembly names __divll etc.). Written with 32-bit operations only, since
 * 64-bit multiply, divide and shift would call these very functions.
 */

typedef union {
    long long s;
    unsigned long long u;
    struct {
        unsigned long hi, lo;
    } w;
} ll_t;

/* unsigned 64-bit division by shift and subtract */
static void udivmod(ll_t n, ll_t d, ll_t *q, ll_t *r)
{
    ll_t quo, rem;
    int i;
    quo.w.hi = quo.w.lo = 0;
    rem.w.hi = rem.w.lo = 0;
    if (d.w.hi == 0 && n.w.hi == 0) {
        quo.w.lo = n.w.lo / d.w.lo;
        rem.w.lo = n.w.lo % d.w.lo;
        *q = quo;
        *r = rem;
        return;
    }
    for (i = 63; i >= 0; i--) {
        unsigned long bit;
        /* rem = rem << 1 | bit i of n */
        rem.w.hi = (rem.w.hi << 1) | (rem.w.lo >> 31);
        bit = i >= 32 ? (n.w.hi >> (i - 32)) & 1 : (n.w.lo >> i) & 1;
        rem.w.lo = (rem.w.lo << 1) | bit;
        /* if rem >= d: rem -= d, set quotient bit */
        if (rem.w.hi > d.w.hi || (rem.w.hi == d.w.hi && rem.w.lo >= d.w.lo)) {
            unsigned long lo = rem.w.lo - d.w.lo;
            rem.w.hi = rem.w.hi - d.w.hi - (lo > rem.w.lo);
            rem.w.lo = lo;
            if (i >= 32)
                quo.w.hi |= 1UL << (i - 32);
            else
                quo.w.lo |= 1UL << i;
        }
    }
    *q = quo;
    *r = rem;
}

static ll_t neg(ll_t x)
{
    ll_t r;
    r.w.lo = ~x.w.lo + 1;
    r.w.hi = ~x.w.hi + (r.w.lo == 0);
    return r;
}

unsigned long long _udivll(unsigned long long a, unsigned long long b)
{
    ll_t x, y, q, r;
    x.u = a;
    y.u = b;
    udivmod(x, y, &q, &r);
    return q.u;
}

unsigned long long _umodll(unsigned long long a, unsigned long long b)
{
    ll_t x, y, q, r;
    x.u = a;
    y.u = b;
    udivmod(x, y, &q, &r);
    return r.u;
}

long long _divll(long long a, long long b)
{
    ll_t x, y, q, r;
    int negq = 0;
    x.s = a;
    y.s = b;
    if ((long)x.w.hi < 0) {
        x = neg(x);
        negq = !negq;
    }
    if ((long)y.w.hi < 0) {
        y = neg(y);
        negq = !negq;
    }
    udivmod(x, y, &q, &r);
    if (negq)
        q = neg(q);
    return q.s;
}

long long _modll(long long a, long long b)
{
    ll_t x, y, q, r;
    int negr = 0;
    x.s = a;
    y.s = b;
    if ((long)x.w.hi < 0) {
        x = neg(x);
        negr = 1;
    }
    if ((long)y.w.hi < 0)
        y = neg(y);
    udivmod(x, y, &q, &r);
    if (negr)
        r = neg(r);
    return r.s;
}

long long _ashlll(long long a, int n)
{
    ll_t x;
    x.s = a;
    n &= 63;
    if (n >= 32) {
        x.w.hi = x.w.lo << (n - 32);
        x.w.lo = 0;
    } else if (n > 0) {
        x.w.hi = (x.w.hi << n) | (x.w.lo >> (32 - n));
        x.w.lo <<= n;
    }
    return x.s;
}

long long _lshrll(long long a, int n)
{
    ll_t x;
    x.s = a;
    n &= 63;
    if (n >= 32) {
        x.w.lo = x.w.hi >> (n - 32);
        x.w.hi = 0;
    } else if (n > 0) {
        x.w.lo = (x.w.lo >> n) | (x.w.hi << (32 - n));
        x.w.hi >>= n;
    }
    return x.s;
}

long long _ashrll(long long a, int n)
{
    ll_t x;
    long hi;
    x.s = a;
    hi = (long)x.w.hi;
    n &= 63;
    if (n >= 32) {
        x.w.lo = (unsigned long)(hi >> (n - 32));
        x.w.hi = (unsigned long)(hi >> 31);
    } else if (n > 0) {
        x.w.lo = (x.w.lo >> n) | (x.w.hi << (32 - n));
        x.w.hi = (unsigned long)(hi >> n);
    }
    return x.s;
}

double _ulltod(unsigned long long a)
{
    ll_t x;
    x.u = a;
    return (double)x.w.hi * 4294967296.0 + (double)x.w.lo;
}

double _lltod(long long a)
{
    ll_t x;
    x.s = a;
    if ((long)x.w.hi < 0) {
        x = neg(x);
        return -((double)x.w.hi * 4294967296.0 + (double)x.w.lo);
    }
    return (double)x.w.hi * 4294967296.0 + (double)x.w.lo;
}

unsigned long long _dtoull(double d)
{
    ll_t x;
    double hi;
    if (d <= 0.0) {
        x.w.hi = x.w.lo = 0;
        return x.u;
    }
    if (d >= 18446744073709551616.0) {
        x.w.hi = x.w.lo = 0xFFFFFFFFUL;
        return x.u;
    }
    hi = d / 4294967296.0;
    x.w.hi = (unsigned long)hi;
    x.w.lo = (unsigned long)(d - (double)x.w.hi * 4294967296.0);
    return x.u;
}

long long _dtoll(double d)
{
    ll_t x;
    if (d < 0) {
        x.u = _dtoull(-d);
        x = neg(x);
        return x.s;
    }
    x.u = _dtoull(d);
    return x.s;
}

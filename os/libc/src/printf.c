/* The printf family. */

#include <errno.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct {
    void (*put)(void *ctx, const char *s, int n);
    void *ctx;
    int count;
} sink;

static void emit(sink *k, const char *s, int n)
{
    if (n > 0) {
        k->put(k->ctx, s, n);
        k->count += n;
    }
}

static void pad(sink *k, char c, int n)
{
    char buf[32];
    int i;
    for (i = 0; i < 32; i++)
        buf[i] = c;
    while (n > 0) {
        int m = n > 32 ? 32 : n;
        emit(k, buf, m);
        n -= m;
    }
}

static double p10(int e)
{
    double r = 1.0, b = 10.0;
    int neg = e < 0;
    if (neg)
        e = -e;
    while (e) {
        if (e & 1)
            r *= b;
        b *= b;
        e >>= 1;
    }
    return neg ? 1.0 / r : r;
}

/*
 * The first n (1..17) significant decimal digits of v > 0, rounded; returns
 * the decimal exponent of the first digit.
 */
static int sig_digits(double v, int n, char *out)
{
    int e = (int)floor(log10(v));
    unsigned long long r, lim = 1;
    double scaled;
    int i, s = n - 1 - e;
    for (i = 0; i < n; i++)
        lim *= 10;
    /* scale in two steps to stay in range for very small numbers */
    if (s > 300) {
        scaled = v * p10(300);
        scaled *= p10(s - 300);
    } else if (s < -300) {
        scaled = v / p10(300);
        scaled /= p10(-s - 300);
    } else if (s >= 0)
        scaled = v * p10(s);
    else
        scaled = v / p10(-s);
    {
        /* round half to even */
        double fl = floor(scaled), diff = scaled - fl;
        r = (unsigned long long)fl;
        if (diff > 0.5 || (diff == 0.5 && (r & 1)))
            r++;
    }
    if (r >= lim) {
        r = (r + 5) / 10;
        e++;
        if (r >= lim) {
            r /= 10;
        }
    } else if (r < lim / 10) {
        r = (unsigned long long)(scaled * 10.0 + 0.5);
        e--;
    }
    for (i = n - 1; i >= 0; i--) {
        out[i] = (char)('0' + (int)(r % 10));
        r /= 10;
    }
    out[n] = 0;
    return e;
}

#define FL_LEFT 1
#define FL_PLUS 2
#define FL_SPACE 4
#define FL_ALT 8
#define FL_ZERO 16

/* format a double into buf (no sign); returns the length */
static int fmt_double(char *buf, double v, int conv, int prec, int flags)
{
    char dig[24];
    int len = 0, e, i, upper = conv == 'E' || conv == 'G' || conv == 'F';
    int lc = conv | 32;
    if (v != v) {
        strcpy(buf, upper ? "NAN" : "nan");
        return 3;
    }
    if (v == 1.0 / 0.0) {
        strcpy(buf, upper ? "INF" : "inf");
        return 3;
    }
    if (prec < 0)
        prec = 6;
    if (lc == 'g') {
        int p = prec == 0 ? 1 : prec;
        int x;
        if (v == 0.0)
            x = 0;
        else
            x = sig_digits(v, p > 17 ? 17 : p, dig);
        if (x < -4 || x >= p) {
            lc = 'e';
            prec = p - 1;
        } else {
            lc = 'f';
            prec = p - 1 - x;
        }
        len = fmt_double(buf, v, upper ? (lc == 'e' ? 'E' : 'F') : lc, prec, flags);
        if (!(flags & FL_ALT) && strchr(buf, '.')) {
            /* strip trailing zeros (and the point) from the fraction */
            char *ep = strchr(buf, lc == 'e' ? (upper ? 'E' : 'e') : 0);
            char tail[16];
            char *end;
            tail[0] = 0;
            if (ep && lc == 'e') {
                strcpy(tail, ep);
                *ep = 0;
            }
            end = buf + strlen(buf);
            while (end > buf && end[-1] == '0')
                *--end = 0;
            if (end > buf && end[-1] == '.')
                *--end = 0;
            strcat(buf, tail);
            len = (int)strlen(buf);
        }
        return len;
    }
    if (lc == 'e') {
        int n = prec + 1;
        if (v == 0.0) {
            for (i = 0; i < n; i++)
                dig[i] = '0';
            e = 0;
        } else {
            e = sig_digits(v, n > 17 ? 17 : n, dig);
            for (i = 17; i < n; i++)
                dig[i] = '0';
        }
        buf[len++] = dig[0];
        if (prec > 0 || (flags & FL_ALT))
            buf[len++] = '.';
        for (i = 1; i < n; i++)
            buf[len++] = i < 24 ? dig[i] : '0';
        buf[len++] = upper ? 'E' : 'e';
        buf[len++] = e < 0 ? '-' : '+';
        if (e < 0)
            e = -e;
        if (e >= 100)
            buf[len++] = (char)('0' + e / 100);
        buf[len++] = (char)('0' + e / 10 % 10);
        buf[len++] = (char)('0' + e % 10);
        buf[len] = 0;
        return len;
    }
    /* %f */
    {
        int n, intdigits;
        if (v == 0.0) {
            e = 0;
            n = 0;
        } else {
            e = (int)floor(log10(v));
            n = e + 1 + prec;          /* significant digits needed */
            if (n > 17) {
                e = sig_digits(v, 17, dig);
                for (i = 17; i < n && i < 23; i++)
                    dig[i] = '0';
            } else if (n > 0) {
                e = sig_digits(v, n, dig);
                n = e + 1 + prec;      /* rounding may have bumped the exponent */
                if (n > 17)
                    n = 17;
                e = sig_digits(v, n > 0 ? n : 1, dig);
            } else {
                /* rounds to 0 or to one unit in the last place */
                if (v >= 0.5 * p10(-prec)) {
                    e = -prec;
                    dig[0] = '1';
                    n = 1;
                } else {
                    n = 0;
                }
            }
        }
        /* digit k (0-based) has decimal weight e-k */
        intdigits = e >= 0 ? e + 1 : 1;
        for (i = 0; i < intdigits && len < 340; i++) {
            int w = intdigits - 1 - i;     /* decimal weight of this digit */
            int idx = e - w;
            buf[len++] = (n > 0 && idx >= 0 && idx < n && idx < 23) ? dig[idx] : '0';
        }
        if (prec > 0 || (flags & FL_ALT))
            buf[len++] = '.';
        for (i = 1; i <= prec && len < 380; i++) {
            /* weight -i */
            int idx = e + i;
            buf[len++] = (n > 0 && idx >= 0 && idx < n && idx < 23) ? dig[idx] : '0';
        }
        buf[len] = 0;
        return len;
    }
}

static int vformat(sink *k, const char *fmt, va_list ap)
{
    char buf[400];
    for (; *fmt; fmt++) {
        int flags = 0, width = -1, prec = -1, lng = 0, size_t_mod = 0;
        int conv, neg = 0, len;
        const char *s;
        char prefix[3];
        int plen = 0;
        if (*fmt != '%') {
            const char *start = fmt;
            while (fmt[1] && fmt[1] != '%')
                fmt++;
            emit(k, start, (int)(fmt - start) + 1);
            continue;
        }
        fmt++;
        for (;; fmt++) {
            if (*fmt == '-')
                flags |= FL_LEFT;
            else if (*fmt == '+')
                flags |= FL_PLUS;
            else if (*fmt == ' ')
                flags |= FL_SPACE;
            else if (*fmt == '#')
                flags |= FL_ALT;
            else if (*fmt == '0')
                flags |= FL_ZERO;
            else
                break;
        }
        if (*fmt == '*') {
            width = va_arg(ap, int);
            if (width < 0) {
                flags |= FL_LEFT;
                width = -width;
            }
            fmt++;
        } else {
            while (*fmt >= '0' && *fmt <= '9') {
                if (width < 0)
                    width = 0;
                width = width * 10 + (*fmt++ - '0');
            }
        }
        if (*fmt == '.') {
            fmt++;
            prec = 0;
            if (*fmt == '*') {
                prec = va_arg(ap, int);
                fmt++;
            } else {
                while (*fmt >= '0' && *fmt <= '9')
                    prec = prec * 10 + (*fmt++ - '0');
            }
        }
        for (;; fmt++) {
            if (*fmt == 'l')
                lng++;
            else if (*fmt == 'h' || *fmt == 'L' || *fmt == 'q')
                lng += *fmt == 'q' ? 2 : 0;
            else if (*fmt == 'z' || *fmt == 't')
                size_t_mod = 1;
            else if (*fmt == 'j')
                lng = 2;
            else
                break;
        }
        conv = *fmt;
        if (!conv)
            break;
        s = buf;
        switch (conv) {
        case 'd':
        case 'i':
        case 'u':
        case 'x':
        case 'X':
        case 'o':
        case 'b':
        case 'p': {
            unsigned long long v;
            int base = 10, i = 0;
            char tmp[66];
            const char *digits = conv == 'X' ? "0123456789ABCDEF" : "0123456789abcdef";
            (void)size_t_mod;
            if (conv == 'p') {
                v = (unsigned long)va_arg(ap, void *);
                flags |= FL_ALT;
                base = 16;
            } else if (lng >= 2) {
                v = va_arg(ap, unsigned long long);
            } else {
                v = va_arg(ap, unsigned long);
                if (conv == 'd' || conv == 'i')
                    v = (unsigned long long)(long long)(long)v;
            }
            if (conv == 'd' || conv == 'i') {
                if ((long long)v < 0) {
                    neg = 1;
                    v = -v;
                }
            }
            if (conv == 'x' || conv == 'X')
                base = 16;
            else if (conv == 'o')
                base = 8;
            else if (conv == 'b')
                base = 2;
            if (lng >= 2 || v > 0xFFFFFFFFULL) {
                while (v) {
                    tmp[i++] = digits[(int)(v % base)];
                    v /= base;
                }
            } else {
                unsigned long w = (unsigned long)v;
                while (w) {
                    tmp[i++] = digits[w % base];
                    w /= base;
                }
            }
            if (prec < 0 && i == 0)
                tmp[i++] = '0';
            while (i < prec)
                tmp[i++] = '0';
            if (prec >= 0)
                flags &= ~FL_ZERO;
            if (conv == 'o' && (flags & FL_ALT) && (i == 0 || tmp[i - 1] != '0'))
                tmp[i++] = '0';
            len = 0;
            while (i)
                buf[len++] = tmp[--i];
            if (neg)
                prefix[plen++] = '-';
            else if ((conv == 'd' || conv == 'i') && (flags & FL_PLUS))
                prefix[plen++] = '+';
            else if ((conv == 'd' || conv == 'i') && (flags & FL_SPACE))
                prefix[plen++] = ' ';
            if ((flags & FL_ALT) && base == 16 && (len > 1 || buf[0] != '0' || conv == 'p')) {
                prefix[plen++] = '0';
                prefix[plen++] = conv == 'X' ? 'X' : 'x';
            }
            break;
        }
        case 'f':
        case 'F':
        case 'e':
        case 'E':
        case 'g':
        case 'G':
        case 'a':
        case 'A': {
            double v = va_arg(ap, double);
            if (v < 0 || (v == 0.0 && 1.0 / v < 0)) {
                neg = 1;
                v = -v;
            }
            if (conv == 'a' || conv == 'A')
                conv = conv == 'a' ? 'e' : 'E';
            len = fmt_double(buf, v, conv, prec, flags);
            if (v != v || v == 1.0 / 0.0)
                flags &= ~FL_ZERO;
            if (neg)
                prefix[plen++] = '-';
            else if (flags & FL_PLUS)
                prefix[plen++] = '+';
            else if (flags & FL_SPACE)
                prefix[plen++] = ' ';
            break;
        }
        case 'c':
            buf[0] = (char)va_arg(ap, int);
            len = 1;
            flags &= ~FL_ZERO;
            break;
        case 's':
            s = va_arg(ap, const char *);
            if (!s)
                s = "(null)";
            len = prec >= 0 ? (int)strnlen(s, prec) : (int)strlen(s);
            flags &= ~FL_ZERO;
            break;
        case 'n':
            if (lng >= 2)
                *va_arg(ap, long long *) = k->count;
            else
                *va_arg(ap, int *) = k->count;
            continue;
        case 'm':
            s = strerror(errno);
            len = (int)strlen(s);
            break;
        case '%':
        default:
            buf[0] = (char)conv;
            len = 1;
            flags &= ~FL_ZERO;
            break;
        }
        {
            int total = plen + len;
            int fill = width > total ? width - total : 0;
            if (!(flags & FL_LEFT) && !(flags & FL_ZERO))
                pad(k, ' ', fill);
            emit(k, prefix, plen);
            if (!(flags & FL_LEFT) && (flags & FL_ZERO))
                pad(k, '0', fill);
            emit(k, s, len);
            if (flags & FL_LEFT)
                pad(k, ' ', fill);
        }
    }
    return k->count;
}

static void put_file(void *ctx, const char *s, int n) { fwrite(s, 1, n, (FILE *)ctx); }

int vfprintf(FILE *f, const char *fmt, va_list ap)
{
    sink k;
    k.put = put_file;
    k.ctx = f;
    k.count = 0;
    return vformat(&k, fmt, ap);
}

int vprintf(const char *fmt, va_list ap) { return vfprintf(stdout, fmt, ap); }

int printf(const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vfprintf(stdout, fmt, ap);
    va_end(ap);
    return r;
}

int fprintf(FILE *f, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vfprintf(f, fmt, ap);
    va_end(ap);
    return r;
}

struct strbuf {
    char *p;
    size_t n;   /* room left, including the terminator */
};

static void put_str(void *ctx, const char *s, int n)
{
    struct strbuf *b = ctx;
    while (n-- > 0) {
        if (b->n > 1) {
            *b->p++ = *s;
            b->n--;
        }
        s++;
    }
}

int vsnprintf(char *s, size_t n, const char *fmt, va_list ap)
{
    sink k;
    struct strbuf b;
    int r;
    b.p = s;
    b.n = n;
    k.put = put_str;
    k.ctx = &b;
    k.count = 0;
    r = vformat(&k, fmt, ap);
    if (n)
        *b.p = 0;
    return r;
}

int vsprintf(char *s, const char *fmt, va_list ap) { return vsnprintf(s, 0x7FFFFFFF, fmt, ap); }

int snprintf(char *s, size_t n, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vsnprintf(s, n, fmt, ap);
    va_end(ap);
    return r;
}

int sprintf(char *s, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vsnprintf(s, 0x7FFFFFFF, fmt, ap);
    va_end(ap);
    return r;
}

struct dynbuf {
    char *p;
    int len, cap, failed;
};

static void put_dyn(void *ctx, const char *s, int n)
{
    struct dynbuf *b = ctx;
    if (b->failed)
        return;
    if (b->len + n + 1 > b->cap) {
        int cap = b->cap ? b->cap : 64;
        char *np;
        while (b->len + n + 1 > cap)
            cap *= 2;
        np = realloc(b->p, cap);
        if (!np) {
            b->failed = 1;
            return;
        }
        b->p = np;
        b->cap = cap;
    }
    memcpy(b->p + b->len, s, n);
    b->len += n;
}

int vasprintf(char **out, const char *fmt, va_list ap)
{
    sink k;
    struct dynbuf b;
    b.p = 0;
    b.len = b.cap = b.failed = 0;
    k.put = put_dyn;
    k.ctx = &b;
    k.count = 0;
    vformat(&k, fmt, ap);
    put_dyn(&b, "", 0);
    if (b.failed || !b.p) {
        free(b.p);
        *out = 0;
        return -1;
    }
    b.p[b.len] = 0;
    *out = b.p;
    return b.len;
}

int asprintf(char **out, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vasprintf(out, fmt, ap);
    va_end(ap);
    return r;
}

static void put_fd(void *ctx, const char *s, int n) { write(*(int *)ctx, s, n); }

int vdprintf(int fd, const char *fmt, va_list ap)
{
    sink k;
    k.put = put_fd;
    k.ctx = &fd;
    k.count = 0;
    return vformat(&k, fmt, ap);
}

int dprintf(int fd, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vdprintf(fd, fmt, ap);
    va_end(ap);
    return r;
}

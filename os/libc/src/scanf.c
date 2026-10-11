/* The scanf family. */

#include <ctype.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int (*get)(void *ctx);
    void (*unget)(void *ctx, int c);
    void *ctx;
    int count;      /* characters consumed */
} source;

static int get(source *s)
{
    int c = s->get(s->ctx);
    if (c != EOF)
        s->count++;
    return c;
}

static void unget(source *s, int c)
{
    if (c != EOF) {
        s->unget(s->ctx, c);
        s->count--;
    }
}

static int vscan(source *src, const char *fmt, va_list ap)
{
    int assigned = 0, c;
    for (; *fmt; fmt++) {
        int suppress = 0, width = 0, lng = 0, half = 0;
        if (isspace((unsigned char)*fmt)) {
            while ((c = get(src)) != EOF && isspace(c))
                ;
            unget(src, c);
            continue;
        }
        if (*fmt != '%' || fmt[1] == '%') {
            if (*fmt == '%')
                fmt++;
            c = get(src);
            if (c != (unsigned char)*fmt) {
                unget(src, c);
                return c == EOF && assigned == 0 ? EOF : assigned;
            }
            continue;
        }
        fmt++;
        if (*fmt == '*') {
            suppress = 1;
            fmt++;
        }
        while (isdigit((unsigned char)*fmt))
            width = width * 10 + (*fmt++ - '0');
        for (;; fmt++) {
            if (*fmt == 'l')
                lng++;
            else if (*fmt == 'h')
                half++;
            else if (*fmt == 'L' || *fmt == 'q' || *fmt == 'j')
                lng = 2;
            else if (*fmt == 'z' || *fmt == 't')
                ;
            else
                break;
        }
        if (!*fmt)
            break;
        if (*fmt == 'n') {
            if (!suppress)
                *va_arg(ap, int *) = src->count;
            continue;
        }
        if (*fmt != 'c' && *fmt != '[') {
            while ((c = get(src)) != EOF && isspace(c))
                ;
            unget(src, c);
        }
        if (width == 0)
            width = *fmt == 'c' ? 1 : 0x7FFFFFFF;
        switch (*fmt) {
        case 'd':
        case 'i':
        case 'u':
        case 'x':
        case 'X':
        case 'o':
        case 'p': {
            char buf[80];
            int n = 0, base = *fmt == 'd' || *fmt == 'u' ? 10 : *fmt == 'o' ? 8 : *fmt == 'i' ? 0 : 16;
            char *end;
            c = get(src);
            if ((c == '-' || c == '+') && n < width) {
                buf[n++] = (char)c;
                c = get(src);
            }
            if (c == '0' && n < width && (base == 0 || base == 16)) {
                buf[n++] = (char)c;
                c = get(src);
                if ((c | 32) == 'x' && n < width) {
                    buf[n++] = (char)c;
                    c = get(src);
                    base = 16;
                } else if (base == 0) {
                    base = 8;
                }
            }
            if (base == 0)
                base = 10;
            while (c != EOF && n < width && n < 79 &&
                   (base == 16 ? isxdigit(c) : base == 8 ? (c >= '0' && c <= '7') : isdigit(c))) {
                buf[n++] = (char)c;
                c = get(src);
            }
            unget(src, c);
            buf[n] = 0;
            if (n == 0 || (n == 1 && (buf[0] == '-' || buf[0] == '+')))
                return assigned ? assigned : (c == EOF ? EOF : 0);
            if (!suppress) {
                unsigned long long v = strtoull(buf, &end, base);
                if (buf[0] == '-' && *fmt != 'u')
                    v = (unsigned long long)strtoll(buf, &end, base);
                if (*fmt == 'p')
                    *va_arg(ap, void **) = (void *)(unsigned long)v;
                else if (lng >= 2)
                    *va_arg(ap, long long *) = (long long)v;
                else if (half >= 2)
                    *va_arg(ap, char *) = (char)v;
                else if (half)
                    *va_arg(ap, short *) = (short)v;
                else
                    *va_arg(ap, int *) = (int)v;
                assigned++;
            }
            break;
        }
        case 'f':
        case 'e':
        case 'E':
        case 'g':
        case 'G':
        case 'a': {
            char buf[80];
            int n = 0, seen_e = 0;
            c = get(src);
            while (c != EOF && n < width && n < 79) {
                if (isdigit(c) || c == '.' || ((c == '-' || c == '+') && (n == 0 || (buf[n - 1] | 32) == 'e'))) {
                } else if ((c | 32) == 'e' && !seen_e && n > 0) {
                    seen_e = 1;
                } else if (isalpha(c) && (n == 0 || isalpha((unsigned char)buf[n - 1])) && strchr("infatyINFATY", c)) {
                } else {
                    break;
                }
                buf[n++] = (char)c;
                c = get(src);
            }
            unget(src, c);
            buf[n] = 0;
            if (n == 0)
                return assigned ? assigned : (c == EOF ? EOF : 0);
            if (!suppress) {
                double v = strtod(buf, 0);
                if (lng)
                    *va_arg(ap, double *) = v;
                else
                    *va_arg(ap, float *) = (float)v;
                assigned++;
            }
            break;
        }
        case 's': {
            char *d = suppress ? 0 : va_arg(ap, char *);
            int n = 0;
            while (n < width && (c = get(src)) != EOF && !isspace(c)) {
                if (d)
                    d[n] = (char)c;
                n++;
            }
            if (n < width)
                unget(src, c);
            if (n == 0)
                return assigned ? assigned : EOF;
            if (d) {
                d[n] = 0;
                assigned++;
            }
            break;
        }
        case 'c': {
            char *d = suppress ? 0 : va_arg(ap, char *);
            int n = 0;
            while (n < width && (c = get(src)) != EOF) {
                if (d)
                    d[n] = (char)c;
                n++;
            }
            if (n == 0)
                return assigned ? assigned : EOF;
            if (d)
                assigned++;
            break;
        }
        case '[': {
            char set[256];
            int neg = 0, n = 0, i;
            char *d;
            fmt++;
            if (*fmt == '^') {
                neg = 1;
                fmt++;
            }
            memset(set, 0, sizeof set);
            if (*fmt == ']') {
                set[']'] = 1;
                fmt++;
            }
            while (*fmt && *fmt != ']') {
                if (fmt[1] == '-' && fmt[2] && fmt[2] != ']') {
                    for (i = (unsigned char)fmt[0]; i <= (unsigned char)fmt[2]; i++)
                        set[i] = 1;
                    fmt += 3;
                } else {
                    set[(unsigned char)*fmt++] = 1;
                }
            }
            d = suppress ? 0 : va_arg(ap, char *);
            while (n < width && (c = get(src)) != EOF && set[c] != neg) {
                if (d)
                    d[n] = (char)c;
                n++;
            }
            if (n < width)
                unget(src, c);
            if (n == 0)
                return assigned ? assigned : (c == EOF ? EOF : 0);
            if (d) {
                d[n] = 0;
                assigned++;
            }
            if (!*fmt)
                fmt--;
            break;
        }
        default:
            return assigned;
        }
    }
    return assigned;
}

static int file_get(void *ctx) { return fgetc((FILE *)ctx); }
static void file_unget(void *ctx, int c) { ungetc(c, (FILE *)ctx); }

int vfscanf(FILE *f, const char *fmt, va_list ap)
{
    source s;
    s.get = file_get;
    s.unget = file_unget;
    s.ctx = f;
    s.count = 0;
    return vscan(&s, fmt, ap);
}

int vscanf(const char *fmt, va_list ap) { return vfscanf(stdin, fmt, ap); }

int fscanf(FILE *f, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vfscanf(f, fmt, ap);
    va_end(ap);
    return r;
}

int scanf(const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vfscanf(stdin, fmt, ap);
    va_end(ap);
    return r;
}

static int str_get(void *ctx)
{
    const char **p = ctx;
    if (!**p)
        return EOF;
    return (unsigned char)*(*p)++;
}

static void str_unget(void *ctx, int c)
{
    const char **p = ctx;
    (void)c;
    (*p)--;
}

int vsscanf(const char *str, const char *fmt, va_list ap)
{
    source s;
    s.get = str_get;
    s.unget = str_unget;
    s.ctx = &str;
    s.count = 0;
    return vscan(&s, fmt, ap);
}

int sscanf(const char *str, const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vsscanf(str, fmt, ap);
    va_end(ap);
    return r;
}

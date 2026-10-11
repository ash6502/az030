/* <stdlib.h>: conversions, sorting, environment, exit, system */

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

/* ---- integer conversions ---- */

static unsigned long long strtox(const char *s, char **end, int base, int *neg, int *ok)
{
    const char *p = s;
    unsigned long long v = 0;
    int any = 0;
    *neg = 0;
    *ok = 1;
    while (isspace((unsigned char)*p))
        p++;
    if (*p == '+' || *p == '-')
        *neg = *p++ == '-';
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] | 32) == 'x' && isxdigit((unsigned char)p[2])) {
        p += 2;
        base = 16;
    } else if (base == 0 && p[0] == '0') {
        base = 8;
    } else if (base == 0) {
        base = 10;
    }
    for (;; p++) {
        int c = (unsigned char)*p, d;
        if (isdigit(c))
            d = c - '0';
        else if (isalpha(c))
            d = (c | 32) - 'a' + 10;
        else
            break;
        if (d >= base)
            break;
        if (v > (~0ULL - d) / base)
            *ok = 0;
        v = v * base + d;
        any = 1;
    }
    if (end)
        *end = (char *)(any ? p : s);
    return v;
}

long long strtoll(const char *s, char **end, int base)
{
    int neg, ok;
    unsigned long long v = strtox(s, end, base, &neg, &ok);
    if (!ok || v > (neg ? 9223372036854775808ULL : 9223372036854775807ULL)) {
        errno = ERANGE;
        return neg ? LLONG_MIN : LLONG_MAX;
    }
    return neg ? -(long long)v : (long long)v;
}

unsigned long long strtoull(const char *s, char **end, int base)
{
    int neg, ok;
    unsigned long long v = strtox(s, end, base, &neg, &ok);
    if (!ok) {
        errno = ERANGE;
        return ULLONG_MAX;
    }
    return neg ? -v : v;
}

long strtol(const char *s, char **end, int base)
{
    int neg, ok;
    unsigned long long v = strtox(s, end, base, &neg, &ok);
    if (!ok || v > (neg ? 2147483648ULL : 2147483647ULL)) {
        errno = ERANGE;
        return neg ? LONG_MIN : LONG_MAX;
    }
    return neg ? -(long)v : (long)v;
}

unsigned long strtoul(const char *s, char **end, int base)
{
    int neg, ok;
    unsigned long long v = strtox(s, end, base, &neg, &ok);
    if (!ok || v > 4294967295ULL) {
        errno = ERANGE;
        return ULONG_MAX;
    }
    return neg ? -(unsigned long)v : (unsigned long)v;
}

long long strtoimax(const char *s, char **end, int base) { return strtoll(s, end, base); }
unsigned long long strtoumax(const char *s, char **end, int base) { return strtoull(s, end, base); }

int atoi(const char *s) { return (int)strtol(s, 0, 10); }
long atol(const char *s) { return strtol(s, 0, 10); }
long long atoll(const char *s) { return strtoll(s, 0, 10); }

/* ---- floating-point conversion ---- */

static double pow10i(int e)
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

double strtod(const char *s, char **end)
{
    const char *p = s;
    double v = 0.0;
    int neg = 0, any = 0, exp = 0;
    long long mant = 0;
    int digits = 0;
    while (isspace((unsigned char)*p))
        p++;
    if (*p == '+' || *p == '-')
        neg = *p++ == '-';
    if (strncasecmp(p, "inf", 3) == 0) {
        p += strncasecmp(p, "infinity", 8) == 0 ? 8 : 3;
        if (end)
            *end = (char *)p;
        v = 1.0 / 0.0;
        return neg ? -v : v;
    }
    if (strncasecmp(p, "nan", 3) == 0) {
        p += 3;
        if (end)
            *end = (char *)p;
        return 0.0 / 0.0;
    }
    if (p[0] == '0' && (p[1] | 32) == 'x') {
        /* hexadecimal floating constant */
        double m = 0.0;
        int e2 = 0;
        p += 2;
        while (isxdigit((unsigned char)*p)) {
            int c = (unsigned char)*p++;
            m = m * 16.0 + (isdigit(c) ? c - '0' : (c | 32) - 'a' + 10);
            any = 1;
        }
        if (*p == '.') {
            p++;
            while (isxdigit((unsigned char)*p)) {
                int c = (unsigned char)*p++;
                m = m * 16.0 + (isdigit(c) ? c - '0' : (c | 32) - 'a' + 10);
                e2 -= 4;
                any = 1;
            }
        }
        if (any && (*p | 32) == 'p') {
            const char *q = p + 1;
            int en = 0, eneg = 0;
            if (*q == '+' || *q == '-')
                eneg = *q++ == '-';
            if (isdigit((unsigned char)*q)) {
                while (isdigit((unsigned char)*q))
                    en = en * 10 + (*q++ - '0');
                e2 += eneg ? -en : en;
                p = q;
            }
        }
        if (end)
            *end = (char *)(any ? p : s);
        v = ldexp(m, e2);
        return neg ? -v : v;
    }
    /* up to 18 significant digits exactly, then scale */
    while (isdigit((unsigned char)*p)) {
        if (digits < 18) {
            mant = mant * 10 + (*p - '0');
            if (mant)
                digits++;
        } else {
            exp++;
        }
        p++;
        any = 1;
    }
    if (*p == '.') {
        p++;
        while (isdigit((unsigned char)*p)) {
            if (digits < 18) {
                mant = mant * 10 + (*p - '0');
                if (mant)
                    digits++;
                exp--;
            }
            p++;
            any = 1;
        }
    }
    if (!any) {
        if (end)
            *end = (char *)s;
        return 0.0;
    }
    if ((*p | 32) == 'e') {
        const char *q = p + 1;
        int en = 0, eneg = 0;
        if (*q == '+' || *q == '-')
            eneg = *q++ == '-';
        if (isdigit((unsigned char)*q)) {
            while (isdigit((unsigned char)*q)) {
                if (en < 10000)
                    en = en * 10 + (*q - '0');
                q++;
            }
            exp += eneg ? -en : en;
            p = q;
        }
    }
    if (end)
        *end = (char *)p;
    v = (double)mant;
    if (exp > 0)
        v *= pow10i(exp);
    else if (exp < -300) {
        v /= pow10i(300);
        v /= pow10i(-exp - 300);
    } else if (exp < 0)
        v /= pow10i(-exp);
    if (v == 1.0 / 0.0 || (v == 0.0 && mant != 0))
        errno = ERANGE;
    return neg ? -v : v;
}

float strtof(const char *s, char **end) { return (float)strtod(s, end); }
long double strtold(const char *s, char **end) { return strtod(s, end); }
double atof(const char *s) { return strtod(s, 0); }

/* ---- arithmetic ---- */

int abs(int x) { return x < 0 ? -x : x; }
long labs(long x) { return x < 0 ? -x : x; }
long long llabs(long long x) { return x < 0 ? -x : x; }

div_t div(int a, int b)
{
    div_t r;
    r.quot = a / b;
    r.rem = a % b;
    return r;
}

ldiv_t ldiv(long a, long b)
{
    ldiv_t r;
    r.quot = a / b;
    r.rem = a % b;
    return r;
}

lldiv_t lldiv(long long a, long long b)
{
    lldiv_t r;
    r.quot = a / b;
    r.rem = a % b;
    return r;
}

static unsigned long long rand_state = 1;

int rand(void)
{
    rand_state = rand_state * 6364136223846793005ULL + 1442695040888963407ULL;
    return (int)((rand_state >> 33) & 0x7FFFFFFF);
}

void srand(unsigned seed) { rand_state = seed; }
long random(void) { return rand(); }
void srandom(unsigned seed) { srand(seed); }

/* ---- sorting and searching ---- */

static void swap_bytes(char *a, char *b, size_t n)
{
    while (n--) {
        char t = *a;
        *a++ = *b;
        *b++ = t;
    }
}

/* introsort-ish: quicksort with median of three, insertion sort for small ranges */
static void qs(char *base, size_t n, size_t size, int (*cmp)(const void *, const void *), int depth)
{
    while (n > 8) {
        char *lo = base, *hi = base + (n - 1) * size, *mid = base + (n / 2) * size;
        char *i, *j;
        if (depth-- == 0) {
            /* fall back to a simple shell sort for pathological input */
            size_t gap, k, m;
            for (gap = n / 2; gap > 0; gap /= 2)
                for (k = gap; k < n; k++)
                    for (m = k; m >= gap && cmp(base + (m - gap) * size, base + m * size) > 0; m -= gap)
                        swap_bytes(base + (m - gap) * size, base + m * size, size);
            return;
        }
        if (cmp(mid, lo) < 0)
            swap_bytes(mid, lo, size);
        if (cmp(hi, mid) < 0) {
            swap_bytes(hi, mid, size);
            if (cmp(mid, lo) < 0)
                swap_bytes(mid, lo, size);
        }
        /* pivot to the front */
        swap_bytes(lo, mid, size);
        i = lo + size;
        j = hi;
        for (;;) {
            while (i <= j && cmp(i, lo) < 0)
                i += size;
            while (i <= j && cmp(j, lo) > 0)
                j -= size;
            if (i >= j)
                break;
            swap_bytes(i, j, size);
            i += size;
            j -= size;
        }
        swap_bytes(lo, j, size);
        {
            size_t left = (j - base) / size;
            size_t right = n - left - 1;
            /* recurse into the smaller part */
            if (left < right) {
                qs(base, left, size, cmp, depth);
                base = j + size;
                n = right;
            } else {
                qs(j + size, right, size, cmp, depth);
                n = left;
            }
        }
    }
    {
        size_t k, m;
        for (k = 1; k < n; k++)
            for (m = k; m > 0 && cmp(base + (m - 1) * size, base + m * size) > 0; m--)
                swap_bytes(base + (m - 1) * size, base + m * size, size);
    }
}

void qsort(void *base, size_t n, size_t size, int (*cmp)(const void *, const void *))
{
    qs(base, n, size, cmp, 64);
}

void *bsearch(const void *key, const void *base, size_t n, size_t size, int (*cmp)(const void *, const void *))
{
    const char *b = base;
    while (n > 0) {
        const char *m = b + (n / 2) * size;
        int c = cmp(key, m);
        if (c == 0)
            return (void *)m;
        if (c > 0) {
            b = m + size;
            n = n - n / 2 - 1;
        } else {
            n /= 2;
        }
    }
    return 0;
}

/* ---- environment ---- */

extern char **environ;
static int env_owned;

char *getenv(const char *name)
{
    size_t n = strlen(name);
    char **e;
    if (!environ)
        return 0;
    for (e = environ; *e; e++)
        if (strncmp(*e, name, n) == 0 && (*e)[n] == '=')
            return *e + n + 1;
    return 0;
}

static int env_count(void)
{
    int n = 0;
    if (environ)
        while (environ[n])
            n++;
    return n;
}

int putenv(char *s)
{
    char *eq = strchr(s, '=');
    size_t n;
    int i, count;
    char **ne;
    if (!eq) {
        errno = EINVAL;
        return -1;
    }
    n = eq - s;
    count = env_count();
    for (i = 0; i < count; i++)
        if (strncmp(environ[i], s, n) == 0 && environ[i][n] == '=') {
            environ[i] = s;
            return 0;
        }
    ne = malloc((count + 2) * sizeof(char *));
    if (!ne)
        return -1;
    for (i = 0; i < count; i++)
        ne[i] = environ[i];
    ne[count] = s;
    ne[count + 1] = 0;
    if (env_owned)
        free(environ);
    environ = ne;
    env_owned = 1;
    return 0;
}

int setenv(const char *name, const char *value, int overwrite)
{
    char *s;
    if (!name || !*name || strchr(name, '=')) {
        errno = EINVAL;
        return -1;
    }
    if (!overwrite && getenv(name))
        return 0;
    s = malloc(strlen(name) + strlen(value) + 2);
    if (!s)
        return -1;
    strcpy(s, name);
    strcat(s, "=");
    strcat(s, value);
    return putenv(s);
}

int unsetenv(const char *name)
{
    size_t n = strlen(name);
    int i, j;
    if (!environ)
        return 0;
    for (i = j = 0; environ[i]; i++)
        if (!(strncmp(environ[i], name, n) == 0 && environ[i][n] == '='))
            environ[j++] = environ[i];
    environ[j] = 0;
    return 0;
}

int clearenv(void)
{
    if (environ)
        environ[0] = 0;
    return 0;
}

/* ---- exit ---- */

#define MAX_ATEXIT 32
static void (*atexit_fns[MAX_ATEXIT])(void);
static int atexit_count;
void (*__stdio_exit)(void);

int atexit(void (*f)(void))
{
    if (atexit_count >= MAX_ATEXIT)
        return -1;
    atexit_fns[atexit_count++] = f;
    return 0;
}

void exit(int status)
{
    while (atexit_count > 0)
        atexit_fns[--atexit_count]();
    if (__stdio_exit)
        __stdio_exit();
    _exit(status);
}

void abort(void)
{
    signal(SIGABRT, SIG_DFL);
    raise(SIGABRT);
    _exit(134);
}

int system(const char *cmd)
{
    pid_t pid;
    int status;
    sighandler_t oi, oq;
    if (!cmd)
        return access("/bin/sh", X_OK) == 0;
    oi = signal(SIGINT, SIG_IGN);
    oq = signal(SIGQUIT, SIG_IGN);
    pid = fork();
    if (pid == 0) {
        char *argv[4];
        signal(SIGINT, SIG_DFL);
        signal(SIGQUIT, SIG_DFL);
        argv[0] = "sh";
        argv[1] = "-c";
        argv[2] = (char *)cmd;
        argv[3] = 0;
        execve("/bin/sh", argv, environ);
        _exit(127);
    }
    if (pid < 0)
        status = -1;
    else if (waitpid(pid, &status, 0) < 0)
        status = -1;
    signal(SIGINT, oi);
    signal(SIGQUIT, oq);
    return status;
}

/* ---- temporary files ---- */

static int fill_template(char *tmpl, int (*make)(const char *))
{
    size_t n = strlen(tmpl);
    char *x;
    int tries, i;
    static const char chars[] = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    if (n < 6 || strcmp(tmpl + n - 6, "XXXXXX") != 0) {
        errno = EINVAL;
        return -1;
    }
    x = tmpl + n - 6;
    srand((unsigned)time(0) ^ ((unsigned)getpid() << 16) ^ (unsigned)rand());
    for (tries = 0; tries < 100; tries++) {
        int r;
        for (i = 0; i < 6; i++)
            x[i] = chars[rand() % 62];
        r = make(tmpl);
        if (r >= 0 || errno != EEXIST)
            return r;
    }
    return -1;
}

static int make_file(const char *p) { return open(p, O_RDWR | O_CREAT | O_EXCL, 0600); }
static int make_dir(const char *p) { return mkdir(p, 0700); }
static int make_none(const char *p)
{
    struct stat st;
    if (lstat(p, &st) == 0) {
        errno = EEXIST;
        return -1;
    }
    return 0;
}

int mkstemp(char *tmpl) { return fill_template(tmpl, make_file); }

char *mkdtemp(char *tmpl) { return fill_template(tmpl, make_dir) < 0 ? 0 : tmpl; }

char *mktemp(char *tmpl)
{
    if (fill_template(tmpl, make_none) < 0)
        tmpl[0] = 0;
    return tmpl;
}

char *realpath(const char *path, char *resolved)
{
    char buf[PATH_MAX], out[PATH_MAX];
    char *parts[128];
    int np = 0, i, hops = 0;
    char *p, *save;
    if (path[0] == '/') {
        strlcpy(buf, path, sizeof buf);
    } else {
        if (!getcwd(buf, sizeof buf))
            return 0;
        strlcat(buf, "/", sizeof buf);
        strlcat(buf, path, sizeof buf);
    }
again:
    np = 0;
    for (p = strtok_r(buf, "/", &save); p; p = strtok_r(0, "/", &save)) {
        if (strcmp(p, ".") == 0)
            continue;
        if (strcmp(p, "..") == 0) {
            if (np)
                np--;
            continue;
        }
        if (np < 128)
            parts[np++] = p;
        /* resolve symbolic links */
        {
            char cur[PATH_MAX], link[PATH_MAX];
            ssize_t n;
            cur[0] = 0;
            for (i = 0; i < np; i++) {
                strlcat(cur, "/", sizeof cur);
                strlcat(cur, parts[i], sizeof cur);
            }
            n = readlink(cur, link, sizeof link - 1);
            if (n >= 0) {
                char rest[PATH_MAX], nb[PATH_MAX];
                link[n] = 0;
                if (++hops > 40) {
                    errno = ELOOP;
                    return 0;
                }
                rest[0] = 0;
                if (save && *save) {
                    strlcpy(rest, save, sizeof rest);
                }
                if (link[0] == '/') {
                    strlcpy(nb, link, sizeof nb);
                } else {
                    nb[0] = 0;
                    for (i = 0; i < np - 1; i++) {
                        strlcat(nb, "/", sizeof nb);
                        strlcat(nb, parts[i], sizeof nb);
                    }
                    strlcat(nb, "/", sizeof nb);
                    strlcat(nb, link, sizeof nb);
                }
                if (rest[0]) {
                    strlcat(nb, "/", sizeof nb);
                    strlcat(nb, rest, sizeof nb);
                }
                strlcpy(buf, nb, sizeof buf);
                goto again;
            }
        }
    }
    out[0] = 0;
    for (i = 0; i < np; i++) {
        strlcat(out, "/", sizeof out);
        strlcat(out, parts[i], sizeof out);
    }
    if (!out[0])
        strcpy(out, "/");
    {
        struct stat st;
        if (stat(out, &st) < 0)
            return 0;
    }
    if (!resolved) {
        resolved = malloc(strlen(out) + 1);
        if (!resolved)
            return 0;
    }
    strcpy(resolved, out);
    return resolved;
}

/* multibyte: UTF-8 is passed through as bytes */
int mblen(const char *s, size_t n) { return s && n && *s ? 1 : 0; }

int mbtowc(wchar_t *w, const char *s, size_t n)
{
    if (!s || !n)
        return 0;
    if (w)
        *w = (unsigned char)*s;
    return *s ? 1 : 0;
}

int wctomb(char *s, wchar_t w)
{
    if (!s)
        return 0;
    *s = (char)w;
    return 1;
}

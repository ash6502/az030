/* <string.h> and <strings.h> */

#include <ctype.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

void *memcpy(void *d, const void *s, size_t n)
{
    char *dp = d;
    const char *sp = s;
    /* word copies when both are aligned */
    if ((((unsigned)dp | (unsigned)sp) & 3) == 0) {
        long *dl = (long *)dp;
        const long *sl = (const long *)sp;
        while (n >= 16) {
            dl[0] = sl[0];
            dl[1] = sl[1];
            dl[2] = sl[2];
            dl[3] = sl[3];
            dl += 4;
            sl += 4;
            n -= 16;
        }
        while (n >= 4) {
            *dl++ = *sl++;
            n -= 4;
        }
        dp = (char *)dl;
        sp = (const char *)sl;
    }
    while (n--)
        *dp++ = *sp++;
    return d;
}

void *memmove(void *d, const void *s, size_t n)
{
    char *dp = d;
    const char *sp = s;
    if (dp == sp || n == 0)
        return d;
    if (dp < sp || dp >= sp + n)
        return memcpy(d, s, n);
    dp += n;
    sp += n;
    while (n--)
        *--dp = *--sp;
    return d;
}

void *memset(void *d, int c, size_t n)
{
    unsigned char *p = d;
    unsigned char b = (unsigned char)c;
    while (n && ((unsigned)p & 3)) {
        *p++ = b;
        n--;
    }
    if (n >= 4) {
        unsigned long w = b * 0x01010101UL;
        unsigned long *wp = (unsigned long *)p;
        while (n >= 4) {
            *wp++ = w;
            n -= 4;
        }
        p = (unsigned char *)wp;
    }
    while (n--)
        *p++ = b;
    return d;
}

int memcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *x = a, *y = b;
    for (; n; n--, x++, y++)
        if (*x != *y)
            return *x - *y;
    return 0;
}

void *memchr(const void *s, int c, size_t n)
{
    const unsigned char *p = s;
    for (; n; n--, p++)
        if (*p == (unsigned char)c)
            return (void *)p;
    return 0;
}

void *memrchr(const void *s, int c, size_t n)
{
    const unsigned char *p = (const unsigned char *)s + n;
    while (n--)
        if (*--p == (unsigned char)c)
            return (void *)p;
    return 0;
}

void *memmem(const void *h, size_t hn, const void *nd, size_t nn)
{
    const char *hp = h;
    size_t i;
    if (nn == 0)
        return (void *)h;
    for (i = 0; i + nn <= hn; i++)
        if (hp[i] == *(const char *)nd && memcmp(hp + i, nd, nn) == 0)
            return (void *)(hp + i);
    return 0;
}

size_t strlen(const char *s)
{
    const char *p = s;
    while (*p)
        p++;
    return p - s;
}

size_t strnlen(const char *s, size_t n)
{
    size_t i = 0;
    while (i < n && s[i])
        i++;
    return i;
}

char *strcpy(char *d, const char *s)
{
    char *r = d;
    while ((*d++ = *s++) != 0)
        ;
    return r;
}

char *stpcpy(char *d, const char *s)
{
    while ((*d = *s++) != 0)
        d++;
    return d;
}

char *strncpy(char *d, const char *s, size_t n)
{
    char *r = d;
    while (n && *s) {
        *d++ = *s++;
        n--;
    }
    while (n--)
        *d++ = 0;
    return r;
}

size_t strlcpy(char *d, const char *s, size_t n)
{
    size_t len = strlen(s);
    if (n) {
        size_t c = len < n - 1 ? len : n - 1;
        memcpy(d, s, c);
        d[c] = 0;
    }
    return len;
}

size_t strlcat(char *d, const char *s, size_t n)
{
    size_t dl = strnlen(d, n);
    if (dl == n)
        return n + strlen(s);
    return dl + strlcpy(d + dl, s, n - dl);
}

char *strcat(char *d, const char *s)
{
    strcpy(d + strlen(d), s);
    return d;
}

char *strncat(char *d, const char *s, size_t n)
{
    char *p = d + strlen(d);
    while (n-- && *s)
        *p++ = *s++;
    *p = 0;
    return d;
}

int strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) {
        if (*a != *b)
            return (unsigned char)*a - (unsigned char)*b;
        if (!*a)
            return 0;
    }
    return 0;
}

int strcoll(const char *a, const char *b) { return strcmp(a, b); }

size_t strxfrm(char *d, const char *s, size_t n)
{
    size_t len = strlen(s);
    if (n)
        strlcpy(d, s, n);
    return len;
}

int strcasecmp(const char *a, const char *b)
{
    while (*a && tolower((unsigned char)*a) == tolower((unsigned char)*b)) {
        a++;
        b++;
    }
    return tolower((unsigned char)*a) - tolower((unsigned char)*b);
}

int strncasecmp(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) {
        int x = tolower((unsigned char)*a), y = tolower((unsigned char)*b);
        if (x != y)
            return x - y;
        if (!x)
            return 0;
    }
    return 0;
}

char *strchr(const char *s, int c)
{
    for (;; s++) {
        if (*s == (char)c)
            return (char *)s;
        if (!*s)
            return 0;
    }
}

char *strrchr(const char *s, int c)
{
    const char *r = 0;
    for (;; s++) {
        if (*s == (char)c)
            r = s;
        if (!*s)
            return (char *)r;
    }
}

char *index(const char *s, int c) { return strchr(s, c); }
char *rindex(const char *s, int c) { return strrchr(s, c); }

char *strstr(const char *h, const char *n)
{
    size_t nl = strlen(n);
    if (!nl)
        return (char *)h;
    for (; *h; h++)
        if (*h == *n && strncmp(h, n, nl) == 0)
            return (char *)h;
    return 0;
}

size_t strspn(const char *s, const char *accept)
{
    const char *p = s;
    while (*p && strchr(accept, *p))
        p++;
    return p - s;
}

size_t strcspn(const char *s, const char *reject)
{
    const char *p = s;
    while (*p && !strchr(reject, *p))
        p++;
    return p - s;
}

char *strpbrk(const char *s, const char *accept)
{
    s += strcspn(s, accept);
    return *s ? (char *)s : 0;
}

char *strtok_r(char *s, const char *delim, char **save)
{
    char *end;
    if (!s)
        s = *save;
    s += strspn(s, delim);
    if (!*s) {
        *save = s;
        return 0;
    }
    end = s + strcspn(s, delim);
    if (*end)
        *end++ = 0;
    *save = end;
    return s;
}

char *strtok(char *s, const char *delim)
{
    static char *save;
    return strtok_r(s, delim, &save);
}

char *strsep(char **sp, const char *delim)
{
    char *s = *sp, *e;
    if (!s)
        return 0;
    e = s + strcspn(s, delim);
    if (*e)
        *e++ = 0;
    else
        e = 0;
    *sp = e;
    return s;
}

char *strdup(const char *s)
{
    size_t n = strlen(s) + 1;
    char *d = malloc(n);
    if (d)
        memcpy(d, s, n);
    return d;
}

char *strndup(const char *s, size_t n)
{
    size_t l = strnlen(s, n);
    char *d = malloc(l + 1);
    if (d) {
        memcpy(d, s, l);
        d[l] = 0;
    }
    return d;
}

static const char *const errors[] = {
    "Success", "Operation not permitted", "No such file or directory", "No such process",
    "Interrupted system call", "Input/output error", "No such device or address", "Argument list too long",
    "Exec format error", "Bad file descriptor", "No child processes", "Resource temporarily unavailable",
    "Out of memory", "Permission denied", "Bad address", "Block device required",
    "Device or resource busy", "File exists", "Invalid cross-device link", "No such device",
    "Not a directory", "Is a directory", "Invalid argument", "Too many open files in system",
    "Too many open files", "Inappropriate ioctl for device", "Text file busy", "File too large",
    "No space left on device", "Illegal seek", "Read-only file system", "Too many links",
    "Broken pipe", "Numerical argument out of domain", "Numerical result out of range", "Unknown error 35",
    "File name too long", "Unknown error 37", "Function not implemented", "Directory not empty",
    "Too many levels of symbolic links",
};

char *strerror(int e)
{
    static char buf[32];
    if (e >= 0 && e < (int)(sizeof errors / sizeof errors[0]))
        return (char *)errors[e];
    if (e == EILSEQ)
        return "Invalid or incomplete multibyte or wide character";
    if (e == ENOTSUP)
        return "Operation not supported";
    if (e == EOVERFLOW)
        return "Value too large for defined data type";
    strcpy(buf, "Unknown error ");
    {
        char tmp[12];
        int i = 0, v = e < 0 ? -e : e;
        do
            tmp[i++] = '0' + v % 10;
        while ((v /= 10) != 0);
        if (e < 0)
            tmp[i++] = '-';
        {
            char *p = buf + strlen(buf);
            while (i)
                *p++ = tmp[--i];
            *p = 0;
        }
    }
    return buf;
}

static const char *const signames[] = {
    "Unknown signal 0", "Hangup", "Interrupt", "Quit", "Illegal instruction", "Trace/breakpoint trap",
    "Aborted", "Bus error", "Floating point exception", "Killed", "User defined signal 1",
    "Segmentation fault", "User defined signal 2", "Broken pipe", "Alarm clock", "Terminated",
    "Unknown signal 16", "Child exited", "Continued", "Stopped (signal)", "Stopped",
    "Stopped (tty input)", "Stopped (tty output)",
};

char *strsignal(int sig)
{
    if (sig >= 0 && sig < (int)(sizeof signames / sizeof signames[0]))
        return (char *)signames[sig];
    if (sig == 28)
        return "Window changed";
    return "Unknown signal";
}

void bzero(void *s, size_t n) { memset(s, 0, n); }
void bcopy(const void *s, void *d, size_t n) { memmove(d, s, n); }
int bcmp(const void *a, const void *b, size_t n) { return memcmp(a, b, n); }

int ffs(int i)
{
    int n = 1;
    if (!i)
        return 0;
    while (!(i & 1)) {
        i = (unsigned)i >> 1;
        n++;
    }
    return n;
}

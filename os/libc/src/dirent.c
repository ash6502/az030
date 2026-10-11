/* <dirent.h> */

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

struct __dir {
    int fd;
    int pos, len;
    char buf[2048];
    struct dirent ent;
};

DIR *fdopendir(int fd)
{
    DIR *d = malloc(sizeof(DIR));
    if (!d)
        return 0;
    d->fd = fd;
    d->pos = d->len = 0;
    return d;
}

DIR *opendir(const char *path)
{
    int fd = open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    DIR *d;
    if (fd < 0)
        return 0;
    d = fdopendir(fd);
    if (!d)
        close(fd);
    return d;
}

struct dirent *readdir(DIR *d)
{
    unsigned char *r;
    if (d->pos >= d->len) {
        long n = __syscall(SYS_getdents, d->fd, (long)d->buf, sizeof d->buf, 0, 0);
        if (n <= 0) {
            if (n < 0)
                errno = (int)-n;
            return 0;
        }
        d->len = (int)n;
        d->pos = 0;
    }
    r = (unsigned char *)d->buf + d->pos;
    d->ent.d_ino = ((unsigned)r[0] << 24) | ((unsigned)r[1] << 16) | ((unsigned)r[2] << 8) | r[3];
    d->ent.d_reclen = (unsigned short)((r[4] << 8) | r[5]);
    d->ent.d_type = r[6];
    d->ent.d_namlen = r[7];
    memcpy(d->ent.d_name, r + 8, r[7]);
    d->ent.d_name[r[7]] = 0;
    d->pos += d->ent.d_reclen ? d->ent.d_reclen : d->len;
    return &d->ent;
}

int closedir(DIR *d)
{
    int r = close(d->fd);
    free(d);
    return r;
}

void rewinddir(DIR *d)
{
    lseek(d->fd, 0, SEEK_SET);
    d->pos = d->len = 0;
}

int dirfd(DIR *d) { return d->fd; }

int alphasort(const struct dirent **a, const struct dirent **b) { return strcmp((*a)->d_name, (*b)->d_name); }

int scandir(const char *path, struct dirent ***list, int (*filter)(const struct dirent *),
            int (*cmp)(const struct dirent **, const struct dirent **))
{
    DIR *d = opendir(path);
    struct dirent *e, **v = 0;
    int n = 0, cap = 0;
    if (!d)
        return -1;
    while ((e = readdir(d)) != 0) {
        struct dirent *c;
        if (filter && !filter(e))
            continue;
        if (n == cap) {
            struct dirent **nv;
            cap = cap ? cap * 2 : 16;
            nv = realloc(v, cap * sizeof *v);
            if (!nv)
                break;
            v = nv;
        }
        c = malloc(sizeof *c);
        if (!c)
            break;
        *c = *e;
        v[n++] = c;
    }
    closedir(d);
    if (cmp)
        qsort(v, n, sizeof *v, (int (*)(const void *, const void *))cmp);
    *list = v;
    return n;
}

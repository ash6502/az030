#ifndef _DIRENT_H
#define _DIRENT_H

#include <sys/types.h>

#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 4
#define DT_BLK 6
#define DT_REG 8
#define DT_LNK 10

struct dirent {
    ino_t d_ino;
    unsigned short d_reclen;
    unsigned char d_type;
    unsigned char d_namlen;
    char d_name[256];
};

typedef struct __dir DIR;

DIR *opendir(const char *path);
DIR *fdopendir(int fd);
struct dirent *readdir(DIR *d);
int closedir(DIR *d);
void rewinddir(DIR *d);
int dirfd(DIR *d);
int scandir(const char *path, struct dirent ***list, int (*filter)(const struct dirent *), int (*cmp)(const struct dirent **, const struct dirent **));
int alphasort(const struct dirent **a, const struct dirent **b);

#endif

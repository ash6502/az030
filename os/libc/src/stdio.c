/* <stdio.h>: buffered streams */

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define F_READ 1
#define F_WRITE 2
#define F_EOF 4
#define F_ERR 8
#define F_OWNBUF 16
#define F_APPEND 32

struct __file {
    int fd;
    int flags;
    int mode;          /* _IOFBF, _IOLBF, _IONBF */
    unsigned char *buf;
    int size;
    int pos;           /* read: next byte; write: bytes buffered */
    int len;           /* read: bytes in buffer */
    int unget;         /* pushed-back character or -1 */
    int pid;           /* popen child */
    struct __file *next;
};

static unsigned char inbuf[BUFSIZ], outbuf[BUFSIZ];
static unsigned char errbuf[1];

static struct __file f_stdin = {0, F_READ, _IOFBF, inbuf, BUFSIZ, 0, 0, -1, 0, 0};
static struct __file f_stdout = {1, F_WRITE, _IOFBF, outbuf, BUFSIZ, 0, 0, -1, 0, 0};
static struct __file f_stderr = {2, F_WRITE, _IONBF, errbuf, 1, 0, 0, -1, 0, 0};

FILE *stdin = &f_stdin;
FILE *stdout = &f_stdout;
FILE *stderr = &f_stderr;

static FILE *open_files;

extern void (*__stdio_exit)(void);

static void flush_all(void)
{
    FILE *f;
    fflush(stdout);
    fflush(stderr);
    for (f = open_files; f; f = f->next)
        fflush(f);
}

void __libc_init(int argc, char **argv, char **envp)
{
    (void)argc;
    (void)argv;
    (void)envp;
    __stdio_exit = flush_all;
    if (isatty(1))
        stdout->mode = _IOLBF;
}

static int parse_mode(const char *mode, int *flags)
{
    int o, plus = strchr(mode, '+') != 0;
    switch (mode[0]) {
    case 'r':
        o = plus ? O_RDWR : O_RDONLY;
        *flags = plus ? F_READ | F_WRITE : F_READ;
        break;
    case 'w':
        o = (plus ? O_RDWR : O_WRONLY) | O_CREAT | O_TRUNC;
        *flags = plus ? F_READ | F_WRITE : F_WRITE;
        break;
    case 'a':
        o = (plus ? O_RDWR : O_WRONLY) | O_CREAT | O_APPEND;
        *flags = (plus ? F_READ | F_WRITE : F_WRITE) | F_APPEND;
        break;
    default:
        errno = EINVAL;
        return -1;
    }
    if (strchr(mode, 'x'))
        o |= O_EXCL;
    if (strchr(mode, 'e'))
        o |= O_CLOEXEC;
    return o;
}

static FILE *new_file(int fd, int flags)
{
    FILE *f = calloc(1, sizeof(FILE));
    if (!f)
        return 0;
    f->buf = malloc(BUFSIZ);
    if (!f->buf) {
        free(f);
        return 0;
    }
    f->fd = fd;
    f->flags = flags | F_OWNBUF;
    f->size = BUFSIZ;
    f->mode = isatty(fd) ? _IOLBF : _IOFBF;
    f->unget = -1;
    f->next = open_files;
    open_files = f;
    return f;
}

FILE *fopen(const char *path, const char *mode)
{
    int flags, fd;
    int o = parse_mode(mode, &flags);
    FILE *f;
    if (o < 0)
        return 0;
    fd = open(path, o, 0666);
    if (fd < 0)
        return 0;
    f = new_file(fd, flags);
    if (!f)
        close(fd);
    return f;
}

FILE *fdopen(int fd, const char *mode)
{
    int flags;
    if (parse_mode(mode, &flags) < 0)
        return 0;
    return new_file(fd, flags);
}

FILE *freopen(const char *path, const char *mode, FILE *f)
{
    int flags, fd;
    int o = parse_mode(mode, &flags);
    if (o < 0)
        return 0;
    fflush(f);
    if (path) {
        fd = open(path, o, 0666);
        if (fd < 0)
            return 0;
        dup2(fd, f->fd);
        close(fd);
    }
    f->flags = (f->flags & F_OWNBUF) | flags;
    f->pos = f->len = 0;
    f->unget = -1;
    return f;
}

static void unlink_file(FILE *f)
{
    FILE **pp;
    for (pp = &open_files; *pp; pp = &(*pp)->next)
        if (*pp == f) {
            *pp = f->next;
            return;
        }
}

int fclose(FILE *f)
{
    int r = fflush(f);
    if (close(f->fd) < 0)
        r = EOF;
    if (f == stdin || f == stdout || f == stderr) {
        f->flags |= F_EOF;
        return r;
    }
    unlink_file(f);
    if (f->flags & F_OWNBUF)
        free(f->buf);
    free(f);
    return r;
}

int fileno(FILE *f) { return f->fd; }

int setvbuf(FILE *f, char *buf, int mode, size_t size)
{
    fflush(f);
    if (buf && size) {
        if (f->flags & F_OWNBUF)
            free(f->buf);
        f->flags &= ~F_OWNBUF;
        f->buf = (unsigned char *)buf;
        f->size = (int)size;
    }
    f->mode = mode;
    return 0;
}

void setbuf(FILE *f, char *buf) { setvbuf(f, buf, buf ? _IOFBF : _IONBF, BUFSIZ); }
void setlinebuf(FILE *f) { setvbuf(f, 0, _IOLBF, 0); }

static int write_all(int fd, const unsigned char *p, int n)
{
    while (n > 0) {
        int w = write(fd, p, n);
        if (w < 0) {
            if (errno == EINTR)
                continue;
            return -1;
        }
        p += w;
        n -= w;
    }
    return 0;
}

int fflush(FILE *f)
{
    if (!f) {
        flush_all();
        return 0;
    }
    if ((f->flags & F_WRITE) && f->pos > 0 && f->len == 0) {
        int r = write_all(f->fd, f->buf, f->pos);
        f->pos = 0;
        if (r < 0) {
            f->flags |= F_ERR;
            return EOF;
        }
    }
    if (f->len > 0) {
        /* reading: discard the buffer and move the file position back */
        int back = f->len - f->pos;
        if (back > 0)
            lseek(f->fd, -back, SEEK_CUR);
        f->pos = f->len = 0;
    }
    f->unget = -1;
    return 0;
}

static int fill(FILE *f)
{
    int n;
    if (f == stdin)
        fflush(stdout);
    if (f->flags & F_EOF)
        return 0;
    do
        n = read(f->fd, f->buf, f->mode == _IONBF ? 1 : f->size);
    while (n < 0 && errno == EINTR);
    if (n <= 0) {
        f->flags |= n == 0 ? F_EOF : F_ERR;
        return 0;
    }
    f->pos = 0;
    f->len = n;
    return n;
}

int fgetc(FILE *f)
{
    if (f->unget >= 0) {
        int c = f->unget;
        f->unget = -1;
        return c;
    }
    if (f->pos >= f->len) {
        if (f->len == 0 && f->pos > 0 && (f->flags & F_WRITE))
            fflush(f);
        f->pos = f->len = 0;
        if (!fill(f))
            return EOF;
    }
    return f->buf[f->pos++];
}

int getc(FILE *f) { return fgetc(f); }
int getchar(void) { return fgetc(stdin); }

int ungetc(int c, FILE *f)
{
    if (c == EOF)
        return EOF;
    f->unget = (unsigned char)c;
    f->flags &= ~F_EOF;
    return (unsigned char)c;
}

int fputc(int c, FILE *f)
{
    unsigned char ch = (unsigned char)c;
    if (f->len > 0)
        fflush(f);
    if (f->mode == _IONBF) {
        if (write_all(f->fd, &ch, 1) < 0) {
            f->flags |= F_ERR;
            return EOF;
        }
        return ch;
    }
    f->buf[f->pos++] = ch;
    if (f->pos >= f->size || (f->mode == _IOLBF && ch == '\n'))
        if (fflush(f) == EOF)
            return EOF;
    return ch;
}

int putc(int c, FILE *f) { return fputc(c, f); }
int putchar(int c) { return fputc(c, stdout); }

size_t fwrite(const void *p, size_t size, size_t n, FILE *f)
{
    const unsigned char *s = p;
    size_t total = size * n, i;
    if (!total)
        return 0;
    if (f->len > 0)
        fflush(f);
    if (f->mode == _IONBF || total >= (size_t)f->size) {
        fflush(f);
        if (write_all(f->fd, s, (int)total) < 0) {
            f->flags |= F_ERR;
            return 0;
        }
        return n;
    }
    for (i = 0; i < total; i++)
        if (fputc(s[i], f) == EOF)
            return i / size;
    return n;
}

size_t fread(void *p, size_t size, size_t n, FILE *f)
{
    unsigned char *d = p;
    size_t total = size * n, got = 0;
    if (!total)
        return 0;
    if (f->unget >= 0) {
        d[got++] = (unsigned char)f->unget;
        f->unget = -1;
    }
    while (got < total) {
        int avail = f->len - f->pos;
        if (avail <= 0) {
            /* large reads go straight to the descriptor */
            if (total - got >= (size_t)f->size) {
                int r;
                if (f == stdin)
                    fflush(stdout);
                r = read(f->fd, d + got, total - got);
                if (r <= 0) {
                    f->flags |= r == 0 ? F_EOF : F_ERR;
                    break;
                }
                got += r;
                continue;
            }
            f->pos = f->len = 0;
            if (!fill(f))
                break;
            avail = f->len;
        }
        if ((size_t)avail > total - got)
            avail = (int)(total - got);
        memcpy(d + got, f->buf + f->pos, avail);
        f->pos += avail;
        got += avail;
    }
    return got / size;
}

int fputs(const char *s, FILE *f)
{
    size_t n = strlen(s);
    return fwrite(s, 1, n, f) == n ? 0 : EOF;
}

int puts(const char *s)
{
    if (fputs(s, stdout) == EOF)
        return EOF;
    return fputc('\n', stdout) == EOF ? EOF : 0;
}

char *fgets(char *s, int n, FILE *f)
{
    int i = 0, c = 0;
    if (n <= 0)
        return 0;
    while (i < n - 1 && (c = fgetc(f)) != EOF) {
        s[i++] = (char)c;
        if (c == '\n')
            break;
    }
    if (i == 0 && c == EOF)
        return 0;
    s[i] = 0;
    return s;
}

char *gets(char *s)
{
    int i = 0, c;
    while ((c = fgetc(stdin)) != EOF && c != '\n')
        s[i++] = (char)c;
    if (i == 0 && c == EOF)
        return 0;
    s[i] = 0;
    return s;
}

ssize_t getdelim(char **line, size_t *n, int delim, FILE *f)
{
    size_t i = 0;
    int c;
    if (!*line || *n == 0) {
        *n = 128;
        *line = malloc(*n);
        if (!*line)
            return -1;
    }
    while ((c = fgetc(f)) != EOF) {
        if (i + 2 > *n) {
            char *nl = realloc(*line, *n * 2);
            if (!nl)
                return -1;
            *line = nl;
            *n *= 2;
        }
        (*line)[i++] = (char)c;
        if (c == delim)
            break;
    }
    if (i == 0 && c == EOF)
        return -1;
    (*line)[i] = 0;
    return (ssize_t)i;
}

ssize_t getline(char **line, size_t *n, FILE *f) { return getdelim(line, n, '\n', f); }

int fseek(FILE *f, long off, int whence)
{
    fflush(f);
    if (whence == SEEK_CUR && f->len > 0)
        off -= f->len - f->pos;
    f->pos = f->len = 0;
    f->unget = -1;
    if (lseek(f->fd, off, whence) < 0)
        return -1;
    f->flags &= ~F_EOF;
    return 0;
}

long ftell(FILE *f)
{
    long p = lseek(f->fd, 0, SEEK_CUR);
    if (p < 0)
        return -1;
    if (f->len > 0)
        return p - (f->len - f->pos) - (f->unget >= 0);
    return p + f->pos;
}

void rewind(FILE *f)
{
    fseek(f, 0, SEEK_SET);
    f->flags &= ~F_ERR;
}

int fgetpos(FILE *f, fpos_t *p)
{
    long r = ftell(f);
    if (r < 0)
        return -1;
    *p = r;
    return 0;
}

int fsetpos(FILE *f, const fpos_t *p) { return fseek(f, *p, SEEK_SET); }
int feof(FILE *f) { return (f->flags & F_EOF) != 0; }
int ferror(FILE *f) { return (f->flags & F_ERR) != 0; }
void clearerr(FILE *f) { f->flags &= ~(F_EOF | F_ERR); }

void perror(const char *s)
{
    int e = errno;
    if (s && *s) {
        fputs(s, stderr);
        fputs(": ", stderr);
    }
    fputs(strerror(e), stderr);
    fputc('\n', stderr);
}

int remove(const char *path)
{
    if (unlink(path) == 0)
        return 0;
    if (errno == EISDIR || errno == EPERM)
        return rmdir(path);
    return -1;
}

FILE *tmpfile(void)
{
    char name[] = "/tmp/tmpfileXXXXXX";
    int fd = mkstemp(name);
    FILE *f;
    if (fd < 0)
        return 0;
    unlink(name);
    f = fdopen(fd, "w+");
    if (!f)
        close(fd);
    return f;
}

char *tmpnam(char *s)
{
    static char buf[L_tmpnam];
    static int n;
    char *p = s ? s : buf;
    do {
        sprintf(p, "/tmp/t%d.%d", getpid(), n++);
    } while (access(p, F_OK) == 0);
    return p;
}

FILE *popen(const char *cmd, const char *mode)
{
    int fds[2], child_end, parent_end;
    int reading = mode[0] == 'r';
    pid_t pid;
    FILE *f;
    if (pipe(fds) < 0)
        return 0;
    parent_end = reading ? fds[0] : fds[1];
    child_end = reading ? fds[1] : fds[0];
    pid = fork();
    if (pid < 0) {
        close(fds[0]);
        close(fds[1]);
        return 0;
    }
    if (pid == 0) {
        char *argv[4];
        close(parent_end);
        dup2(child_end, reading ? 1 : 0);
        if (child_end != (reading ? 1 : 0))
            close(child_end);
        argv[0] = "sh";
        argv[1] = "-c";
        argv[2] = (char *)cmd;
        argv[3] = 0;
        execve("/bin/sh", argv, environ);
        _exit(127);
    }
    close(child_end);
    f = fdopen(parent_end, reading ? "r" : "w");
    if (!f) {
        close(parent_end);
        return 0;
    }
    f->pid = pid;
    return f;
}

int pclose(FILE *f)
{
    int pid = f->pid, status;
    fclose(f);
    if (waitpid(pid, &status, 0) < 0)
        return -1;
    return status;
}

/* System call wrappers. */

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/times.h>
#include <sys/utsname.h>
#include <sys/wait.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

int errno;
char **environ;

/* Return r, setting errno for kernel error values. */
static long ret(long r)
{
    if (r < 0 && r > -4096) {
        errno = (int)-r;
        return -1;
    }
    return r;
}

#define SC0(n) ret(__syscall(n, 0, 0, 0, 0, 0))
#define SC1(n, a) ret(__syscall(n, (long)(a), 0, 0, 0, 0))
#define SC2(n, a, b) ret(__syscall(n, (long)(a), (long)(b), 0, 0, 0))
#define SC3(n, a, b, c) ret(__syscall(n, (long)(a), (long)(b), (long)(c), 0, 0))

long syscall(long nr, ...)
{
    va_list ap;
    long a[5];
    int i;
    va_start(ap, nr);
    for (i = 0; i < 5; i++)
        a[i] = va_arg(ap, long);
    va_end(ap);
    return ret(__syscall(nr, a[0], a[1], a[2], a[3], a[4]));
}

ssize_t read(int fd, void *buf, size_t n) { return SC3(SYS_read, fd, buf, n); }
ssize_t write(int fd, const void *buf, size_t n) { return SC3(SYS_write, fd, buf, n); }
int close(int fd) { return SC1(SYS_close, fd); }
off_t lseek(int fd, off_t off, int whence) { return SC3(SYS_lseek, fd, off, whence); }
int dup(int fd) { return SC1(SYS_dup, fd); }
int dup2(int old, int new_fd) { return SC2(SYS_dup2, old, new_fd); }
int pipe(int fds[2]) { return SC1(SYS_pipe, fds); }
int fsync(int fd) { return SC1(SYS_fsync, fd); }
int ftruncate(int fd, off_t len) { return SC2(SYS_ftruncate, fd, len); }
void sync(void) { __syscall(SYS_sync, 0, 0, 0, 0, 0); }

int open(const char *path, int flags, ...)
{
    va_list ap;
    int mode = 0;
    if (flags & O_CREAT) {
        va_start(ap, flags);
        mode = va_arg(ap, int);
        va_end(ap);
    }
    return SC3(SYS_open, path, flags, mode);
}

int creat(const char *path, mode_t mode) { return open(path, O_WRONLY | O_CREAT | O_TRUNC, mode); }

int truncate(const char *path, off_t len)
{
    int fd = open(path, O_WRONLY);
    int r;
    if (fd < 0)
        return -1;
    r = ftruncate(fd, len);
    close(fd);
    return r;
}

int fcntl(int fd, int cmd, ...)
{
    va_list ap;
    long arg;
    va_start(ap, cmd);
    arg = va_arg(ap, long);
    va_end(ap);
    return SC3(SYS_fcntl, fd, cmd, arg);
}

int ioctl(int fd, unsigned long req, ...)
{
    va_list ap;
    long arg;
    va_start(ap, req);
    arg = va_arg(ap, long);
    va_end(ap);
    return SC3(SYS_ioctl, fd, req, arg);
}

int isatty(int fd)
{
    struct termios t;
    return tcgetattr(fd, &t) == 0;
}

char *ttyname(int fd) { return isatty(fd) ? "/dev/console" : (char *)0; }

pid_t fork(void) { return SC0(SYS_fork); }
pid_t vfork(void) { return fork(); }
int execve(const char *path, char *const argv[], char *const envp[]) { return SC3(SYS_execve, path, argv, envp); }

void _exit(int status)
{
    for (;;)
        __syscall(SYS_exit, status, 0, 0, 0, 0);
}

void _Exit(int status) { _exit(status); }

pid_t waitpid(pid_t pid, int *status, int options)
{
    long r;
    do
        r = __syscall(SYS_waitpid, pid, (long)status, options, 0, 0);
    while (r == -EINTR);
    return ret(r);
}

pid_t wait(int *status) { return waitpid(-1, status, 0); }

pid_t getpid(void) { return SC0(SYS_getpid); }
pid_t getppid(void) { return SC0(SYS_getppid); }
pid_t getpgid(pid_t pid) { return SC1(SYS_getpgid, pid); }
pid_t getpgrp(void) { return getpgid(0); }
int setpgid(pid_t pid, pid_t pgid) { return SC2(SYS_setpgid, pid, pgid); }
pid_t setsid(void) { return SC0(SYS_setsid); }
uid_t getuid(void) { return SC0(SYS_getuid); }
uid_t geteuid(void) { return SC0(SYS_geteuid); }
gid_t getgid(void) { return SC0(SYS_getgid); }
gid_t getegid(void) { return SC0(SYS_getegid); }
int setuid(uid_t uid) { return SC1(SYS_setuid, uid); }
int setgid(gid_t gid) { return SC1(SYS_setgid, gid); }

int getgroups(int n, gid_t *list)
{
    if (n > 0)
        list[0] = getegid();
    return 1;
}

pid_t tcgetpgrp(int fd)
{
    pid_t p;
    if (ioctl(fd, TIOCGPGRP, &p) < 0)
        return -1;
    return p;
}

int tcsetpgrp(int fd, pid_t pgrp) { return ioctl(fd, TIOCSPGRP, &pgrp); }

int chdir(const char *path) { return SC1(SYS_chdir, path); }

char *getcwd(char *buf, size_t size)
{
    if (SC2(SYS_getcwd, buf, size) < 0)
        return (char *)0;
    return buf;
}

int unlink(const char *path) { return SC1(SYS_unlink, path); }
int rmdir(const char *path) { return SC1(SYS_rmdir, path); }
int mkdir(const char *path, mode_t mode) { return SC2(SYS_mkdir, path, mode); }
int link(const char *from, const char *to) { return SC2(SYS_link, from, to); }
int symlink(const char *target, const char *path) { return SC2(SYS_symlink, target, path); }
ssize_t readlink(const char *path, char *buf, size_t size) { return SC3(SYS_readlink, path, buf, size); }
int access(const char *path, int mode) { return SC2(SYS_access, path, mode); }
int chmod(const char *path, mode_t mode) { return SC2(SYS_chmod, path, mode); }
int chown(const char *path, uid_t uid, gid_t gid) { return SC3(SYS_chown, path, uid, gid); }
int lchown(const char *path, uid_t uid, gid_t gid) { return chown(path, uid, gid); }
int rename(const char *from, const char *to) { return SC2(SYS_rename, from, to); }
int mknod(const char *path, mode_t mode, dev_t dev) { return SC3(SYS_mknod, path, mode, dev); }
int mkfifo(const char *path, mode_t mode) { return mknod(path, S_IFIFO | (mode & 0777), 0); }
mode_t umask(mode_t mask) { return SC1(SYS_umask, mask); }
int stat(const char *path, struct stat *st) { return SC2(SYS_stat, path, st); }
int lstat(const char *path, struct stat *st) { return SC2(SYS_lstat, path, st); }
int fstat(int fd, struct stat *st) { return SC2(SYS_fstat, fd, st); }

int fchmod(int fd, mode_t mode)
{
    (void)fd;
    (void)mode;
    errno = ENOSYS;
    return -1;
}

int kill(pid_t pid, int sig) { return SC2(SYS_kill, pid, sig); }
int killpg(pid_t pgrp, int sig) { return kill(-pgrp, sig); }
int raise(int sig) { return kill(getpid(), sig); }
int pause(void) { return SC0(SYS_pause); }
unsigned alarm(unsigned secs) { return (unsigned)__syscall(SYS_alarm, secs, 0, 0, 0, 0); }

/* the heap */
static char *cur_brk;

int brk(void *addr)
{
    long r = __syscall(SYS_brk, (long)addr, 0, 0, 0, 0);
    if ((unsigned long)r < (unsigned long)addr) {
        errno = ENOMEM;
        return -1;
    }
    cur_brk = (char *)r;
    return 0;
}

void *sbrk(int incr)
{
    char *old;
    if (!cur_brk)
        cur_brk = (char *)__syscall(SYS_brk, 0, 0, 0, 0, 0);
    old = cur_brk;
    if (incr == 0)
        return old;
    if (brk(old + incr) < 0)
        return (void *)-1;
    return old;
}

int getpagesize(void) { return 4096; }

long sysconf(int name)
{
    switch (name) {
    case _SC_PAGESIZE:
        return 4096;
    case _SC_CLK_TCK:
        return 100;
    case _SC_OPEN_MAX:
        return 32;
    case _SC_ARG_MAX:
        return 131072;
    }
    errno = EINVAL;
    return -1;
}

/* signals */
extern void __sigreturn(void);

int sigaction(int sig, const struct sigaction *act, struct sigaction *old)
{
    struct sigaction a;
    if (act) {
        a = *act;
        a.sa_restorer = __sigreturn;
        act = &a;
    }
    return SC3(SYS_sigaction, sig, act, old);
}

sighandler_t signal(int sig, sighandler_t h)
{
    struct sigaction a, old;
    a.sa_handler = h;
    a.sa_mask = 0;
    a.sa_flags = SA_RESTART;
    if (sigaction(sig, &a, &old) < 0)
        return SIG_ERR;
    return old.sa_handler;
}

int sigprocmask(int how, const sigset_t *set, sigset_t *old) { return SC3(SYS_sigprocmask, how, set, old); }
int sigemptyset(sigset_t *s) { *s = 0; return 0; }
int sigfillset(sigset_t *s) { *s = ~0U; return 0; }
int sigaddset(sigset_t *s, int sig) { *s |= 1U << sig; return 0; }
int sigdelset(sigset_t *s, int sig) { *s &= ~(1U << sig); return 0; }
int sigismember(const sigset_t *s, int sig) { return (*s >> sig) & 1; }

int sigsuspend(const sigset_t *mask)
{
    sigset_t old;
    sigprocmask(SIG_SETMASK, mask, &old);
    pause();
    sigprocmask(SIG_SETMASK, &old, (sigset_t *)0);
    errno = EINTR;
    return -1;
}

/* time */
time_t time(time_t *t)
{
    time_t now = (time_t)__syscall(SYS_time, 0, 0, 0, 0, 0);
    if (t)
        *t = now;
    return now;
}

int gettimeofday(struct timeval *tv, void *tz)
{
    (void)tz;
    return SC1(SYS_gettimeofday, tv);
}

int settimeofday(const struct timeval *tv, const void *tz)
{
    (void)tz;
    return SC1(SYS_settimeofday, tv->tv_sec);
}

clock_t times(struct tms *t) { return (clock_t)__syscall(SYS_times, (long)t, 0, 0, 0, 0); }

clock_t clock(void)
{
    struct tms t;
    times(&t);
    return t.tms_utime + t.tms_stime;
}

int clock_gettime(clockid_t id, struct timespec *ts)
{
    struct timeval tv;
    (void)id;
    gettimeofday(&tv, 0);
    ts->tv_sec = tv.tv_sec;
    ts->tv_nsec = tv.tv_usec * 1000;
    return 0;
}

static int sleep_ms(unsigned long ms) { return SC1(SYS_sleep_ms, ms); }

int nanosleep(const struct timespec *req, struct timespec *rem)
{
    unsigned long ms = req->tv_sec * 1000 + (req->tv_nsec + 999999) / 1000000;
    if (rem) {
        rem->tv_sec = 0;
        rem->tv_nsec = 0;
    }
    return sleep_ms(ms);
}

unsigned sleep(unsigned secs) { return sleep_ms(secs * 1000UL) < 0 ? secs : 0; }
int usleep(useconds_t us) { return sleep_ms((us + 999) / 1000); }

int uname(struct utsname *u) { return SC1(SYS_uname, u); }

int gethostname(char *name, size_t len)
{
    int fd = open("/etc/hostname", O_RDONLY);
    int n = 0;
    if (fd >= 0) {
        n = read(fd, name, len - 1);
        close(fd);
    }
    if (n <= 0) {
        struct utsname u;
        int i;
        uname(&u);
        for (i = 0; u.nodename[i] && i < (int)len - 1; i++)
            name[i] = u.nodename[i];
        n = i;
    }
    while (n > 0 && (name[n - 1] == '\n' || name[n - 1] == ' '))
        n--;
    name[n] = 0;
    return 0;
}

/* termios */
int tcgetattr(int fd, struct termios *t) { return ioctl(fd, TCGETS, t); }

int tcsetattr(int fd, int when, const struct termios *t)
{
    unsigned long req = when == TCSANOW ? TCSETS : when == TCSADRAIN ? TCSETSW : TCSETSF;
    return ioctl(fd, req, t);
}

void cfmakeraw(struct termios *t)
{
    t->c_iflag &= ~(ICRNL | INLCR | IGNCR | IXON | ISTRIP);
    t->c_oflag &= ~OPOST;
    t->c_lflag &= ~(ECHO | ECHONL | ICANON | ISIG | IEXTEN);
    t->c_cc[VMIN] = 1;
    t->c_cc[VTIME] = 0;
}

speed_t cfgetospeed(const struct termios *t) { (void)t; return B115200; }
speed_t cfgetispeed(const struct termios *t) { (void)t; return B115200; }
int cfsetospeed(struct termios *t, speed_t s) { (void)t; (void)s; return 0; }
int cfsetispeed(struct termios *t, speed_t s) { (void)t; (void)s; return 0; }
int tcdrain(int fd) { (void)fd; return 0; }

int tcflush(int fd, int which)
{
    struct termios t;
    (void)which;
    if (tcgetattr(fd, &t) < 0)
        return -1;
    return ioctl(fd, TCSETSF, &t);
}

#ifndef _UNISTD_H
#define _UNISTD_H

#include <sys/types.h>

#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2

#define R_OK 4
#define W_OK 2
#define X_OK 1
#define F_OK 0

#ifndef SEEK_SET
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#endif

ssize_t read(int fd, void *buf, size_t n);
ssize_t write(int fd, const void *buf, size_t n);
int close(int fd);
off_t lseek(int fd, off_t off, int whence);
int dup(int fd);
int dup2(int old, int new_fd);
int pipe(int fds[2]);
int fsync(int fd);
int ftruncate(int fd, off_t len);
int truncate(const char *path, off_t len);
int isatty(int fd);
char *ttyname(int fd);

pid_t fork(void);
pid_t vfork(void);
int execve(const char *path, char *const argv[], char *const envp[]);
int execv(const char *path, char *const argv[]);
int execvp(const char *file, char *const argv[]);
int execvpe(const char *file, char *const argv[], char *const envp[]);
int execl(const char *path, const char *arg, ...);
int execlp(const char *file, const char *arg, ...);
int execle(const char *path, const char *arg, ...);
void _exit(int status);

pid_t getpid(void);
pid_t getppid(void);
pid_t getpgrp(void);
pid_t getpgid(pid_t pid);
int setpgid(pid_t pid, pid_t pgid);
pid_t setsid(void);
uid_t getuid(void);
uid_t geteuid(void);
gid_t getgid(void);
gid_t getegid(void);
int setuid(uid_t uid);
int setgid(gid_t gid);
int getgroups(int n, gid_t *list);
pid_t tcgetpgrp(int fd);
int tcsetpgrp(int fd, pid_t pgrp);

int chdir(const char *path);
char *getcwd(char *buf, size_t size);
int unlink(const char *path);
int rmdir(const char *path);
int link(const char *from, const char *to);
int symlink(const char *target, const char *path);
ssize_t readlink(const char *path, char *buf, size_t size);
int access(const char *path, int mode);
int chown(const char *path, uid_t uid, gid_t gid);
int lchown(const char *path, uid_t uid, gid_t gid);
void sync(void);

unsigned sleep(unsigned secs);
int usleep(useconds_t usecs);
unsigned alarm(unsigned secs);
int pause(void);
void *sbrk(int incr);
int brk(void *addr);
int getpagesize(void);
long sysconf(int name);
int gethostname(char *name, size_t len);
char *getlogin(void);
char *getpass(const char *prompt);

extern char **environ;
extern char *optarg;
extern int optind, opterr, optopt;
int getopt(int argc, char *const argv[], const char *opts);

#define _SC_PAGESIZE 1
#define _SC_PAGE_SIZE 1
#define _SC_CLK_TCK 2
#define _SC_OPEN_MAX 3
#define _SC_ARG_MAX 4

#endif

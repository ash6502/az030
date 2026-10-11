/* exec family, getopt, passwd/group, libgen, assert, locale and odds and ends */

#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <libgen.h>
#include <locale.h>
#include <pwd.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>

/* ---- exec ---- */

int execv(const char *path, char *const argv[]) { return execve(path, argv, environ); }

int execvpe(const char *file, char *const argv[], char *const envp[])
{
    const char *path;
    char buf[1024];
    int saw_eacces = 0;
    if (strchr(file, '/'))
        return execve(file, argv, envp);
    path = getenv("PATH");
    if (!path)
        path = "/bin:/usr/bin";
    while (*path) {
        const char *end = strchr(path, ':');
        size_t n = end ? (size_t)(end - path) : strlen(path);
        if (n == 0) {
            strcpy(buf, file);
        } else if (n + strlen(file) + 2 <= sizeof buf) {
            memcpy(buf, path, n);
            buf[n] = '/';
            strcpy(buf + n + 1, file);
        } else {
            buf[0] = 0;
        }
        if (buf[0]) {
            execve(buf, argv, envp);
            if (errno == EACCES)
                saw_eacces = 1;
            else if (errno != ENOENT && errno != ENOTDIR)
                return -1;
        }
        if (!end)
            break;
        path = end + 1;
    }
    errno = saw_eacces ? EACCES : ENOENT;
    return -1;
}

int execvp(const char *file, char *const argv[]) { return execvpe(file, argv, environ); }

#define MAX_ARGS 256

static int collect(const char *arg, va_list ap, char **argv)
{
    int n = 0;
    argv[n++] = (char *)arg;
    while (n < MAX_ARGS - 1 && (argv[n] = va_arg(ap, char *)) != 0)
        n++;
    argv[n] = 0;
    return n;
}

int execl(const char *path, const char *arg, ...)
{
    char *argv[MAX_ARGS];
    va_list ap;
    va_start(ap, arg);
    collect(arg, ap, argv);
    va_end(ap);
    return execve(path, argv, environ);
}

int execlp(const char *file, const char *arg, ...)
{
    char *argv[MAX_ARGS];
    va_list ap;
    va_start(ap, arg);
    collect(arg, ap, argv);
    va_end(ap);
    return execvp(file, argv);
}

int execle(const char *path, const char *arg, ...)
{
    char *argv[MAX_ARGS];
    char **envp;
    va_list ap;
    va_start(ap, arg);
    collect(arg, ap, argv);
    envp = va_arg(ap, char **);
    va_end(ap);
    return execve(path, argv, envp);
}

/* ---- getopt ---- */

char *optarg;
int optind = 1, opterr = 1, optopt;
static int optpos;

int getopt(int argc, char *const argv[], const char *opts)
{
    const char *p;
    int c;
    if (optind >= argc || !argv[optind] || argv[optind][0] != '-' || !argv[optind][1]) {
        optpos = 0;
        return -1;
    }
    if (strcmp(argv[optind], "--") == 0) {
        optind++;
        optpos = 0;
        return -1;
    }
    if (optpos == 0)
        optpos = 1;
    c = (unsigned char)argv[optind][optpos++];
    p = strchr(opts, c);
    if (!p || c == ':') {
        optopt = c;
        if (opterr && opts[0] != ':')
            fprintf(stderr, "%s: invalid option -- '%c'\n", argv[0], c);
        if (!argv[optind][optpos]) {
            optind++;
            optpos = 0;
        }
        return '?';
    }
    if (p[1] == ':') {
        if (argv[optind][optpos]) {
            optarg = argv[optind] + optpos;
        } else if (p[2] == ':') {
            optarg = 0;
        } else if (optind + 1 < argc) {
            optarg = argv[++optind];
        } else {
            optopt = c;
            optind++;
            optpos = 0;
            if (opts[0] == ':')
                return ':';
            if (opterr)
                fprintf(stderr, "%s: option requires an argument -- '%c'\n", argv[0], c);
            return '?';
        }
        optind++;
        optpos = 0;
        return c;
    }
    if (!argv[optind][optpos]) {
        optind++;
        optpos = 0;
    }
    return c;
}

/* ---- passwd and group ---- */

static char pwline[512];
static struct passwd pw;

static char *field(char **p)
{
    char *s = *p, *e = strchr(s, ':');
    if (e) {
        *e = 0;
        *p = e + 1;
    } else {
        *p = s + strlen(s);
    }
    return s;
}

static struct passwd *pw_search(const char *name, uid_t uid)
{
    FILE *f = fopen("/etc/passwd", "r");
    if (!f)
        return 0;
    while (fgets(pwline, sizeof pwline, f)) {
        char *p = pwline;
        char *nl = strchr(pwline, '\n');
        if (nl)
            *nl = 0;
        pw.pw_name = field(&p);
        pw.pw_passwd = field(&p);
        pw.pw_uid = (uid_t)atoi(field(&p));
        pw.pw_gid = (gid_t)atoi(field(&p));
        pw.pw_gecos = field(&p);
        pw.pw_dir = field(&p);
        pw.pw_shell = field(&p);
        if (name ? strcmp(pw.pw_name, name) == 0 : pw.pw_uid == uid) {
            fclose(f);
            return &pw;
        }
    }
    fclose(f);
    return 0;
}

struct passwd *getpwnam(const char *name) { return pw_search(name, 0); }
struct passwd *getpwuid(uid_t uid) { return pw_search(0, uid); }

static char grline[512];
static char *grmem[32];
static struct group gr;

static struct group *gr_search(const char *name, gid_t gid)
{
    FILE *f = fopen("/etc/group", "r");
    if (!f)
        return 0;
    while (fgets(grline, sizeof grline, f)) {
        char *p = grline, *m;
        int n = 0;
        char *nl = strchr(grline, '\n');
        if (nl)
            *nl = 0;
        gr.gr_name = field(&p);
        gr.gr_passwd = field(&p);
        gr.gr_gid = (gid_t)atoi(field(&p));
        m = field(&p);
        while (*m && n < 31) {
            grmem[n++] = m;
            m = strchr(m, ',');
            if (!m)
                break;
            *m++ = 0;
        }
        grmem[n] = 0;
        gr.gr_mem = grmem;
        if (name ? strcmp(gr.gr_name, name) == 0 : gr.gr_gid == gid) {
            fclose(f);
            return &gr;
        }
    }
    fclose(f);
    return 0;
}

struct group *getgrnam(const char *name) { return gr_search(name, 0); }
struct group *getgrgid(gid_t gid) { return gr_search(0, gid); }

char *getlogin(void)
{
    char *u = getenv("LOGNAME");
    struct passwd *p;
    if (u)
        return u;
    p = getpwuid(getuid());
    return p ? p->pw_name : 0;
}

char *getpass(const char *prompt)
{
    static char buf[128];
    struct termios t, old;
    int n = 0, c;
    int fd = open("/dev/tty", O_RDWR);
    if (fd < 0)
        fd = 0;
    write(2, prompt, strlen(prompt));
    tcgetattr(fd, &old);
    t = old;
    t.c_lflag &= ~ECHO;
    tcsetattr(fd, TCSAFLUSH, &t);
    while (n < 127) {
        char ch;
        if (read(fd, &ch, 1) <= 0)
            break;
        c = ch;
        if (c == '\n' || c == '\r')
            break;
        buf[n++] = (char)c;
    }
    buf[n] = 0;
    tcsetattr(fd, TCSAFLUSH, &old);
    write(2, "\n", 1);
    if (fd > 2)
        close(fd);
    return buf;
}

/* ---- libgen ---- */

char *basename(char *path)
{
    char *p;
    size_t n;
    if (!path || !*path)
        return ".";
    n = strlen(path);
    while (n > 1 && path[n - 1] == '/')
        path[--n] = 0;
    p = strrchr(path, '/');
    return p && p[1] ? p + 1 : path;
}

char *dirname(char *path)
{
    char *p;
    size_t n;
    if (!path || !*path)
        return ".";
    n = strlen(path);
    while (n > 1 && path[n - 1] == '/')
        path[--n] = 0;
    p = strrchr(path, '/');
    if (!p)
        return ".";
    while (p > path && p[-1] == '/')
        p--;
    if (p == path)
        return "/";
    *p = 0;
    return path;
}

/* ---- assert ---- */

void __assert_fail(const char *expr, const char *file, int line, const char *func)
{
    fprintf(stderr, "%s:%d: %s: Assertion `%s' failed.\n", file, line, func, expr);
    abort();
}

/* ---- locale ---- */

char *setlocale(int cat, const char *locale)
{
    (void)cat;
    if (!locale || !*locale || strcmp(locale, "C") == 0 || strcmp(locale, "POSIX") == 0)
        return "C";
    return 0;
}

struct lconv *localeconv(void)
{
    static struct lconv l = {".", "", ""};
    return &l;
}

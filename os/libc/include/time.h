#ifndef _TIME_H
#define _TIME_H

#include <stddef.h>

typedef long time_t;
typedef long clock_t;
typedef int clockid_t;

#define CLOCKS_PER_SEC 100
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1

struct tm {
    int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst;
    long tm_gmtoff;
    const char *tm_zone;
};

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

time_t time(time_t *t);
clock_t clock(void);
double difftime(time_t a, time_t b);
time_t mktime(struct tm *tm);
time_t timegm(struct tm *tm);
struct tm *gmtime(const time_t *t);
struct tm *gmtime_r(const time_t *t, struct tm *out);
struct tm *localtime(const time_t *t);
struct tm *localtime_r(const time_t *t, struct tm *out);
char *asctime(const struct tm *tm);
char *ctime(const time_t *t);
size_t strftime(char *s, size_t n, const char *fmt, const struct tm *tm);
int nanosleep(const struct timespec *req, struct timespec *rem);
int clock_gettime(clockid_t id, struct timespec *ts);
void tzset(void);

#endif

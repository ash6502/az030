/* <time.h>: calendar time (UTC). */

#include <stdio.h>
#include <string.h>
#include <time.h>

static const char *const wdays[] = {"Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"};
static const char *const months[] = {"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"};
static const char *const lwdays[] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};
static const char *const lmonths[] = {"January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"};

static int leap(int y) { return (y % 4 == 0 && y % 100 != 0) || y % 400 == 0; }

static int mdays(int y, int m)
{
    static const int d[] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return m == 1 && leap(y) ? 29 : d[m];
}

struct tm *gmtime_r(const time_t *t, struct tm *tm)
{
    long secs = *t;
    long days = secs / 86400, rem = secs % 86400;
    int y = 1970, m = 0;
    if (rem < 0) {
        rem += 86400;
        days--;
    }
    tm->tm_hour = rem / 3600;
    tm->tm_min = rem % 3600 / 60;
    tm->tm_sec = rem % 60;
    tm->tm_wday = (int)((days + 4) % 7);
    if (tm->tm_wday < 0)
        tm->tm_wday += 7;
    while (days < 0) {
        y--;
        days += leap(y) ? 366 : 365;
    }
    while (days >= (leap(y) ? 366 : 365)) {
        days -= leap(y) ? 366 : 365;
        y++;
    }
    tm->tm_yday = (int)days;
    while (days >= mdays(y, m)) {
        days -= mdays(y, m);
        m++;
    }
    tm->tm_year = y - 1900;
    tm->tm_mon = m;
    tm->tm_mday = (int)days + 1;
    tm->tm_isdst = 0;
    tm->tm_gmtoff = 0;
    tm->tm_zone = "UTC";
    return tm;
}

struct tm *gmtime(const time_t *t)
{
    static struct tm tm;
    return gmtime_r(t, &tm);
}

struct tm *localtime_r(const time_t *t, struct tm *tm) { return gmtime_r(t, tm); }
struct tm *localtime(const time_t *t) { return gmtime(t); }
void tzset(void) {}

time_t timegm(struct tm *tm)
{
    long days = 0;
    int y, m;
    int year = tm->tm_year + 1900, mon = tm->tm_mon;
    while (mon < 0) {
        mon += 12;
        year--;
    }
    while (mon > 11) {
        mon -= 12;
        year++;
    }
    if (year >= 1970)
        for (y = 1970; y < year; y++)
            days += leap(y) ? 366 : 365;
    else
        for (y = year; y < 1970; y++)
            days -= leap(y) ? 366 : 365;
    for (m = 0; m < mon; m++)
        days += mdays(year, m);
    days += tm->tm_mday - 1;
    {
        time_t t = days * 86400 + tm->tm_hour * 3600 + tm->tm_min * 60 + tm->tm_sec;
        gmtime_r(&t, tm);
        return t;
    }
}

time_t mktime(struct tm *tm) { return timegm(tm); }

double difftime(time_t a, time_t b) { return (double)(a - b); }

char *asctime(const struct tm *tm)
{
    static char buf[32];
    sprintf(buf, "%s %s %2d %02d:%02d:%02d %d\n", wdays[tm->tm_wday % 7], months[tm->tm_mon % 12], tm->tm_mday, tm->tm_hour, tm->tm_min, tm->tm_sec,
            tm->tm_year + 1900);
    return buf;
}

char *ctime(const time_t *t) { return asctime(localtime(t)); }

size_t strftime(char *s, size_t n, const char *fmt, const struct tm *tm)
{
    size_t len = 0;
    char tmp[64];
    for (; *fmt; fmt++) {
        const char *add = tmp;
        if (*fmt != '%') {
            tmp[0] = *fmt;
            tmp[1] = 0;
        } else {
            int h12;
            fmt++;
            if (*fmt == 'E' || *fmt == 'O')
                fmt++;
            h12 = tm->tm_hour % 12 == 0 ? 12 : tm->tm_hour % 12;
            switch (*fmt) {
            case 'a': add = wdays[tm->tm_wday % 7]; break;
            case 'A': add = lwdays[tm->tm_wday % 7]; break;
            case 'b':
            case 'h': add = months[tm->tm_mon % 12]; break;
            case 'B': add = lmonths[tm->tm_mon % 12]; break;
            case 'c': sprintf(tmp, "%s %s %2d %02d:%02d:%02d %d", wdays[tm->tm_wday % 7], months[tm->tm_mon % 12], tm->tm_mday, tm->tm_hour, tm->tm_min, tm->tm_sec, tm->tm_year + 1900); break;
            case 'C': sprintf(tmp, "%02d", (tm->tm_year + 1900) / 100); break;
            case 'd': sprintf(tmp, "%02d", tm->tm_mday); break;
            case 'D': sprintf(tmp, "%02d/%02d/%02d", tm->tm_mon + 1, tm->tm_mday, tm->tm_year % 100); break;
            case 'e': sprintf(tmp, "%2d", tm->tm_mday); break;
            case 'F': sprintf(tmp, "%d-%02d-%02d", tm->tm_year + 1900, tm->tm_mon + 1, tm->tm_mday); break;
            case 'H': sprintf(tmp, "%02d", tm->tm_hour); break;
            case 'I': sprintf(tmp, "%02d", h12); break;
            case 'j': sprintf(tmp, "%03d", tm->tm_yday + 1); break;
            case 'k': sprintf(tmp, "%2d", tm->tm_hour); break;
            case 'l': sprintf(tmp, "%2d", h12); break;
            case 'm': sprintf(tmp, "%02d", tm->tm_mon + 1); break;
            case 'M': sprintf(tmp, "%02d", tm->tm_min); break;
            case 'n': add = "\n"; break;
            case 'p': add = tm->tm_hour < 12 ? "AM" : "PM"; break;
            case 'P': add = tm->tm_hour < 12 ? "am" : "pm"; break;
            case 'r': sprintf(tmp, "%02d:%02d:%02d %s", h12, tm->tm_min, tm->tm_sec, tm->tm_hour < 12 ? "AM" : "PM"); break;
            case 'R': sprintf(tmp, "%02d:%02d", tm->tm_hour, tm->tm_min); break;
            case 's': {
                struct tm c = *tm;
                sprintf(tmp, "%ld", (long)timegm(&c));
                break;
            }
            case 'S': sprintf(tmp, "%02d", tm->tm_sec); break;
            case 't': add = "\t"; break;
            case 'T': sprintf(tmp, "%02d:%02d:%02d", tm->tm_hour, tm->tm_min, tm->tm_sec); break;
            case 'u': sprintf(tmp, "%d", tm->tm_wday == 0 ? 7 : tm->tm_wday); break;
            case 'w': sprintf(tmp, "%d", tm->tm_wday); break;
            case 'x': sprintf(tmp, "%02d/%02d/%02d", tm->tm_mon + 1, tm->tm_mday, tm->tm_year % 100); break;
            case 'X': sprintf(tmp, "%02d:%02d:%02d", tm->tm_hour, tm->tm_min, tm->tm_sec); break;
            case 'y': sprintf(tmp, "%02d", tm->tm_year % 100); break;
            case 'Y': sprintf(tmp, "%d", tm->tm_year + 1900); break;
            case 'z': add = "+0000"; break;
            case 'Z': add = "UTC"; break;
            case '%': add = "%"; break;
            default:
                tmp[0] = '%';
                tmp[1] = *fmt;
                tmp[2] = 0;
            }
        }
        {
            size_t l = strlen(add);
            if (len + l >= n)
                return 0;
            memcpy(s + len, add, l);
            len += l;
        }
        if (!*fmt)
            break;
    }
    s[len] = 0;
    return len;
}

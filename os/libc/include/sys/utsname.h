#ifndef _SYS_UTSNAME_H
#define _SYS_UTSNAME_H

struct utsname {
    char sysname[32];
    char nodename[32];
    char release[32];
    char version[64];
    char machine[32];
};

int uname(struct utsname *u);

#endif

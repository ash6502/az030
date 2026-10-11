#ifndef _TERMIOS_H
#define _TERMIOS_H

#include <sys/types.h>

typedef unsigned int tcflag_t;
typedef unsigned char cc_t;
typedef unsigned int speed_t;

#define NCCS 20

struct termios {
    tcflag_t c_iflag, c_oflag, c_cflag, c_lflag;
    cc_t c_cc[NCCS];
};

#define VINTR 0
#define VQUIT 1
#define VERASE 2
#define VKILL 3
#define VEOF 4
#define VTIME 5
#define VMIN 6
#define VSUSP 10
#define VWERASE 14

#define ISTRIP 0x20
#define INLCR 0x40
#define IGNCR 0x80
#define ICRNL 0x100
#define IXON 0x400
#define BRKINT 0x2
#define INPCK 0x10
#define OPOST 0x1
#define ONLCR 0x4
#define CS8 0x30
#define ISIG 0x1
#define ICANON 0x2
#define ECHO 0x8
#define ECHOE 0x10
#define ECHOK 0x20
#define ECHONL 0x40
#define ECHOCTL 0x200
#define IEXTEN 0x8000

#define TCSANOW 0
#define TCSADRAIN 1
#define TCSAFLUSH 2

#define B0 0
#define B9600 9600
#define B19200 19200
#define B38400 38400
#define B115200 115200

int tcgetattr(int fd, struct termios *t);
int tcsetattr(int fd, int when, const struct termios *t);
void cfmakeraw(struct termios *t);
speed_t cfgetospeed(const struct termios *t);
speed_t cfgetispeed(const struct termios *t);
int cfsetospeed(struct termios *t, speed_t s);
int cfsetispeed(struct termios *t, speed_t s);
int tcflush(int fd, int which);
int tcdrain(int fd);

#define TCIFLUSH 0
#define TCOFLUSH 1
#define TCIOFLUSH 2

#endif

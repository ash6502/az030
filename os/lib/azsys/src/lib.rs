//! The az030 OS system-call ABI, shared by the kernel and user-space runtimes.
//!
//! Calling convention: `trap #0` with the call number in d0 and arguments in
//! d1, d2, d3, d4, d5. The result comes back in d0; values in -4095..-1 are
//! negated errno codes. All other registers are preserved.
//!
//! Structures exchanged with the kernel are made of 32-bit words only, so their
//! layout is the same for every compiler on the system.

#![no_std]

pub mod nr {
    pub const EXIT: u32 = 0;
    pub const FORK: u32 = 1;
    pub const READ: u32 = 2;
    pub const WRITE: u32 = 3;
    pub const OPEN: u32 = 4;
    pub const CLOSE: u32 = 5;
    pub const WAITPID: u32 = 6;
    pub const LINK: u32 = 7;
    pub const UNLINK: u32 = 8;
    pub const EXECVE: u32 = 9;
    pub const CHDIR: u32 = 10;
    pub const TIME: u32 = 11;
    pub const MKNOD: u32 = 12;
    pub const CHMOD: u32 = 13;
    pub const CHOWN: u32 = 14;
    pub const BRK: u32 = 15;
    pub const STAT: u32 = 16;
    pub const LSEEK: u32 = 17;
    pub const GETPID: u32 = 18;
    pub const MOUNT: u32 = 19;
    pub const UMOUNT: u32 = 20;
    pub const SETUID: u32 = 21;
    pub const GETUID: u32 = 22;
    pub const FSTAT: u32 = 23;
    pub const PAUSE: u32 = 24;
    pub const UTIME: u32 = 25;
    pub const ACCESS: u32 = 26;
    pub const SYNC: u32 = 27;
    pub const KILL: u32 = 28;
    pub const RENAME: u32 = 29;
    pub const MKDIR: u32 = 30;
    pub const RMDIR: u32 = 31;
    pub const DUP: u32 = 32;
    pub const PIPE: u32 = 33;
    pub const TIMES: u32 = 34;
    pub const SETGID: u32 = 35;
    pub const GETGID: u32 = 36;
    pub const SIGACTION: u32 = 37;
    pub const IOCTL: u32 = 38;
    pub const FCNTL: u32 = 39;
    pub const SETPGID: u32 = 40;
    pub const UMASK: u32 = 41;
    pub const DUP2: u32 = 42;
    pub const GETPPID: u32 = 43;
    pub const SETSID: u32 = 44;
    pub const GETCWD: u32 = 45;
    pub const GETDENTS: u32 = 46;
    pub const LSTAT: u32 = 47;
    pub const SYMLINK: u32 = 48;
    pub const READLINK: u32 = 49;
    pub const SLEEP_MS: u32 = 50;
    pub const GETTIMEOFDAY: u32 = 51;
    pub const UNAME: u32 = 52;
    pub const SIGRETURN: u32 = 53;
    pub const SIGPROCMASK: u32 = 54;
    pub const GETPGID: u32 = 55;
    pub const REBOOT: u32 = 56;
    pub const PROCINFO: u32 = 57;
    pub const SYSINFO: u32 = 58;
    pub const ALARM: u32 = 59;
    pub const FTRUNCATE: u32 = 60;
    pub const FSYNC: u32 = 61;
    pub const STATFS: u32 = 62;
    pub const GETEUID: u32 = 63;
    pub const GETEGID: u32 = 64;
    pub const SETTIMEOFDAY: u32 = 65;
    pub const MOUNTINFO: u32 = 66;
    pub const COUNT: u32 = 67;
}

pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const ESRCH: i32 = 3;
    pub const EINTR: i32 = 4;
    pub const EIO: i32 = 5;
    pub const ENXIO: i32 = 6;
    pub const E2BIG: i32 = 7;
    pub const ENOEXEC: i32 = 8;
    pub const EBADF: i32 = 9;
    pub const ECHILD: i32 = 10;
    pub const EAGAIN: i32 = 11;
    pub const ENOMEM: i32 = 12;
    pub const EACCES: i32 = 13;
    pub const EFAULT: i32 = 14;
    pub const ENOTBLK: i32 = 15;
    pub const EBUSY: i32 = 16;
    pub const EEXIST: i32 = 17;
    pub const EXDEV: i32 = 18;
    pub const ENODEV: i32 = 19;
    pub const ENOTDIR: i32 = 20;
    pub const EISDIR: i32 = 21;
    pub const EINVAL: i32 = 22;
    pub const ENFILE: i32 = 23;
    pub const EMFILE: i32 = 24;
    pub const ENOTTY: i32 = 25;
    pub const ETXTBSY: i32 = 26;
    pub const EFBIG: i32 = 27;
    pub const ENOSPC: i32 = 28;
    pub const ESPIPE: i32 = 29;
    pub const EROFS: i32 = 30;
    pub const EMLINK: i32 = 31;
    pub const EPIPE: i32 = 32;
    pub const EDOM: i32 = 33;
    pub const ERANGE: i32 = 34;
    pub const ENAMETOOLONG: i32 = 36;
    pub const ENOSYS: i32 = 38;
    pub const ENOTEMPTY: i32 = 39;
    pub const ELOOP: i32 = 40;

    pub fn message(e: i32) -> &'static str {
        match e {
            EPERM => "Operation not permitted",
            ENOENT => "No such file or directory",
            ESRCH => "No such process",
            EINTR => "Interrupted system call",
            EIO => "I/O error",
            ENXIO => "No such device or address",
            E2BIG => "Argument list too long",
            ENOEXEC => "Exec format error",
            EBADF => "Bad file descriptor",
            ECHILD => "No child processes",
            EAGAIN => "Resource temporarily unavailable",
            ENOMEM => "Out of memory",
            EACCES => "Permission denied",
            EFAULT => "Bad address",
            ENOTBLK => "Block device required",
            EBUSY => "Device or resource busy",
            EEXIST => "File exists",
            EXDEV => "Cross-device link",
            ENODEV => "No such device",
            ENOTDIR => "Not a directory",
            EISDIR => "Is a directory",
            EINVAL => "Invalid argument",
            ENFILE => "Too many open files in system",
            EMFILE => "Too many open files",
            ENOTTY => "Inappropriate ioctl for device",
            ETXTBSY => "Text file busy",
            EFBIG => "File too large",
            ENOSPC => "No space left on device",
            ESPIPE => "Illegal seek",
            EROFS => "Read-only file system",
            EMLINK => "Too many links",
            EPIPE => "Broken pipe",
            EDOM => "Numerical argument out of domain",
            ERANGE => "Numerical result out of range",
            ENAMETOOLONG => "File name too long",
            ENOSYS => "Function not implemented",
            ENOTEMPTY => "Directory not empty",
            ELOOP => "Too many levels of symbolic links",
            _ => "Unknown error",
        }
    }
}

pub mod sig {
    pub const SIGHUP: u32 = 1;
    pub const SIGINT: u32 = 2;
    pub const SIGQUIT: u32 = 3;
    pub const SIGILL: u32 = 4;
    pub const SIGTRAP: u32 = 5;
    pub const SIGABRT: u32 = 6;
    pub const SIGBUS: u32 = 7;
    pub const SIGFPE: u32 = 8;
    pub const SIGKILL: u32 = 9;
    pub const SIGUSR1: u32 = 10;
    pub const SIGSEGV: u32 = 11;
    pub const SIGUSR2: u32 = 12;
    pub const SIGPIPE: u32 = 13;
    pub const SIGALRM: u32 = 14;
    pub const SIGTERM: u32 = 15;
    pub const SIGCHLD: u32 = 17;
    pub const SIGCONT: u32 = 18;
    pub const SIGSTOP: u32 = 19;
    pub const SIGTSTP: u32 = 20;
    pub const SIGTTIN: u32 = 21;
    pub const SIGTTOU: u32 = 22;
    pub const SIGWINCH: u32 = 28;
    pub const NSIG: u32 = 32;

    pub const SIG_DFL: u32 = 0;
    pub const SIG_IGN: u32 = 1;

    pub const SIG_BLOCK: u32 = 0;
    pub const SIG_UNBLOCK: u32 = 1;
    pub const SIG_SETMASK: u32 = 2;

    pub fn name(s: u32) -> &'static str {
        match s {
            SIGHUP => "Hangup",
            SIGINT => "Interrupt",
            SIGQUIT => "Quit",
            SIGILL => "Illegal instruction",
            SIGTRAP => "Trace/breakpoint trap",
            SIGABRT => "Aborted",
            SIGBUS => "Bus error",
            SIGFPE => "Floating point exception",
            SIGKILL => "Killed",
            SIGUSR1 => "User defined signal 1",
            SIGSEGV => "Segmentation fault",
            SIGUSR2 => "User defined signal 2",
            SIGPIPE => "Broken pipe",
            SIGALRM => "Alarm clock",
            SIGTERM => "Terminated",
            SIGCHLD => "Child exited",
            SIGCONT => "Continued",
            SIGSTOP => "Stopped (signal)",
            SIGTSTP => "Stopped",
            SIGTTIN => "Stopped (tty input)",
            SIGTTOU => "Stopped (tty output)",
            SIGWINCH => "Window changed",
            _ => "Unknown signal",
        }
    }
}

pub mod flags {
    pub const O_RDONLY: u32 = 0;
    pub const O_WRONLY: u32 = 1;
    pub const O_RDWR: u32 = 2;
    pub const O_ACCMODE: u32 = 3;
    pub const O_CREAT: u32 = 0x40;
    pub const O_EXCL: u32 = 0x80;
    pub const O_NOCTTY: u32 = 0x100;
    pub const O_TRUNC: u32 = 0x200;
    pub const O_APPEND: u32 = 0x400;
    pub const O_NONBLOCK: u32 = 0x800;
    pub const O_DIRECTORY: u32 = 0x10000;
    pub const O_NOFOLLOW: u32 = 0x20000;
    pub const O_CLOEXEC: u32 = 0x80000;

    pub const SEEK_SET: u32 = 0;
    pub const SEEK_CUR: u32 = 1;
    pub const SEEK_END: u32 = 2;

    pub const F_DUPFD: u32 = 0;
    pub const F_GETFD: u32 = 1;
    pub const F_SETFD: u32 = 2;
    pub const F_GETFL: u32 = 3;
    pub const F_SETFL: u32 = 4;
    pub const FD_CLOEXEC: u32 = 1;

    pub const WNOHANG: u32 = 1;
    pub const WUNTRACED: u32 = 2;

    pub const R_OK: u32 = 4;
    pub const W_OK: u32 = 2;
    pub const X_OK: u32 = 1;
    pub const F_OK: u32 = 0;

    pub const AT_FDCWD: i32 = -100;

    pub const REBOOT_RESTART: u32 = 1;
    pub const REBOOT_HALT: u32 = 2;
    pub const REBOOT_POWEROFF: u32 = 3;

    pub const MS_RDONLY: u32 = 1;
}

pub mod mode {
    pub const S_IFMT: u32 = 0o170000;
    pub const S_IFSOCK: u32 = 0o140000;
    pub const S_IFLNK: u32 = 0o120000;
    pub const S_IFREG: u32 = 0o100000;
    pub const S_IFBLK: u32 = 0o060000;
    pub const S_IFDIR: u32 = 0o040000;
    pub const S_IFCHR: u32 = 0o020000;
    pub const S_IFIFO: u32 = 0o010000;
    pub const S_ISUID: u32 = 0o4000;
    pub const S_ISGID: u32 = 0o2000;
    pub const S_ISVTX: u32 = 0o1000;
}

/// Terminal ioctls and termios bits.
pub mod tty {
    pub const TCGETS: u32 = 0x5401;
    pub const TCSETS: u32 = 0x5402;
    pub const TCSETSW: u32 = 0x5403;
    pub const TCSETSF: u32 = 0x5404;
    pub const TIOCGPGRP: u32 = 0x540F;
    pub const TIOCSPGRP: u32 = 0x5410;
    pub const TIOCGWINSZ: u32 = 0x5413;
    pub const TIOCSWINSZ: u32 = 0x5414;
    pub const FIONREAD: u32 = 0x541B;

    // iflag
    pub const ICRNL: u32 = 0x100;
    pub const INLCR: u32 = 0x40;
    pub const IGNCR: u32 = 0x80;
    pub const IXON: u32 = 0x400;
    pub const ISTRIP: u32 = 0x20;
    // oflag
    pub const OPOST: u32 = 0x1;
    pub const ONLCR: u32 = 0x4;
    // lflag
    pub const ISIG: u32 = 0x1;
    pub const ICANON: u32 = 0x2;
    pub const ECHO: u32 = 0x8;
    pub const ECHOE: u32 = 0x10;
    pub const ECHOK: u32 = 0x20;
    pub const ECHONL: u32 = 0x40;
    pub const ECHOCTL: u32 = 0x200;
    pub const IEXTEN: u32 = 0x8000;
    // cc indices
    pub const VINTR: usize = 0;
    pub const VQUIT: usize = 1;
    pub const VERASE: usize = 2;
    pub const VKILL: usize = 3;
    pub const VEOF: usize = 4;
    pub const VTIME: usize = 5;
    pub const VMIN: usize = 6;
    pub const VSUSP: usize = 10;
    pub const VWERASE: usize = 14;
    pub const NCCS: usize = 20;
}

/// `stat` / `fstat` / `lstat` result.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    pub dev: u32,
    pub ino: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub size: u32,
    pub atime: u32,
    pub mtime: u32,
    pub ctime: u32,
    pub blksize: u32,
    pub blocks: u32,
}

/// Header of a `getdents` record; the NUL-terminated name follows and the record
/// is padded to a multiple of 4 bytes (`reclen`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DirentHdr {
    pub ino: u32,
    pub reclen: u16,
    pub dtype: u8,
    pub namlen: u8,
}

pub const DT_UNKNOWN: u8 = 0;
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_BLK: u8 = 6;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub cc: [u8; tty::NCCS],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Winsize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timeval {
    pub sec: u32,
    pub usec: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Tms {
    pub utime: u32,
    pub stime: u32,
    pub cutime: u32,
    pub cstime: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SigAction {
    /// SIG_DFL, SIG_IGN or a handler address.
    pub handler: u32,
    pub mask: u32,
    pub flags: u32,
    /// Code that calls `sigreturn` when the handler returns.
    pub restorer: u32,
}

pub const SA_NOCLDSTOP: u32 = 0x0000_0001;
pub const SA_RESTART: u32 = 0x1000_0000;
pub const SA_NODEFER: u32 = 0x4000_0000;
pub const SA_RESETHAND: u32 = 0x8000_0000;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Utsname {
    pub sysname: [u8; 32],
    pub nodename: [u8; 32],
    pub release: [u8; 32],
    pub version: [u8; 64],
    pub machine: [u8; 32],
}

/// Process states reported by `procinfo`.
pub const PS_RUN: u32 = 0;
pub const PS_SLEEP: u32 = 1;
pub const PS_STOP: u32 = 2;
pub const PS_ZOMBIE: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    pub sid: u32,
    pub uid: u32,
    pub state: u32,
    /// Resident memory in KB.
    pub mem_kb: u32,
    /// CPU time in clock ticks.
    pub utime: u32,
    pub stime: u32,
    /// Seconds since boot when the process started.
    pub start: u32,
    pub tty: u32,
    pub name: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SysInfo {
    pub uptime: u32,
    pub total_kb: u32,
    pub free_kb: u32,
    pub kernel_kb: u32,
    pub buffers_kb: u32,
    pub procs: u32,
    pub hz: u32,
    pub load: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StatFs {
    pub bsize: u32,
    pub blocks: u32,
    pub bfree: u32,
    pub files: u32,
    pub ffree: u32,
    pub flags: u32,
}

/// One entry of `mountinfo`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MountInfo {
    pub dev: u32,
    pub flags: u32,
    pub path: [u8; 64],
    pub source: [u8; 32],
}

/// Clock ticks per second.
pub const HZ: u32 = 100;

/// Make a device number.
pub const fn makedev(major: u32, minor: u32) -> u32 {
    major << 8 | minor
}

pub mod dev {
    /// Character devices.
    pub const MEM_MAJOR: u32 = 1; // 3 null, 5 zero, 8 random
    pub const TTY_MAJOR: u32 = 4; // 0 console (UART)
    pub const CTTY_MAJOR: u32 = 5; // 0 /dev/tty
    /// Block devices: SCSI disks, minor = SCSI ID.
    pub const SD_MAJOR: u32 = 8;
}

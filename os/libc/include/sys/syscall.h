#ifndef _SYS_SYSCALL_H
#define _SYS_SYSCALL_H

/* System call numbers (trap #0, number in d0, arguments in d1-d5). */
#define SYS_exit 0
#define SYS_fork 1
#define SYS_read 2
#define SYS_write 3
#define SYS_open 4
#define SYS_close 5
#define SYS_waitpid 6
#define SYS_link 7
#define SYS_unlink 8
#define SYS_execve 9
#define SYS_chdir 10
#define SYS_time 11
#define SYS_mknod 12
#define SYS_chmod 13
#define SYS_chown 14
#define SYS_brk 15
#define SYS_stat 16
#define SYS_lseek 17
#define SYS_getpid 18
#define SYS_mount 19
#define SYS_umount 20
#define SYS_setuid 21
#define SYS_getuid 22
#define SYS_fstat 23
#define SYS_pause 24
#define SYS_utime 25
#define SYS_access 26
#define SYS_sync 27
#define SYS_kill 28
#define SYS_rename 29
#define SYS_mkdir 30
#define SYS_rmdir 31
#define SYS_dup 32
#define SYS_pipe 33
#define SYS_times 34
#define SYS_setgid 35
#define SYS_getgid 36
#define SYS_sigaction 37
#define SYS_ioctl 38
#define SYS_fcntl 39
#define SYS_setpgid 40
#define SYS_umask 41
#define SYS_dup2 42
#define SYS_getppid 43
#define SYS_setsid 44
#define SYS_getcwd 45
#define SYS_getdents 46
#define SYS_lstat 47
#define SYS_symlink 48
#define SYS_readlink 49
#define SYS_sleep_ms 50
#define SYS_gettimeofday 51
#define SYS_uname 52
#define SYS_sigreturn 53
#define SYS_sigprocmask 54
#define SYS_getpgid 55
#define SYS_reboot 56
#define SYS_procinfo 57
#define SYS_sysinfo 58
#define SYS_alarm 59
#define SYS_ftruncate 60
#define SYS_fsync 61
#define SYS_statfs 62
#define SYS_geteuid 63
#define SYS_getegid 64
#define SYS_settimeofday 65
#define SYS_mountinfo 66
#define SYS_klog 67

long __syscall(long nr, long a1, long a2, long a3, long a4, long a5);
long syscall(long nr, ...);

#endif

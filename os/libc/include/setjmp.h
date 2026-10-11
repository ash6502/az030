#ifndef _SETJMP_H
#define _SETJMP_H

/* return address, sp, d2-d7, a2-a6, fp2-fp7 */
typedef long jmp_buf[13 + 18];
typedef jmp_buf sigjmp_buf;

int setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val);
int _setjmp(jmp_buf env);
void _longjmp(jmp_buf env, int val);
int sigsetjmp(sigjmp_buf env, int savemask);
void siglongjmp(sigjmp_buf env, int val);

#endif

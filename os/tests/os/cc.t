# Compile and run C programs on AZ/UX.
login root
> cd /tmp && cc -o hello /usr/src/examples/hello.c && ./hello one two
? argument 2: two
> cc -o primes /usr/src/examples/primes.c && ./primes 50
? 15 primes up to 50
> cc -S /usr/src/examples/hello.c && head -5 hello.s
? _main
> printf 'int sq(int x) { return x * x; }\n' > sq.c && printf 'int sq(int);\n#include <stdio.h>\nint main(void) { printf("%%d\\n", sq(12)); return 0; }\n' > m.c && cc -c sq.c && cc -o m m.c sq.o && ./m
? ^144

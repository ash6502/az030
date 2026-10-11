# Assemble and link on the OS.
login root
> cd /tmp && as /usr/src/examples/hello.s && ld -o hello hello.o && ./hello && ar rcs libh.a hello.o && ar t libh.a
? ^hello.o
! error

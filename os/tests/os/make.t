# make: rules, variables, functions, incremental rebuilds.
login root
> mkdir -p /tmp/mk && cd /tmp/mk && printf 'int add(int a, int b);\n' > add.h && printf '#include "add.h"\nint add(int a, int b) { return a + b; }\n' > add.c
> printf '#include <stdio.h>\n#include "add.h"\nint main(void) { printf("sum=%%d\\n", add(40, 2)); return 0; }\n' > main.c
> printf 'SRCS := $(wildcard *.c)\nOBJS = $(SRCS:.c=.o)\nCFLAGS = -O\n\nprog: $(OBJS)\n\t$(CC) -o $@ $^\n\n%%.o: %%.c add.h\n\t$(CC) $(CFLAGS) -c $<\n\n.PHONY: clean show\nclean:\n\trm -f prog $(OBJS)\nshow:\n\t@echo objs=$(OBJS) upper=$(patsubst %%.o,%%.O,$(OBJS)) n=$(words $(SRCS))\n' > Makefile && cat Makefile
> make && ./prog
? sum=42
> make
? up to date
> touch add.h && make -n
? cc -O -c add.c
> make show
? objs=add.o main.o upper=add.O main.O n=2
> make clean && ls
! add.o

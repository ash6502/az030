# Core utilities.
login root
> ls -l /etc
? passwd
> ls /
? bin
> mkdir -p /tmp/t/a/b && cd /tmp/t && echo hello > f1 && cat f1 && cp f1 f2 && mv f2 a/f3 && ls -R
? f3
> printf '%s %5d|%-4s|%x %.2f\n' abc 42 xy 255 3.14159
? abc    42\|xy  \|ff 3.14
> seq 1 5 | sort -nr | head -3 | tr '\n' ' '; echo
? 5 4 3
> echo "one two three" | wc -w; printf 'b\na\nb\nc\n' | sort | uniq -c
? 2 b
> grep -n root /etc/passwd; grep -c x /etc/group
? 1:root
> echo 'hello world' | sed 's/world/there/; s/^/> /'
? > hello there
> printf 'x\ny\nz\n' > a1; printf 'x\nY\nz\n' > a2; diff a1 a2; diff -u a1 a2
? \+Y
> find /tmp/t -name 'f*' | sort
? /tmp/t/f1
> expr 6 \* 7; test 3 -lt 5 && echo lt; [ -f /etc/passwd ] && echo file
? file
> date +%Y; uname -a; id; whoami
? uid=0\(root\)
> ps
? ps
> df -h; du -s /etc; free
? Mem:
> cut -d: -f1,7 /etc/passwd; echo abc | od -c | head -1
? root:/bin/sh
> sha256sum /etc/hostname; echo hi | rev; seq 3 | tac | paste -s -d,
? 3,2,1
> chmod 600 f1 && ls -l f1 && ln -s f1 link && readlink link && stat f1 | head -2
? -rw-------
> cd / && rm -r /tmp/t && ls -a /tmp

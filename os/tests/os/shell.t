# Shell language basics (builtins only).
login root
> echo hello $USER; pwd; cd /etc; pwd
? ^/etc$
> for i in 1 2 3; do echo "n=$i $((i*i))"; done
? n=3 9
> x=abc.tar.gz; echo ${x%%.*} ${x#*.} ${#x}; echo $(echo sub $(echo nested))
? ^abc tar.gz 10
> f() { echo "args: $# $@"; return 3; }; f a "b c"; echo st=$?
? st=3
> if [ -d /etc ]; then echo yes; else echo no; fi; case abc in a*) echo matched;; *) echo nope;; esac
? matched
> cat <<EOF
= here $USER
= EOF
> echo /etc/*
? /etc/group
> type cd sh; echo $((7 * (3 + 4) % 5))
? ^4$

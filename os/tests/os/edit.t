# The editor: type, save, search, cut/paste, undo.
login root
> edit /tmp/e.c
sleep 2
raw int main(void)\r{\r\treturn 0;\r}\r
sleep 1
key O
sleep 1
raw \r
? Wrote 5 lines
key W
raw return\r
sleep 1
key K
key U
key U
sleep 1
raw \x1bu
sleep 1
key X
sleep 1
raw y
sleep 1
> cat -A /tmp/e.c
? ^\^Ireturn 0;\$

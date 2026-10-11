#!/bin/sh
# Differential test of azcc + libc: build each tests/c/*.c with the host C
# compiler and with azcc for AZ/UX, run both (AZ/UX in the emulator) and compare.
#
#   tests/c/run.sh [test.c...]
set -e
OS=$(cd "$(dirname "$0")/../.." && pwd)
B=$OS/build/ctest
T=$OS/target/release
rm -rf "$B"
mkdir -p "$B/root"
cp -R "$OS/build/root/." "$B/root/"
mkdir -p "$B/root/tests"
tests=${*:-$OS/tests/c/*.c}
script=$B/script.t
echo "login root" > "$script"
for t in $tests; do
    n=$(basename "$t" .c)
    cc -w -o "$B/$n.host" "$t" -lm
    "$B/$n.host" > "$B/$n.expected"
    "$T/azcc" -isystem "$OS/libc/include" -c -o "$B/$n.o" "$t"
    "$T/azld" -o "$B/root/tests/$n" "$OS/build/libc/crt0.o" "$B/$n.o" "$OS/build/libc/libc.a"
    chmod +x "$B/root/tests/$n"
    echo "> echo '=== $n'; /tests/$n; echo '=== end'" >> "$script"
done
"$T/azmkdisk" -o "$B/disk.img" --size 32M --root "$B/root" --manifest "$OS/root/manifest" \
    --boot "$OS/build/boot.bin" --kernel "$OS/build/kernel.bin" --load 0x10000 \
    --entry 0x$(grep ' _start$' "$OS/build/kernel.map" | cut -d' ' -f1) >/dev/null
sed -e "s|@DISK@|$B/disk.img|" -e "s|@ROM@|$OS/../rom/boot.bin|" "$OS/az030.toml.in" | sed 's/throttle = true/throttle = false/' > "$B/emu.toml"
python3 "$OS/tools/emutest.py" -c "$B/emu.toml" -t 120 "$script" > "$B/output.log" 2>&1 || true
fail=0
for t in $tests; do
    n=$(basename "$t" .c)
    tr -d '\r' < "$B/output.log" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | sed -n "/^=== $n\$/,/^=== end\$/p" | sed '1d;$d' > "$B/$n.actual"
    if diff -u "$B/$n.expected" "$B/$n.actual" > "$B/$n.diff"; then
        echo "PASS $n"
    else
        echo "FAIL $n"
        cat "$B/$n.diff"
        fail=1
    fi
done
exit $fail

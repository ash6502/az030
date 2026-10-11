#!/usr/bin/env python3
"""Build /usr/share/man for the root file system.

    tools/genman.py OUTDIR

Section 1 pages are generated from the `//!` documentation comment at the top
of each program in user/src/bin; hand-written pages in man/manN/ replace or add
to them. Aliases (egrep, [, ...) get a one-line page pointing at the real one.
"""

import os
import re
import sys
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
OS = os.path.dirname(HERE)
BIN = os.path.join(OS, "user", "src", "bin")
MAN = os.path.join(OS, "man")


def doc_comment(path):
    lines = []
    with open(path) as f:
        for line in f:
            if line.startswith("//!"):
                lines.append(line[3:].rstrip("\n")[1:] if line[3:4] == " " else line[3:].rstrip("\n"))
            elif lines or line.strip():
                break
    return lines


def page_from_doc(name, doc):
    if not doc:
        return None
    first = doc[0]
    m = re.match(r"([^:]+):\s*(.*)", first)
    names, summary = (m.group(1), m.group(2)) if m else (name, first)
    summary = summary.rstrip(".")
    body = doc[1:]
    while body and not body[0].strip():
        body.pop(0)
    synopsis = []
    desc = []
    for l in body:
        if l.startswith("    ") and not desc:
            synopsis.append(l.strip())
        else:
            desc.append(l)
    out = [f"{name.upper()}(1)", "", "NAME", f"     {names.strip()} - {summary}", ""]
    if synopsis:
        out.append("SYNOPSIS")
        out += ["     " + s for s in synopsis]
        out.append("")
    text = "\n".join(desc).strip()
    if text:
        out.append("DESCRIPTION")
        for para in re.split(r"\n\s*\n", text):
            if para.startswith("    "):
                out += ["     " + l for l in para.splitlines()]
            else:
                joined = " ".join(l.strip() for l in para.splitlines())
                out += textwrap.wrap(joined, 70, initial_indent="     ", subsequent_indent="     ")
            out.append("")
    out.append("SEE ALSO")
    out.append("     intro(1)")
    return "\n".join(out) + "\n"


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    outdir = sys.argv[1]
    os.makedirs(os.path.join(outdir, "man1"), exist_ok=True)
    for entry in sorted(os.listdir(BIN)):
        p = os.path.join(BIN, entry)
        if entry.endswith(".rs"):
            name, src = entry[:-3], p
        elif os.path.isdir(p) and os.path.exists(os.path.join(p, "main.rs")):
            name, src = entry, os.path.join(p, "main.rs")
        else:
            continue
        page = page_from_doc(name, doc_comment(src))
        if page:
            with open(os.path.join(outdir, "man1", name + ".1"), "w") as f:
                f.write(page)
    # hand-written pages
    if os.path.isdir(MAN):
        for sec in sorted(os.listdir(MAN)):
            d = os.path.join(MAN, sec)
            if not os.path.isdir(d):
                continue
            os.makedirs(os.path.join(outdir, sec), exist_ok=True)
            for page in os.listdir(d):
                with open(os.path.join(d, page)) as f:
                    text = f.read()
                with open(os.path.join(outdir, sec, page), "w") as f:
                    f.write(text)


if __name__ == "__main__":
    main()

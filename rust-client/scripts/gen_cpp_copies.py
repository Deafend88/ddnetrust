"""Expand C++ copy markers in an rs_parity tool source (PLAN §12.4, kind 2).

A template line `//@@COPY <file> <a> <b>` (file relative to `src/`, lines 1-based,
inclusive) becomes

    // C++ copy: <file>:<a>-<b>
    <those source lines, verbatim>
    // end of copy

which `cpp_copies_match_source` (ddnet/tests/parity.rs) later checks against the source.
Run from the repo root:

    python rust-client/scripts/gen_cpp_copies.py <template.cpp> src/tools/<tool>.cpp
"""

import re
import sys

src, out = sys.argv[1], sys.argv[2]
res = []
for line in open(src, encoding="utf-8").read().split("\n"):
    m = re.match(r"//@@COPY (\S+) (\d+) (\d+)$", line)
    if not m:
        res.append(line)
        continue
    f, a, b = m.group(1), int(m.group(2)), int(m.group(3))
    source = open("src/" + f, encoding="utf-8").read().split("\n")
    res.append(f"// C++ copy: {f}:{a}-{b}")
    res.extend(source[a - 1 : b])
    res.append("// end of copy")
open(out, "w", encoding="utf-8", newline="\n").write("\n".join(res))

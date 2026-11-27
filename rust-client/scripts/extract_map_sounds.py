"""Write every embedded sound of the given maps to <out dir>/<map>_<index>.opus (PLAN §13.3,
the corpus of `rs_parity opus`). Maps are only read. Usage:

    python rust-client/scripts/extract_map_sounds.py <out dir> <map>...
"""

import os
import struct
import sys
import zlib


def sounds(path):
    b = open(path, "rb").read()
    _, ver, _, _, nt, ni, nd, isz, _ = struct.unpack_from("<4s8i", b, 0)
    types = [struct.unpack_from("<3i", b, 36 + 12 * i) for i in range(nt)]
    off = 36 + 12 * nt
    ioff = struct.unpack_from(f"<{ni}i", b, off)
    off += 4 * ni
    doff = list(struct.unpack_from(f"<{nd}i", b, off)) + [None]
    off += 4 * nd
    if ver == 4:
        off += 4 * nd
    istart, dstart = off, off + isz
    for t in types:
        if t[0] != 7:  # MAPITEMTYPE_SOUND
            continue
        for k in range(t[1], t[1] + t[2]):
            o = istart + ioff[k]
            _, size = struct.unpack_from("<2i", b, o)
            _, external, _, data = struct.unpack_from("<4i", b, o + 8)[:4]
            if external:
                continue
            end = dstart + doff[data + 1] if doff[data + 1] is not None else len(b)
            raw = b[dstart + doff[data] : end]
            yield k - t[1], zlib.decompress(raw) if ver == 4 else raw


out = sys.argv[1]
maps = [os.path.join(a, f) for a in sys.argv[2:] if os.path.isdir(a) for f in sorted(os.listdir(a))]
maps += [a for a in sys.argv[2:] if not os.path.isdir(a)]
for m in [m for m in maps if m.endswith(".map")]:
    name = os.path.splitext(os.path.basename(m))[0][:40].replace(" ", "_")
    for i, data in sounds(m):
        open(os.path.join(out, f"{name}_{i}.opus"), "wb").write(data)

"""Writes the PNG test corpus to rust-client/tests/golden/png/ (stdlib only).

Every PNG colour type and bit depth, tRNS, Adam7 interlacing, odd sizes and broken
files. `rs_parity png` loads them with the C++ CImageLoader and `tests/assets.rs`
compares the Rust loader with that. Deterministic: rerunning writes the same bytes.
"""

import os
import struct
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "tests", "golden", "png")

# Adam7 passes: (x start, y start, x step, y step)
ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
CHANNELS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}


def chunk(kind, data, crc=None):
    c = zlib.crc32(kind + data) if crc is None else crc
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", c & 0xFFFFFFFF)


def sample(x, y, c, depth):
    """A deterministic sample value for pixel (x, y), channel c."""
    return (x * 37 + y * 101 + c * 59 + x * y * 13) % (1 << depth)


def row_bytes(samples, depth):
    if depth == 16:
        return b"".join(struct.pack(">H", s) for s in samples)
    if depth == 8:
        return bytes(samples)
    bits = "".join(format(s, "0%db" % depth) for s in samples)
    bits += "0" * (-len(bits) % 8)
    return bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))


def image_data(w, h, color, depth, interlace, filter_byte=0):
    ch = CHANNELS[color]
    passes = ADAM7 if interlace else [(0, 0, 1, 1)]
    raw = b""
    for x0, y0, dx, dy in passes:
        xs = list(range(x0, w, dx))
        if not xs:
            continue
        for y in range(y0, h, dy):
            samples = [sample(x, y, c, depth) for x in xs for c in range(ch)]
            raw += bytes([filter_byte]) + row_bytes(samples, depth)
    return raw


def png(w, h, color, depth, interlace=0, trns=None, plte=None, filter_byte=0):
    ihdr = struct.pack(">IIBBBBB", w, h, depth, color, 0, 0, interlace)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
    if color == 3:
        n = plte if plte is not None else 1 << depth
        out += chunk(b"PLTE", bytes((i * 7 + k * 50) % 256 for i in range(n) for k in range(3)))
    if trns is not None:
        out += chunk(b"tRNS", trns)
    out += chunk(b"IDAT", zlib.compress(image_data(w, h, color, depth, interlace, filter_byte), 9))
    return out + chunk(b"IEND", b"")


def main():
    os.makedirs(OUT, exist_ok=True)
    files = {}
    depths = {0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16]}
    for color, ds in depths.items():
        for d in ds:
            files["c%d_d%d.png" % (color, d)] = png(7, 5, color, d)
            files["c%d_d%d_adam7.png" % (color, d)] = png(13, 11, color, d, interlace=1)
    files["c0_d8_trns.png"] = png(6, 4, 0, 8, trns=struct.pack(">H", 37))
    files["c0_d16_trns.png"] = png(6, 4, 0, 16, trns=struct.pack(">H", 4000))
    files["c2_d8_trns.png"] = png(6, 4, 2, 8, trns=struct.pack(">HHH", 37, 59, 101))
    files["c3_d8_trns.png"] = png(9, 9, 3, 8, trns=bytes(range(0, 250, 10)))
    files["c3_d4_trns_adam7.png"] = png(9, 9, 3, 4, interlace=1, trns=bytes([0, 128, 255]))
    files["c6_d8_1x1.png"] = png(1, 1, 6, 8)
    files["c6_d8_33x3.png"] = png(33, 3, 6, 8)
    files["c6_d8_paeth_filters.png"] = png(8, 8, 6, 8, filter_byte=4)

    good = png(7, 5, 6, 8)
    files["bad_signature.png"] = b"\x89PNX" + good[4:]
    files["bad_empty.png"] = b""
    for n in (8, 20, 33, 40, len(good) - 13, len(good) - 1):
        files["bad_truncated_%d.png" % n] = good[:n]
    ihdr_end = 8 + 25
    files["bad_ihdr_crc.png"] = good[:29] + b"\0\0\0\0" + good[33:]
    files["bad_idat_crc.png"] = good[:-16] + bytes([good[-16] ^ 0xFF]) + good[-15:]
    files["bad_no_iend.png"] = good[:-12]
    files["bad_trailing_data.png"] = good + b"garbage"
    files["bad_zero_width.png"] = png(0, 5, 6, 8)
    files["bad_zero_height.png"] = png(5, 0, 6, 8)
    files["bad_filter_type.png"] = png(4, 4, 6, 8, filter_byte=5)
    files["bad_depth_for_rgb.png"] = png(4, 4, 2, 8)[:16] + b"\x04" + png(4, 4, 2, 8)[17:]
    files["bad_palette_missing.png"] = png(4, 4, 3, 8, plte=0)
    files["bad_palette_short.png"] = png(4, 4, 3, 8, plte=3)
    for name, (w, h) in {"bad_wide": (1000001, 1), "bad_tall": (1, 1000001), "bad_huge": (1 << 31, 1)}.items():
        ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
        files[name + ".png"] = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(b"\0" * 5)) + chunk(b"IEND", b"")
    files["bad_idat_short.png"] = good[:ihdr_end] + chunk(b"IDAT", zlib.compress(b"\0" * 10)) + chunk(b"IEND", b"")
    files["bad_idat_garbage.png"] = good[:ihdr_end] + chunk(b"IDAT", b"not zlib at all") + chunk(b"IEND", b"")

    raw = image_data(7, 5, 6, 8, 0)
    z = zlib.compress(raw, 9)
    files["bad_adler32.png"] = good[:ihdr_end] + chunk(b"IDAT", z[:-4] + bytes(b ^ 0x55 for b in z[-4:])) + chunk(b"IEND", b"")
    files["c6_d8_extra_image_data.png"] = good[:ihdr_end] + chunk(b"IDAT", zlib.compress(raw + b"\0" * 40, 9)) + chunk(b"IEND", b"")
    files["c6_d8_split_idat.png"] = good[:ihdr_end] + b"".join(chunk(b"IDAT", z[i:i + 7]) for i in range(0, len(z), 7)) + chunk(b"IEND", b"")

    for name, data in sorted(files.items()):
        with open(os.path.join(OUT, name), "wb") as f:
            f.write(data)
    print("wrote %d files to %s" % (len(files), os.path.normpath(OUT)))


if __name__ == "__main__":
    main()

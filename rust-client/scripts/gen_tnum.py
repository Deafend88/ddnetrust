"""Makes the digits of the bundled Inter fonts tabular: the cmap entries of 0-9 point to
Inter's own tabular glyphs (`zero.tf` .. `nine.tf`, the targets of its `tnum` feature),
so every digit has the same advance and changing numbers in the HUD do not jitter (egui
cannot switch OpenType features on). Punctuation keeps its proportional glyphs.
Idempotent; Inter's licence (SIL OFL 1.1, no reserved font name) allows the change.

Usage: python rust-client/scripts/gen_tnum.py   (needs fontTools)
"""

import os

from fontTools.ttLib import TTFont

FONTS = os.path.join(os.path.dirname(__file__), "..", "assets", "fonts")
DIGITS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]

for name in [
	"Inter-Regular.ttf",
	"Inter-Medium.ttf",
	"Inter-SemiBold.ttf",
	"InterDisplay-Medium.ttf",
	"InterDisplay-SemiBold.ttf",
]:
	path = os.path.join(FONTS, name)
	font = TTFont(path)
	glyphs = set(font.getGlyphOrder())
	for table in font["cmap"].tables:
		if not table.isUnicode():
			continue
		for i, glyph in enumerate(DIGITS):
			if ord("0") + i in table.cmap and glyph + ".tf" in glyphs:
				table.cmap[ord("0") + i] = glyph + ".tf"
	font.save(path)
	widths = {font["hmtx"][font.getBestCmap()[ord(c)]][0] for c in "0123456789"}
	print(name, "digit advances:", widths)

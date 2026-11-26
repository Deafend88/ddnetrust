"""Generates the icon fonts and rust-client/client/src/icons.rs.

Icons are Material Symbols Rounded, filled (Apache 2.0), except where Phosphor Fill (MIT)
reads better (user review): the ghost (no collision), the horizontal barbell (practice)
and the player's previous / play / pause / next.
- assets/fonts/MaterialSymbolsRounded-Fill.ttf: a static instance of Google's variable
  font (FILL 1, wght 400, GRAD 0, opsz 24), subset to the icons below.
- assets/fonts/Phosphor-Extra.ttf: Phosphor 2.1.2 fill subset to its icons.
Both are in the `icons` font family (style.rs); their codepoints must not overlap.

Usage (needs fontTools):
  python rust-client/scripts/gen_icons.py <MaterialSymbolsRounded[FILL,GRAD,opsz,wght].ttf>
      <its .codepoints file> <Phosphor-Fill.ttf>
The Material files are from github.com/google/material-design-icons (variablefont/),
Phosphor-Fill.ttf from the npm package @phosphor-icons/web 2.1.2 (src/fill/).
Add icons to ICONS and run it again when a widget needs another icon.
"""

import os
import sys

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "rust-client", "client", "src", "icons.rs")
FONTS = os.path.join(ROOT, "rust-client", "assets", "fonts")

# (Rust constant, "material" or "phosphor", icon name). One meaning per icon: the
# comment says what it stands for.
ICONS = [
	("USER", "material", "person"),  # player name
	("FINGERPRINT", "material", "fingerprint"),  # client id
	("MONITOR", "material", "desktop_windows"),  # framerate
	("WIFI", "material", "wifi"),  # ping
	("GLOBE", "material", "public"),  # server address
	("CLOCK", "material", "schedule"),  # local time
	("GAUGE", "material", "speed"),  # speed
	("MAP_PIN", "material", "location_on"),  # position
	("CROSSHAIR", "material", "my_location"),  # aim angle
	("TIMER", "material", "timer"),  # race time
	("PULSE", "material", "monitor_heart"),  # server tick rate
	("CHECK_CIRCLE", "material", "check_circle"),  # finish
	("TROPHY", "material", "trophy"),  # record
	("FLAG_CHECKERED", "material", "sports_score"),  # race start/finish line
	("SKULL", "material", "skull"),  # kill
	("MEGAPHONE", "material", "campaign"),  # server broadcast
	("MUSIC_NOTES", "material", "music_note"),  # now playing
	("PLAY", "phosphor", "play"),  # media play
	("PAUSE", "phosphor", "pause"),  # media pause
	("SKIP_BACK", "phosphor", "skip-back"),  # previous track
	("SKIP_FORWARD", "phosphor", "skip-forward"),  # next track
	("VOLUME_OFF", "material", "volume_off"),  # muted
	("VOLUME_DOWN", "material", "volume_down"),  # low volume
	("VOLUME_UP", "material", "volume_up"),  # high volume
	("LYRICS", "material", "lyrics"),  # synced lyrics
	("GEAR", "material", "settings"),  # settings
	("CHECK", "material", "check"),  # done
	("EYE", "material", "visibility"),  # show / spectating
	("MOVE", "material", "open_with"),  # edit the HUD layout
	("RESET", "material", "restart_alt"),  # reset to defaults
	("CONNECT", "material", "cable"),  # connect
	("GAME", "material", "sports_esports"),  # back to the game
	("DISCONNECT", "material", "logout"),  # disconnect
	("POWER", "material", "power_settings_new"),  # quit
	("SNOWFLAKE", "material", "ac_unit"),  # deep freeze
	("FLAME", "material", "local_fire_department"),  # jetpack (a mark on the gun)
	("ARROW_FAT_LINES_UP", "material", "keyboard_double_arrow_up"),  # endless jump
	("SPARKLE", "material", "auto_awesome"),  # teleport weapon
	("PERSON", "material", "accessibility_new"),  # solo
	("GHOST", "phosphor", "ghost"),  # collision with others off
	("LOCK", "material", "lock"),  # lock mode
	("BARBELL", "phosphor", "barbell"),  # practice mode
	("USERS", "material", "group"),  # team (team 0 mode)
	("INFO", "material", "info"),  # information
	("CARET_UP", "material", "keyboard_arrow_up"),  # move up
	("CARET_DOWN", "material", "keyboard_arrow_down"),  # move down
	("SERVER", "material", "dns"),  # a server chat line
	("WHISPER", "material", "alternate_email"),  # a whisper
	("FRIEND", "material", "favorite"),  # a friend
	("CHAT", "material", "chat_bubble"),  # chat input
	("DIR_LEFT", "material", "arrow_back"),  # a player presses left
	("DIR_UP", "material", "arrow_upward"),  # a player jumps
	("DIR_RIGHT", "material", "arrow_forward"),  # a player presses right
	("HOOK_STRONG", "material", "arrow_circle_up"),  # their hook is stronger than ours
	("HOOK_WEAK", "material", "arrow_circle_down"),  # their hook is weaker than ours
	("SCOREBOARD", "material", "leaderboard"),  # scoreboard
]


def phosphor_codes(font):
	"""Phosphor icon name -> codepoint, from the fill font's `<name>-fill` ligatures."""
	cmap = font.getBestCmap()
	glyph_cp = {}
	for cp, glyph in cmap.items():
		glyph_cp.setdefault(glyph, cp)
	char = {glyph: chr(cp) for cp, glyph in cmap.items() if cp < 128}
	codes = {}
	for lookup in font["GSUB"].table.LookupList.Lookup:
		for sub in lookup.SubTable:
			sub = getattr(sub, "ExtSubTable", sub)
			for first, ligatures in getattr(sub, "ligatures", {}).items():
				for lig in ligatures:
					name = "".join(char.get(g, "?") for g in [first] + lig.Component)
					if lig.LigGlyph in glyph_cp:
						codes[name.removesuffix("-fill")] = glyph_cp[lig.LigGlyph]
	return codes


def subset_font(font, unicodes, out):
	"""Keeps only `unicodes` (no ligatures or layout), saved to `out`."""
	options = subset.Options()
	options.layout_features = []
	options.name_IDs = ["*"]
	options.notdef_outline = True
	sub = subset.Subsetter(options)
	sub.populate(unicodes=unicodes)
	sub.subset(font)
	font.save(out)


def main():
	material_ttf, material_points, phosphor_ttf = sys.argv[1:4]
	with open(material_points, encoding="utf-8") as f:
		material = {name: int(cp, 16) for name, cp in (line.split() for line in f if line.strip())}
	phosphor_font = TTFont(phosphor_ttf)
	phosphor = phosphor_codes(phosphor_font)
	codes = {
		const: (material if src == "material" else phosphor)[name] for const, src, name in ICONS
	}
	by_src = {
		src: sorted({codes[c] for c, s, _ in ICONS if s == src}) for src in ("material", "phosphor")
	}
	assert not set(by_src["material"]) & set(by_src["phosphor"]), "codepoints overlap"

	static = instancer.instantiateVariableFont(
		TTFont(material_ttf), {"FILL": 1, "wght": 400, "GRAD": 0, "opsz": 24}
	)
	subset_font(static, by_src["material"], os.path.join(FONTS, "MaterialSymbolsRounded-Fill.ttf"))
	subset_font(phosphor_font, by_src["phosphor"], os.path.join(FONTS, "Phosphor-Extra.ttf"))

	with open(OUT, "w", encoding="utf-8", newline="\n") as f:
		f.write("// Generated by rust-client/scripts/gen_icons.py. Do not edit.\n\n")
		f.write(
			"//! Icon glyphs: Material Symbols Rounded, filled (Apache 2.0), and some Phosphor fill\n"
			"//! icons (MIT); draw them with the `icons` font family (`style.rs`).\n\n"
		)
		for const, src, name in ICONS:
			label = "Material Symbols" if src == "material" else "Phosphor"
			f.write(f'/// {label} `{name}`.\npub const {const}: &str = "\\u{{{codes[const]:x}}}";\n')
		f.write(f"\n/// Every icon above, in order (the gallery shows them all).\npub const ALL: [&str; {len(ICONS)}] = [\n")
		for const, _, _ in ICONS:
			f.write(f"    {const},\n")
		f.write("];\n")
	print(len(ICONS), "icons")


if __name__ == "__main__":
	main()

# Decisions

Places where the Rust client deliberately differs from the C++ client, or where C++
behavior looks like a bug but is kept.

## 2026-11-21 — 0.7 packets are rejected on receive

C++: `CNetBase::UnpackPacket` (network.cpp:282). `CNetClient::Recv` passes a
`Sixup` flag by value, so a connected 0.6 client still parses any datagram whose
header marks it as 0.7 (connless with `buf[0] & 3 == 1`, or the `UNUSED` flag bit)
and feeds it to the connection. Real 0.6 servers never send these; in the golden
vectors they only come from random garbage (146 of 3,654 decode cases).

Rust: `protocol::packet::decode` returns `None` for them. 0.7 is out of scope
(PLAN §1), and dropping is the safe choice for unexpected input. Everything else
`decode` does matches C++ byte for byte.

## 2026-11-21 — `CUnpacker::GetString` returns a copy

C++ sanitizes the receive buffer in place and returns a pointer into it. Rust returns
a sanitized `String` and leaves the input untouched. No C++ caller reads the same
bytes again after `GetString`, so nothing observable changes.

## 2026-11-21 — C++ bug kept out: remote-controlled assert in `UnpackDelta`

C++: `CSnapshotDelta::UnpackDelta` (snapshot.cpp:513) bounds an item size only by the
remaining delta data (up to 128 KiB), then calls `CSnapshotBuilder::NewItemRaw`, whose
`dbg_assert` (snapshot.cpp:885) requires the size to fit a 64 KiB snapshot. A server can
therefore crash a C++ client with one crafted delta. This looks like a bug in C++.

Rust: `protocol::snapshot::unpack_delta` treats that case like the other "snapshot
full" case and returns `-302`, the code C++ would return if the assert were absent.
All non-crashing inputs behave identically (978 golden delta cases).

## 2026-11-21 — Validated ("alt") snapshot is a list, not a `CSnapshot`

C++: `CClient::UnpackAndValidateSnapshot` (client.cpp:2432) rebuilds the validated items
into a second `CSnapshot` and returns -4 if that builder runs out of space. Rust returns
a `Vec<Item>`. The builder can only overflow if validation grows the items past 64 KiB
(defaults filled in for short items); real snapshots are far below that, so the -4 path
is not reproduced.

## 2026-11-21 — `what-is` UUID queries are not answered

C++: `UnpackMessageId` (protocol_ex.cpp:21) answers `what-is@ddnet.tw` with `it-is` or
`i-dont-know`. No code in this repository sends `what-is` (grep: only the 0.7
translation layer mentions it), so the Rust client drops it. Add the answer if a
server is ever seen sending it.

## 2026-11-21 — In-band map download, verified by CRC, capped at the announced size

C++ downloads over HTTPS when the server sends `map-details` with a SHA-256 and then
verifies only the SHA-256; otherwise it downloads in-band and verifies the CRC32.
HTTP download is deferred (PLAN §8), so the Rust client always downloads in-band and
verifies the CRC32, i.e. it behaves like a C++ client talking to a server that sends
no `map-details`. C++ keeps appending `MAP_DATA` chunks to the file without a limit;
Rust disconnects when the data would exceed the size announced in `MAP_CHANGE`, so
a server cannot make it use unbounded memory.

## 2026-11-22 — Map image and quad layer items that are too short

C++ `CMapImages::OnMapLoadImpl` reads `CMapItemImage_v2::m_MustBe1` (int 6) of every
image with version > 1, and `CMapItemLayerQuads::m_Image` (int 6) of every quad layer,
but `CMap::Load` never checks that those items are that long, so a crafted map makes
the C++ client read past the item (into the next item's memory). Rust cannot
reproduce reading foreign memory: a too-short image item is not loaded
(`MapImage::Truncated`) and a too-short quad layer uses no image. No real map has
such items; the parity data marks these cases as `truncated` on both sides.

## 2026-11-22 — Storage: the C++ user directory is read, never written

C++ `CStorage` uses `$USERDIR` (`%APPDATA%\DDNet`) as its save path and creates its
folders there at start-up. DDNet-rs searches the same paths in the same order
(`storage.cfg` or `$USERDIR`, `$DATADIR`, `$CURRENTDIR`, same data-dir discovery, same
refusal of `../`, absolute and drive paths) but only reads them; what it writes
(screenshots, later settings) goes to its own `DDNet-rs` directory, so the real DDNet
client's files are never touched. Data-dir discovery additionally accepts
`--data-dir`/`DDNET_DATA_DIR` first and Steam's default install path last; the
repository's `data/` takes the place of C++'s compiled-in `DATA_DIR`.

## 2026-11-22 — Map rendering: what differs from C++ on purpose

- Map images the C++ client fails to load become its "null texture" (a debug pattern);
  DDNet-rs draws those layers untextured. Tilesets whose size is not divisible by 16
  are resized to powers of two by C++ before cutting them into tiles; DDNet-rs draws
  such tile layers untextured. Neither happens with the bundled maps.
- Mipmaps are built on the CPU with a 2x2 average (C++: GPU blits); results differ by
  at most rounding.
- Envelopes run on the C++ in-game clock (predicted tick and intra tick since the
  round start); C++ only uses local time for offline backgrounds, which DDNet-rs does
  not have.
- Reference screenshots come from the Vulkan backend (the default of DDNet's Windows
  builds). This repo's C++ client fell back to the legacy OpenGL 1.x path when built
  without Vulkan, which renders tiles differently; that path is not a reference.
- Running the stock C++ client for comparisons: even with `cl_show_welcome 1` it asks
  the master servers and info.ddnet.org on start; the comparison script blocks this
  with a dead local proxy (seen failing in its log), so it stays local.

## 2026-11-22 — PNG loading: png-rs set up to behave like libpng

`CImageLoader::LoadPng` uses libpng; DDNet-rs uses the `png` crate with the same
transformations and these adjustments, each checked against the C++ loader on 64
crafted files and the 546 PNGs in `data/` (`rs_parity png`): Adler-32 checking on
(libpng checks it, png-rs does not by default); libpng's default 1,000,000 pixel
width/height limit; and an `IEND` chunk appended to the input, because libpng stops
after the image data and accepts files without `IEND`, while png-rs reads the next
chunk header. The pnglite compatibility flags C++ computes only feed a warning and are
not reproduced. C++ truncates `mapres/<name>.png` at 511 bytes; Rust does not, which
only matters for names so long that neither file can exist.

## 2026-11-22 — Tees: what differs from C++ on purpose

- Quad containers (`RenderQuadContainerAsSprite`, the tee, hand, weapon, hook and
  emoticon quads) are drawn with the math of the buffered backends (Vulkan, GL3: rotate
  in container space, then scale, then move), not the unbuffered fallback (scale, then
  rotate). The reference screenshots come from Vulkan; the two differ for mirrored eyes
  and non-square scales.
- `CAnimState::Add` evaluates into an uninitialized temporary; DDNet-rs starts it at
  zero. Only a time outside every keyframe range would read it, and the C++ callers
  only pass times in [0, 1] (checked by `rs_parity anims`).
- Skins are loaded synchronously on first use from `skins/` and the C++ client's
  `downloadedskins/`; there is no skin download, so unknown skins are drawn as
  `default` (C++ downloads them from skins.ddnet.org first).
- Not drawn yet: particles (freeze flakes, sparkles of invincible tees, skid trails,
  powerup shine), muzzle flashes and the ninja hadoken (they pick a frame with
  `rand()`), emoticon messages (`Sv_Emoticon` is not handled), the hook collision line
  (only with the aim flag), the unpredicted shadow (default off), 0.7 skins.

## 2026-11-22 — Interpolation: simplifications

- C++ reuses last snapshot's evolved character when the snapped object did not change
  (OnNewSnapshot); DDNet-rs evolves again from the snapped tick, which gives the same
  core (evolution is deterministic).
- C++ keeps `m_PredictedWorld` between frames and requires the local character in it
  to draw it predicted; DDNet-rs draws it predicted once any prediction has run, which
  is the same condition after the first prediction.
- Antiping (`cl_antiping 1`: other players predicted and smoothed) is not ported; its
  default is off.

## 2026-11-23 — Entities: what differs from C++ on purpose

- Pickup bobbing and grenade spin run on a local clock in C++ (`s_Time`, from the
  first time they are drawn); ours runs from client start. Two clients never agree on
  it, so screenshot comparisons mask pickups.
- Every tune zone uses the default tuning until `Sv_TuneParams`/tune zone messages are
  handled (projectile curvature and speed, laser width timing).
- The antiping branches of CItems (predicted projectiles, lasers, pickups and smoke
  trail reconstruction) are not ported: they need `cl_antiping 1` and weapon
  prediction.
- SnapCollectEntities sorts with `std::sort` (unstable); ours is stable. Servers give
  every entity its own snapshot id, so the order is the same.
- Particles (bullet and smoke trails, powerup shine, explosions) are not drawn.

## 2026-11-24 — Input, camera and settings: what differs from C++ on purpose

- Settings: the C++ client's `settings_ddnet.cfg` is read once at start (first storage
  path that has it) and never written; then our own `%APPDATA%\DDNet-rs\settings.cfg`.
  Only our variables, `bind`, `unbind` and `unbindall` are executed; every other command
  (other clients' settings, `exec`, ...) is ignored. Colours are read in the packed form
  the C++ client saves; `$RRGGBB` and colour names are not supported yet.
- Binds run only the commands DDNet-rs implements (`+left`, `+right`, `+jump`, `+hook`,
  `+fire`, `+weapon1`-`5`, `+nextweapon`, `+prevweapon`, `zoom+`, `zoom-`, `zoom`); chat,
  emote, spectate, kill, vote and console binds do nothing yet.
- winit's synthetic key events on focus changes are ignored, like C++, which only
  clears its key states.
- The spectator camera (free view, auto-sync with the spectated player's camera,
  smoothing between targets) is not ported; while spectating the camera stays on the
  last position. The camera messages already keep their last values while spectating.
- `MaxZoomLevel` assumes tile buffering (240 with `cl_limit_max_zoom_level 1`), as with
  the Vulkan backend.
- `rs_parity camera` runs the C++ components outside a real CGameClient; the two
  functions it stubs (`CGameClient::IsWorldPaused`, `CScoreboard::IsActive`) are not
  reached on the exercised paths, and `cl_limit_max_zoom_level` is 0 there because the
  limit asks IGraphics.

## 2026-11-24 — Menus (M6.8): minimal on purpose

- egui, not a port of C++ `CMenus`: a start screen, the Esc menu and a settings page;
  the full design comes later. Menu input follows C++ (`SnapInput` with
  `PLAYERFLAG_IN_MENU`, binds while menus are open).
- Our `settings.cfg` holds every variable and bind, so after the first save it
  overrides the C++ `settings_ddnet.cfg` completely (C++ saves only non-default
  values, which would let a value set back to its default lose against the C++ file).
- Settings files: only the commands DDNet-rs uses are executed; others (stock settings
  we do not use, other clients' such as TClient's) are skipped with one summary line.
- Name and skin changes apply on the next connect; C++ sends `Cl_ChangeInfo` in game.
- `rs_data_dir` is our own variable (not C++): the data dir when neither `--data-dir`
  nor `DDNET_DATA_DIR` is given.
- `CHud::RenderPlayerState` (hud.cpp:842): the "gun hit disabled" indicator tests
  `m_GrenadeHitDisabled && m_HasTelegunGun` (and draws the laser indicator's quad), so it
  follows the grenade flag, not `m_GunHitDisabled` (which the core does not even have).
  Suspected C++ bug; the status bar (7.7) keeps the same decision.

## 2026-11-25 — Dynamic Island (M7.9) and HUD numbers

- The watermark and the island are separate widgets (a merge was tried in e9b529b29 and
  reverted at the user's request). The local clock lives in the watermark (on by
  default, first, then ping and fps); the island has only the race timer with the
  checkpoint difference, its status (pause / spectator text) and its events (music with
  7.10).
- Ported from C++: `RenderGameTimer`, `OnMessage`/`RenderDDRaceEffects`, `CBroadcast`,
  `RenderPauseNotification`, `RenderSpectatorHud`. C++'s own fades (finish 4-6 s,
  broadcast last 1 s) are replaced by the island's transition; the lifetimes (6 s, 10 s)
  are kept. C++ sizes the timer by its width class; tabular digits give the same.
- Events by priority: finish > broadcast > the idle timer. During a broadcast the timer
  stays at its end, but the pause or spectator status is hidden (C++ shows all).
  Broadcasts are trimmed (servers pad them with blank lines and spaces to place them on
  screen) and wrap at 480 pt.
- `Sv_RecordLegacy` (`m_DDRaceRecordMessage`, old DDRace servers) is not ported.
- Size springs are slightly underdamped (`Spring::update_damped`, zeta 0.75, ~3 %
  overshoot, settled in `motion::SPRING`); fades stay critically damped.
- Tabular digits: egui cannot switch OpenType features on, so `scripts/gen_tnum.py`
  points the digits 0-9 of the bundled Inter fonts at Inter's own `.tf` glyphs (OFL
  allows it; no reserved font name). Digits are tabular in all our UI text, menus too;
  punctuation stays proportional.
- Numbers in the HUD sit right-aligned in fields of a minimum width
  (`widgets::number` with a template): fps "0000", ping "000", uid "00", tps "00",
  local time "00:00", speed "-00.00"; movement position "0000.00", speed "-00.00",
  aim "-000.00". Longer values still grow the field.
- The watermark's fps changes at most every 0.5 s (C++ draws its average every frame).
  Position, speed and ping are not throttled (gameplay information, and the fields no
  longer jitter).

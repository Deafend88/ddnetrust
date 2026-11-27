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

## 2026-11-25 — Media player (M7.10)

- SMTC is polled every 0.5 s on its own thread (no WinRT event handlers): simpler, and a
  0.5 s delay is invisible for title/cover; the position moves on locally in between.
- Covers through Windows' `BitmapDecoder` instead of an image crate (platform first).
- The island's expanded player may send media commands (play/pause, next, previous,
  seek, source) to the OS media session; it never touches game input. The island area
  takes clicks only while the player is open (menu mode).
- The "⋯" menu offers the source only; "compact" and "hide" from SPEC §2 are covered by
  the idle mini player and the HUD editor's item toggle. The chosen source lasts for the
  run.
- Lyrics come from LRCLIB (lrclib.net) only when the user turns them on; the request
  carries the track's title and artist and a plain client name, nothing else. WinRT's
  HTTP client and JSON parser are used (no new crate). "None found" is cached.
- Large text uses Inter Display (Apple's SF fonts and symbols may not be embedded).
- `unsafe` exception (user decision 2026-11-25): per-app volume needs Windows Core Audio
  COM (`IAudioSessionManager2`, `IAudioSessionControl2`, `ISimpleAudioVolume`); no
  maintained crate wraps session volume safely (`wasapi` lists sessions only,
  `volumecontrol-windows` is device-wide). It lives in `client/src/audio.rs` alone,
  behind a safe API, with a SAFETY comment on every call. Rule: FFI to OS APIs only,
  when no safe crate exists.
- Icons: Material Symbols Rounded (filled) by the user's choice, with Phosphor's ghost
  and barbell where Material reads worse. Material glyphs are scaled 1.15 so the icon
  size tokens keep their optical size.
- Lyrics show in one place (subtitles under the island by default, the card, or off);
  the default fetches lyrics for the playing track (the user chose "under the island"
  as the default).

## 2026-11-25 — Antiping 8.1: messages the prediction world needs

- `Sv_PreInput` with a negative intended tick is dropped. C++ indexes its 200-slot ring
  with `Tick % 200`, which is negative then (out of bounds, undefined behaviour).
- The received tuning (`m_aTuning`) lives in the predictor, so it starts at the default
  for every new map/connection. C++ keeps it in `CGameClient` across connections; the
  server sends the tuning on every join, so only the first snapshots before that could
  differ.
- `mapbug` map settings are ignored: map bug emulation (`CMapBugs`) is not ported.
- `str_tofloat` follows `strtod` for hex floats (`0x1.8p3`), `nan(...)` and the empty
  string (0). ponytail: hex mantissas are exact up to 15 significant digits.
- C++ client prediction does not model the server's practice-mode death tiles (freeze
  instead of death), so it mispredicts the tick a practice tee touches one; so do we.

## 2026-11-25 — Antiping 8.2: collision for weapons

- `CLaser::DoBounce` tests `Res == -1` around a temporary `SetCollisionAt`, but the
  prediction calls `IntersectLineTeleWeapon` without `pTeleNr`, so it returns a tile
  index (1..=TILE_NOLASER) or 0, never -1: that branch is dead in C++. It is not ported,
  so the game tile grid stays immutable (no `Cell`, unlike PLAN §12.3 assumed).
- `intersect_line_tele_weapon` is the prediction's call without `pTeleNr`, which skips
  the tele lookup and is the same loop as `IntersectLine`.

## 2026-11-26 — Antiping 8.3: weapons and entities in the prediction world

- `sv_hit`, `sv_deepfly` and `sv_dragger_range`, which the C++ prediction reads from
  `g_Config`, are constants with their defaults (1, 1, 700): servers never send them, so
  a C++ client always uses its own defaults.
- Predicted events (sounds, explosions, hammer hits, damage indicators) are not created:
  C++ drops them unless `cl_predict_events` is on (default 0), and DDNet-rs has no
  sounds or particles yet. They change no physics.
- The gun's `new CProjectile(..., 0, -1)` passes SoundImpact 0 and Layer -1 (its
  comments say Force 0, SoundImpact -1); the layer -1 is kept.
- `m_GameTeam` comes from the snapshot's PlayerInfo team in team games; C++ keeps a
  client's last known team when its PlayerInfo is missing, DDNet-rs uses 0 then (only
  FNG's melt hammer reads it).
- CEntity leaves `m_Number`/`m_Layer` of fired lasers uninitialized; nothing reads them,
  so the Rust laser has no such fields.

## 2026-11-26 — Antiping 8.4: snapshot entities into the prediction world

- `GameWorld::copy_world` is C++ `CopyWorld`: a clone without the doors (C++ copies no
  doors). The parent/child links C++ sets only feed the smoke-trail reconstruction
  (`m_DestroyTick`), which is not ported.
- `find_match` leaves out the character case of C++ `FindMatch`: only the renderer's
  entity lookups use it.
- `IsLocalTeam` without a local player (-1) indexes the C++ teams array out of bounds;
  DDNet-rs treats every owner as local then (as the door code already did).

## 2026-11-26 — Antiping 8.5: prediction loop and settings

- `CClient::GetSmoothTick` calls `std::clamp(v, GameTime, PredTime)`, which is undefined
  when the predicted time is behind the game time. DDNet-rs follows MSVC's STL (the
  reference build: the upper bound wins); libstdc++ lets the lower bound win.
- `IsWorldPaused()` is paused *or game over* (C++ `GAMESTATEFLAG_GAMEOVER |
  GAMESTATEFLAG_PAUSED`); DDNet-rs only checked paused until now (fixed: `world_paused`,
  and `Predict()` through it). `OnPredict` itself only stops for paused, as in C++.
- `OnPredict` while paused only copies the snapshot's local character into
  `m_PredictedChar`, which only the ghost reads (not ported); DDNet-rs returns without it.
- `OnNewSnapshot` reads every `DDNetCharacter` of a snapshot (active or not) into
  `m_aClients[i].m_Predicted` and the solo state; DDNet-rs did this only for characters
  present in both snapshots (fixed). Negative ids (C++ out-of-bounds write) are skipped.
- `m_aClients[i].m_Predicted`/`m_PrevPredicted` start as a `CCharacterCore::Reset` core
  (`CClientData::Reset`); fields `Reset` leaves unset are uninitialized in C++ and never
  read before a prediction writes them. The parity dump leaves them out.
- The `m_aPredPos` ring is indexed with `tick.rem_euclid(200)`: C++ `Tick % 200` is
  negative (out of bounds) for negative ticks, which a server never sends.

## 2026-11-26 — Antiping 8.6: smoothing

- Suspected C++ bug, kept: the `cl_antiping_smooth` block sizes a window as
  `Len = 1000 * PredTime * SmoothPace` with `PredTime` in ms (µs units), but compares it
  with `time_get()` deltas in ns (`time_freq()` is 1e9). Windows therefore last at most
  ~3.2 ms instead of up to 3.2 s; the 700 ms / 300 ms caps (in ns) never bind, and a
  window has always ended by the next snapshot frame. Measured live: the largest
  per-frame step of a mispredicted tee is the same with smoothing on and off.
- C++ `std::min`/`std::clamp` on floats are ported with their NaN behaviour
  (`b < a ? b : a`), not `f32::min`. Float to `int64_t` casts of NaN or out-of-range
  values (C++ UB) saturate in Rust; they need a NaN position to happen.

## 2026-11-26 — Antiping 8.7: rendering

- `m_NinjaJetpack` is only set by the vanilla `CCharacter::Read` path (no
  `DDNetCharacter`), which is not ported, so the gunfire branches that read it in
  `UpdateRenderedCharacters` see false.
- `m_IsPredictedLocal` of a client that left the snapshot keeps its old value in C++;
  the item renderer reads it for lasers' owners. DDNet-rs only knows it for active
  clients (`Rendered::predicted_local`), so a laser whose owner just left is drawn from
  the snapshot instead of not at all. Only the local tee is ever predicted-local.
- `CItems` writes `m_LastRenderTick` into the server-state world's projectiles; only the
  smoke-trail reconstruction reads it (particles, not ported), so the renderer keeps the
  world read-only.
- Fixed on the way: a door laser's head time is the plain game tick in C++ (no intra
  tick added); DDNet-rs added `IntraGameTick`.

## 2026-11-27 — M9 13.1: chat

- The sent-chat history (C++ `CStaticRingBuffer<CHistoryEntry, 64 KiB, FLAG_RECYCLE>`)
  recycles by a byte budget with the ring's block cost (entry + 24-byte header, rounded
  to 24) instead of its block layout (`ponytail:` in `chat.rs`). They differ only after
  ~64 KiB of sent chat, in which old lines Up still reaches; the parity data stays below.
- Not ported: the desktop notification on a highlight (`Client()->Notify`), streamer
  mode's censoring of `/login`, `/save` etc. (`cl_streamer_mode`), the chat censor (off
  in C++ too), `cl_chat_old` and `cl_chat_size` (sizes come from our type scale, CLAUDE.md
  UI rules; `cl_chat_width` and the C++ height limits are kept), video recording.
- Chat sounds are decided exactly like C++ (rate limit, switches) and dropped until
  sound support (13.3).
- Look (user decision 2026-11-27): kind icons (server, client echo, whisper, team, friend)
  plus subtle tints of the C++ `cl_message_*_color` values and name colours; highlights
  use our accent instead of `cl_message_highlight_color`. Each line is a pill, a wrapped
  line a rounded rectangle. Lines wrap under the name, not after it as in C++.
- `ddnet-saves.txt` (`StoreSave`) is written to our own user dir, never DDNet's.
- Friends and foes (`add_friend`, `add_foe`) are read from `settings_ddnet.cfg` and kept
  in our `settings.cfg`; `IsFriend` compares strings without C++'s hashes (same result).
- The client table (`ddnet::clients`, C++ `m_aClients`) keeps a name from a `ClientInfo`
  without `PlayerInfo` forever, as C++ does (only active clients are reset).
- Paste needs egui-winit's `clipboard` feature (`arboard`); C++ chat pastes through SDL.
- `rs_parity chat`: 2 of 14 hand-made mutations are unobservable by construction: the
  Shift+Tab `size_t` wrap (every command is tried before the index can wrap) and the
  custom-colour check of repeat folding (lines that differ in it already differ in their
  client id).

## 2026-11-27 — M9 13.3: sound

- WavPack: our own decoder (`client/src/wavpack.rs`), a port of the libwavpack 5.9.0 code
  `CSound::DecodeWV` reaches (ddnet-libs has 5.9.0): single-stream mono/stereo, lossless and
  hybrid lossy without a correction file, every decorrelation term, joint/false stereo,
  shifts, extended-integer fixups, the muting of bad blocks, CRC checks, junk skipping and
  the length from the file end (`seek_eof_information`). Identical to libwavpack on every
  `data/audio/*.wv` and 4 damaged variants of each (655 cases, `rs_parity wv`).
- Not ported (the decode fails where C++ might return samples): multi-stream layouts (a
  block without FINAL_BLOCK; only >2-channel files have them, which DDNet refuses
  anyway), float and DSD data (DDNet refuses float through its 16-bit check unless the
  file is malformed), the extended 32-bit `wvx` stream, legacy (pre-4.0) files.
- C++ callocs whatever length the header claims (up to INT_MAX frames) and crashes when
  that fails; our allocation is fallible, so such a file fails to load instead.
- Opus (map sounds): `opus-pure` 0.2.5 (pure Rust, Ogg demuxer included) decodes; on top
  `client/src/opus.rs` ports what opusfile's `op_read` does in float builds: libopus
  `opus_pcm_soft_clip` per packet, then `op_float2short_filter` (triangular dither with IIR
  noise shaping, PRNG seeded with the stream serial, muting after silence, `lrintf`).
  A float decoder cannot be bit-exact with libopus 1.3.1 on x86, so the tolerance (§13.13
  answer 1) is: lengths, channels and loop starts exact; samples within 9 steps of 16 bits
  (peak -71 dBFS), mean difference at most 1 step. Measured on all 60 embedded sounds of
  `data/maps` and the user's 303 downloaded maps (`rs_parity opus`, 143M samples): all
  lengths/channels/loop starts identical, worst 9, mean 0.64, 66% of samples identical.
  The committed test checks Sunny Side Up's sound (4 windows) against opusfile.
- A damaged Ogg page (CRC) fails the whole Opus sound; opusfile resyncs and plays the
  rest. A panic inside the third-party decoder is caught and is a failed decode (map
  sounds come from servers).
- `LOOP_START=` is parsed with Rust's integer parse (C++ `str_toint`); they differ only on
  odd spellings such as a leading `+`.
- Device: cpal (WASAPI) at the device's rate (SDL opens with ALLOW_FREQUENCY_CHANGE). The
  callback pulls `Mix` in SDL's fixed chunks of `snd_buffer_size` frames, so looping
  voices restart at the same frames as in C++. Stereo goes into the first two device
  channels, mono devices get the mean. Not ported: reopening the device after it is
  unplugged (C++ `UpdateDevice`) and `AdvancePlayback` (voices advancing without a
  device): without a device nothing plays and nothing advances.
- Sound sets always load on a thread (C++ `cl_threadsoundloading`, default 1); the random
  pick of `GetSampleId` uses our own generator (C++ `rand()`), never the last sample.
- Predicted events (`cl_predict_events`, DDNet servers set `PREDICT_EVENTS`): the world
  creates every event of the C++ prediction world (sounds, hammer hits, explosions, damage
  indicators; `rs_parity weapons` now dumps them: identical on all maps). Only the sounds
  and hammer hits are played; explosions and damage indicators feed particles (not ported).
- `ProcessEvents` runs inside the predictor's snapshot handling (before the prediction, as
  C++ `OnNewSnapshot` before `OnPredict`), so a server event is checked against the
  previous frame's predicted events.
- No dummy: the other-players air-jump check skips only the predicted local tee.
- The map-sound warning is printed to stderr (C++ shows a warning popup).


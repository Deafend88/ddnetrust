# DDNet-rs — Plan

Goal: a Rust client that is wire- and physics-identical to the C++ DDNet client in this
repo (DDNet 20.3, `DDNET_VERSION_NUMBER 20030`, `src/game/version.h`). The C++ code is
the spec. This document covers architecture, the C++ → Rust mapping, verification per
milestone, and the main risks. Open questions for review are at the end.

---

## 1. Scope

In scope (milestones 1–6): 0.6 protocol + DDNet extensions (UUID messages/objects,
`clientver`, `capabilities`, `map-details`, ping-ex), in-band map download, DDNet
datafile maps incl. game/front/tele/speedup/switch/tune layers, character physics +
client prediction, then rendering/input/basic UI.

Out of scope until asked: 0.7 / "sixup" (all `IsSixup()` branches are skipped), dummy,
rcon beyond the one `crashmeplx` line the C++ client sends, demos, server browser,
editor, HTTP map download, STUN, websockets, ghosts, sound.

---

## 2. Architecture

```
rust-client/
  Cargo.toml                workspace; one member now, a second (renderer) at M6
  ddnet/                    library crate + headless binary
    src/
      lib.rs
      protocol/             pure, no I/O, no clock, no RNG
        varint.rs           compression.cpp (CVariableInt)
        packer.rs           packer.cpp (CAbstractPacker / CUnpacker)
        huffman.rs          huffman.cpp
        packet.rs           network.cpp: packet header, chunk header, connless,
                            security token, compress-if-smaller
        uuid.rs             uuid_manager.cpp + protocol_ex.cpp (UnpackMessageId)
        snapshot.rs         snapshot.cpp: CSnapshot, delta unpack, builder, CRC, storage
        generated.rs        generated from datasrc/network.py (see §4), committed
      net/
        conn.rs             network_conn.cpp + network_client.cpp as a sans-IO
                            state machine: feed(datagram, now) / update(now) -> datagrams
        client.rs           client.cpp subset: handshake, map download, snapshot
                            assembly, input sending, disconnect
      map/
        datafile.rs         datafile.cpp reader (v3/v4, zlib data)
        layers.rs           layers.cpp + the mapitems.h structs it needs
      physics/
        vmath.rs            base/vmath.h + base/math.h subset (own vec2, exact formulas)
        tuning.rs           tuning.h + CTuneParam/CTuningParams
        collision.rs        collision.cpp
        core.rs             gamecore.cpp (CCharacterCore, CWorldCore)
        teams.rs            teamscore.cpp
        world.rs            client/prediction/gameworld.cpp
        character.rs        client/prediction/entities/character.cpp
                            (other prediction entities later, as needed)
    src/bin/ddnet-headless.rs
    tests/                  golden.rs, parity.rs, server.rs
  tests/golden/               golden vectors + parity dumps produced by C++ tools
  scripts/gen_protocol.py   datasrc/network.py -> ddnet/src/protocol/generated.rs
  docs/PLAN.md, PROGRESS.md, DECISIONS.md (created on first entry)
```

Design rules:

- **One crate until rendering.** Modules, not crates. The renderer becomes a second
  crate at M6 so `wgpu`/`winit` never touch protocol/physics build or test times.
- **Sans-IO core.** Protocol and connection code take time (`now: i64`, same units as
  `time_get()`) and randomness (tokens, connection UUID) as inputs and return bytes. The
  socket loop is a thin `std::net::UdpSocket` (non-blocking) wrapper. This makes whole
  packets reproducible in tests, which "byte-identical to C++" requires.
- **No async runtime.** One UDP socket and a 50 Hz loop don't need tokio.
- **Errors, not panics, on network input.** Every `dbg_assert` on a sender-side
  invariant stays an assert; every receive path returns `Result`/`Option`. Fuzz-style
  tests feed truncated/garbage packets to `packet.rs`, `huffman.rs`, `snapshot.rs`.
- **Behavior ported verbatim.** Same order of float ops, same casts, same constants,
  same quirks; each ported function carries `// C++: path:line`.
- **Assets** (M6) are loaded from the user's DDNet data dir at runtime; nothing copied.

Planned dependencies (minimal; each checked via context7 before use):
`flate2` (datafile zlib + map CRC32 via its `crc32fast`), `getrandom` (tokens, UUIDs).
M6 adds `winit`, `wgpu`, `egui` (+ `egui-wgpu`, `egui-winit`) and `png` (§9.1). Nothing else unless a milestone proves
the need. Checked rung 2: the existing Rust in this repo (`src/base/*.rs`,
`src/engine/**/*.rs`) is a cxx bridge for console/config; nothing reusable.

---

## 3. C++ → Rust mapping

| C++ source | Rust | Notes |
|---|---|---|
| `engine/shared/compression.{h,cpp}` | `protocol/varint.rs` | ESDDDDDD EDDDDDDD…, sign via `~i`; max 5 bytes; masks `0x7F,0x7F,0x7F,0x0F` on unpack |
| `engine/shared/packer.{h,cpp}` | `protocol/packer.rs` | 2048-byte buffer, sticky error flag, UTF-8 re-encode with U+FFFD, `Limit`/truncation, `GetString` sanitize flags (`str_sanitize`, `str_sanitize_cc`, skip whitespace from `base/str.cpp`) |
| `engine/shared/huffman.{h,cpp}` | `protocol/huffman.rs` | freq table verbatim; tree built with **stable** sort desc by freq; 10-bit decode LUT; EOF symbol 256 |
| `engine/shared/network.{h,cpp}` | `protocol/packet.rs` | 3-byte header (`flags<<2 \| ack>>8`, ack, num_chunks); chunk header split 4 (2 or 3 bytes); connless `0xFF×6` / `xe`+4 extra; token appended *before* compression; compressed only if `0 < size < raw`; `IsSeqInBackroom`; `IsValidConnectionOrientedPacket` |
| `engine/shared/network_conn.cpp`, `network_client.cpp` | `net/conn.rs` | states OFFLINE/CONNECT/PENDING/ONLINE/ERROR (no WANT_TOKEN: 0.7 only); `TKEN` magic; resend ring buffer (32 KiB budget); ack window check; resend after 1 s; flush after 500 ms; keepalive after 1 s; connect retry every 500 ms; `conn_timeout` 100 s |
| `engine/shared/uuid_manager.cpp`, `protocol_ex*.{h,cpp}` | `protocol/uuid.rs` | UUIDs are MD5(namespace ‖ name) v3; computed at codegen time, so no md5 dependency at runtime; WHATIS → ITIS/IDONTKNOW answers |
| `engine/shared/protocol.h`, `engine/message.h` | `protocol/generated.rs` + small hand-written `Msg` packer | msg id `(id<<1)\|sys`; ex messages `0`/`1` + 16-byte UUID |
| `engine/shared/snapshot.{h,cpp}` | `protocol/snapshot.rs` | item key `type<<16\|id`; delta = deleted keys, updates with wrapping add; static item sizes; error codes kept (-101…-302); CRC = wrapping sum of item ints; `IsValid`; extended (UUID) item types via type-0 items with id ≥ 0x4000 |
| `engine/client/client.cpp` (`SendInfo`, `SendReady`, `SendEnterGame`, `SendMapRequest`, `SendInput`, `ProcessServerPacket`, `UnpackAndValidateSnapshot`, `PumpNetwork`) | `net/client.rs` | multi-part snap assembly (64 parts × 900 B), ack tick, CRC error counter, delta tick purge, input ring of 200 |
| `engine/client/smooth_time.cpp` | `net/client.rs` (M5) | game time / predicted time smoothing; needed for prediction tick and input timing |
| `game/client/gameclient.cpp` (`OnConnected`, `SendInfo`, ISDDNETLEGACY, `OnSnapInput`, `OnNewSnapshot`, `OnPredict`, `UpdatePrediction`) | `net/client.rs` (M3), `physics/world.rs` glue (M5) | only the network-visible and prediction parts |
| `datasrc/network.py`, `datasrc/datatypes.py`, `datasrc/compile.py` | `scripts/gen_protocol.py` → `generated.rs` | enums/flags, NetObj structs as `[i32; N]` layouts, msg pack/unpack, `SecureUnpackObj` range checks, static sizes, ex UUIDs |
| `engine/shared/datafile.{h,cpp}` | `map/datafile.rs` | header v3/v4, swaplen fix-up, item types, item offsets, zlib data; little-endian only (we read with `from_le_bytes`, so no swap path) |
| `engine/shared/map.cpp` (tile layer loading/validation) | `map/mod.rs` | tilemap data size checks, legacy tele/speedup/switch/tune handling |
| `game/layers.{h,cpp}`, `game/mapitems.h` | `map/mod.rs` | the last layer with each flag wins, as in C++ (it assigns on every match); `CTile`, `CTeleTile`, `CSpeedupTile`, `CSwitchTile`, `CTuneTile` layouts |
| `base/vmath.h`, `base/math.h` | `physics/vmath.rs` | own `Vec2` (no glam: its normalize differs in op order); `normalize` = `l = 1/len; (x*l, y*l)`; `round_to_int`, `round_truncate`, `mix`, `closest_point_on_line` |
| `game/tuning.h`, `CTuneParam` (gamecore.h) | `physics/tuning.rs` | values stored as `int` hundredths; read back as `/100.0f` — must be reproduced, it changes physics |
| `game/collision.{h,cpp}` | `physics/collision.rs` | `MoveBox`, `TestBox`, `IsOnGround`, `IntersectLine*`, move restrictions, tele/speedup/switch/tune lookups, `std::map` → `BTreeMap` |
| `game/gamecore.{h,cpp}` | `physics/core.rs` | `Tick`, `TickDeferred`, `Move`, `Write/Read/Quantize`, `SaturatedAdd`, `VelocityRamp`; `std::set` → `BTreeSet` |
| `game/teamscore.{h,cpp}` | `physics/teams.rs` | `CanCollide`, `CanKeepHook`, team super |
| `game/mapbugs.{h,cpp}` | `physics/world.rs` | map-specific bug emulation flags read by prediction |
| `game/client/prediction/gameworld.cpp` | `physics/world.rs` | entity tick order: per type PreTick/Tick, then TickDeferred, then switch timers; no PRNG on client → `RandomOr0` always 0 |
| `game/client/prediction/entities/character.cpp` | `physics/character.rs` | `DDRaceTick`, `HandleTiles` (freeze, tele, speedup, switch, stoppers, jetpack, ninja), `DDRacePostCoreTick`, `HandleWeapons` subset needed for movement (hammer/gun/shotgun/grenade/laser later) |
| `game/client/prediction/entities/{projectile,laser,pickup,door,dragger,plasma}.cpp` | `physics/*.rs` | ported only when a parity scenario needs them (M4/M5) |

---

## 4. Protocol code generation

`datasrc/network.py` is the single source for message/object layouts. Hand-porting ~100
messages and objects would drift. `scripts/gen_protocol.py` imports `datasrc/network.py`
and `datasrc/datatypes.py` unchanged (no edits to `datasrc/`) and emits one
`generated.rs` containing, in the same order as `compile.py`:

- enums/flags (`EMOTE_*`, `CHARACTERFLAG_*`, `GAMEINFOFLAG_*`, …) and `INPUT_STATE_MASK`;
- `NETOBJTYPE_*` / `NETMSGTYPE_*` ids, ex-type UUIDs (MD5 v3, computed with `hashlib`);
- each NetObject as a `#[repr(C)]` struct of `i32` fields plus `from_ints`/`as_ints`;
- `secure_unpack_obj` with the same range clamping as the generated `CNetObjHandler`
  (out-of-range fields are clamped, short items without defaults are dropped);
- pack/unpack for every NetMessage the client sends or handles.

The output is committed so `cargo build` needs no Python. Regenerate with
`python rust-client/scripts/gen_protocol.py` whenever `datasrc/` changes.

---

## 5. Verification per milestone

All C++-produced fixtures are generated once by small C++ tools in this repo, committed
under `rust-client/tests/golden/`, and regenerated by a documented command. `cargo test`
never needs a C++ build.

### M2 — primitives
- New tool `src/tools/rs_golden.cpp` (+1 line in the `TOOLS_SRC` list in
  `CMakeLists.txt`). Writes text fixtures (hex) for:
  - varint pack/unpack over edge values (0, ±63, ±64, ±2^k±1, INT_MIN, INT_MAX) and
    truncated inputs;
  - packer strings: ASCII, multibyte, invalid UTF-8, limits that truncate mid-codepoint,
    buffer-full errors; unpacker sanitize flags;
  - Huffman compress/decompress of a corpus (empty, 1 byte, all 256 bytes, real
    captured chunks), plus bad input that must return -1;
  - chunk headers (vital/non-vital, sizes 0..1023, sequences 0..1023);
  - full `SendPacket` output for constructed packets with a fixed security token
    (control, compressible, incompressible, resend flag);
  - snapshot: delta unpack + CRC for builder-made snapshots, including extended items.
- Rust `tests/golden.rs` checks every vector byte-for-byte; malformed-input tests assert
  no panic.

### M3 — headless client
- Build `DDNet-Server` from this repo (CMake + MSVC; see risks).
- `tests/server.rs`, run when `DDNET_SERVER` points at the binary (otherwise the test
  reports itself skipped): start server on a random local port with `coverage.map`,
  connect the Rust client, assert: CONNECTACCEPT → online, CLIENTVER+INFO sent, map
  downloaded in-band and CRC matches, CON_READY → StartInfo, READYTOENTER → ENTERGAME,
  ≥ 2 snapshots decoded with matching CRC, inputs acknowledged (INPUTTIMING received),
  server log shows our client entered with version string `DDNet-rs 0.1.0`, clean
  CTRL_CLOSE and server logs the drop reason.
- Wire check: a capture of the C++ client's handshake from this repo vs ours for the same
  injected token/UUID — bytes equal except the version string and fields derived from it.

### M4 — map + physics parity
- New tool `src/tools/rs_parity.cpp` (+ `TOOLS_SRC` entry and a `target_sources` block
  adding `src/game/client/prediction/*` and `game/client/{laser,projectile,pickup}_data.cpp`,
  the same pattern `map_render` already uses). It loads a map, builds a prediction
  `CGameWorld`, spawns N characters, feeds per-tick inputs, and dumps per tick:
  input, `CNetObj_CharacterCore` fields, raw `f32` bits of pos/vel/hook pos/dir, jumped,
  freeze start/end, tele/speedup-affected state, switch states.
- Maps (already in `data/maps/`, loaded from there, not copied): `coverage.map`
  (all six game layers), `LearnToPlay.map`, `Gold Mine.map`, `Tutorial.map`, `Tsunami.map`,
  `ctf1.map`/`dm1.map` (vanilla physics).
- Scenarios: scripted (hold hook in each direction, wall jumps, freeze entry/exit,
  every tele type, speedups, switch open/close/timed, tune zones, two-player hook
  and collision) plus seeded random input walks of several thousand ticks.
- `tests/parity.rs` replays the inputs and requires bit-identical state every tick.
  Required: 100%.

### M5 — prediction
- Port the client loop pieces: `CSmoothTime`, pred tick computation, input buffer,
  `OnPredict` (rewind to last snapshot, replay stored inputs), `ReadDDNet`/`Read` from
  snapshot, anti-ping limited to what local-player prediction needs.
- Verified by: (a) parity harness extended to "snapshot → predict k ticks" comparing
  with the C++ prediction world fed the same snapshot+inputs, (b) live run against the
  local server measuring zero prediction mismatches for the local player on scripted
  movement (log every tick where predicted ≠ received core).

### M6 — rendering, input, UI
See §9 (plan for review, 2026-11-22).

---

## 6. C++ changes (kept minimal)

| Change | Why |
|---|---|
| `src/tools/rs_golden.cpp` + 1 line in `TOOLS_SRC` | M2 fixtures |
| `src/tools/rs_parity.cpp` + `TOOLS_SRC` line + prediction sources for that tool | M4/M5 fixtures |
| `rs_parity` modes for M6 (png, mapimages, envelopes, anims, camera/mouse math, projectiles); `rs_parity` added to the CMake regex of tools that link `engine-gfx` (the PNG loader); `game/map/render_map.cpp`/`animstate.cpp` in its sources if needed | M6 fixtures (§9.4) |
| A C++ client build (`CLIENT=ON`, `VULKAN=ON`) in `build-rs-client`, no source change; `render_map.cpp` compiled into `rs_parity` | M6 reference screenshots and envelope parity (§9.4) |

No changes to engine/game code. Each is explained in its commit message.

---

## 7. Risks

1. **Float determinism.** Physics is `float` with `sqrt`, `atan2`, `pow`. Rust `+ - * /
   sqrt` are IEEE-exact, same as MSVC/x64 SSE2. `atan2f`/`powf` come from the platform
   libm in both; on Windows both resolve to the UCRT. The parity harness runs both on
   the same machine to prove it. Rust never contracts to FMA; C++ builds don't either
   on baseline x86-64 (no `/fp:contract`, no `-march` in `CMakeLists.txt`), but C++
   ARM64 builds may — so parity is only claimed for x86-64.
2. **Float→int casts.** C++ `(int)f` out of range is UB; MSVC/x64 yields `INT_MIN`
   (cvttss2si). Rust `as i32` saturates. Use one helper `trunc_i32_x86(f)` that returns
   `i32::MIN` for NaN/out-of-range at every C++ cast site (e.g. `m_HookTick =
   (float)…`, `int End = Distance + 1`, `m_Angle`). Decision logged in DECISIONS.md.
3. **Integer overflow.** C++ relies on wrapping through `unsigned` casts (snapshot
   diff, CRC sum). Use explicit `wrapping_*`; debug-build overflow panics would
   otherwise turn server garbage into crashes.
4. **Prediction code depends on globals.** `g_Config` values (`cl_antiping_players`,
   …), `CWorldConfig`, `CMapBugs`, game-info flags. Port as explicit parameters with
   C++ defaults; harness fixes the same values on both sides.
5. **Which C++ is "the truth" for physics.** The client prediction world and the server
   `CCharacter` are separate implementations; known differences include random tele
   outs (server uses `CPrng`, client always picks index 0). See question 1.
6. **Public servers run older DDNet versions.** Physics/protocol changes between
   versions (e.g. the `MoveBox` early-break in this tree) mean "identical to this repo"
   ≠ "identical to every public server". We match this repo; mismatches against older
   servers are handled by the same game-info flags the C++ client uses.
7. **Huffman tree ties.** Construction depends on `std::stable_sort` with a strict
   comparator; Rust's `sort_by` is stable too. Golden vectors cover all 257 symbols.
8. **Hidden randomness in packets.** Security token, connection UUID, timeout seed.
   Injected from outside so tests are deterministic.
9. **Map download.** The C++ client prefers HTTPS (`maps.ddnet.org` by SHA256) when the
   server sends `map-details`; we do in-band only. Functionally fine, but servers see a
   different request pattern for maps they serve via HTTP. See question 3.
10. **Building the C++ side.** No CMake/compiler on PATH; Visual Studio 18 Community
    (MSVC 14.51, bundled CMake) is installed and `ddnet-libs/` is present, so builds
    must run from a VS developer environment. First build may need fixes to toolchain
    detection, not to code.
11. **Nested Cargo workspace.** The repo root already has a `[workspace]`
    (`Cargo.toml`). `rust-client/Cargo.toml` is its own workspace root, which Cargo
    accepts; the root `.cargo/` config (`+crt-static` on MSVC) also applies to us,
    which is harmless. Verified in M2.

---

## 8. Review decisions (2026-11-21)

1. **Physics reference:** the C++ client prediction code
   (`src/game/client/prediction/`). The server `CCharacter` may be shown as an
   informational second column, not required to match.
2. **Version reporting:** `clientver` sends `DDNetVersion() = 20030` (the feature level
   we implement; servers gate behavior on it) with the string `DDNet-rs 0.1.0`.
   `NETMSG_INFO` keeps `"0.6 626fce9a778df4d4"`, which servers require.
3. **HTTP map download:** deferred; in-band only.
4. **Git:** repo initialised here; `origin` = github.com/Deafend88/ddnetrust,
   `upstream` = github.com/ddnet/ddnet; work happens on branch `rust-client`.

---

## 9. M6 plan: rendering, input, basic UI (draft for review, 2026-11-22)

Goal: a window that shows a local game the way the C++ client does: map layers,
tees with skins, hooks and entities, interpolated between ticks like C++, with C++
camera and mouse behaviour, the two camera messages the server expects, and a
minimal menu. Not in M6: HUD, chat, scoreboard, nameplates, particles, sounds,
demos, server browser, skin download, entities overlay, dummy. Still localhost only:
the connect box accepts only loopback addresses (127.0.0.1 / ::1).

### 9.1 Stack (versions checked 2026-11-22 via context7 and crates.io)

| Need | Choice | Why |
|---|---|---|
| Window, input | `winit` 0.30.13 (`ApplicationHandler` + `EventLoop::run_app`) | 0.31 is still beta; egui's integration pins `^0.30.13` |
| GPU | `wgpu` 30 | Vulkan/DX12/Metal/GL from one API; texture arrays and offscreen targets for tiles and screenshots |
| UI | `egui` 0.37 + `egui-wgpu` 0.37 + `egui-winit` 0.37 | immediate mode, draws into our own wgpu pass (no `eframe`); pins `wgpu ^30` and `winit ^0.30.13`; needs Rust 1.95 (we have 1.98) |
| PNG | `png` | skins, game skin and mapres are PNG; one small decoder, no `image` crate |
| Text | egui's font system with DDNet's own `fonts/*.ttf` from the data dir | no freetype binding; M6 needs text only in menus |

Nothing else. No async runtime: the network pump runs in `about_to_wait` on the
winit thread, in the same order as `CClient::Run` (pump network, update, render).

### 9.2 Architecture

A second crate, so wgpu/winit never touch the protocol/physics build:

```
rust-client/
  ddnet/                 library (+ the small additions below)
  client/                binary `ddnet-rs`
    src/
      main.rs            arguments, event loop
      app.rs             winit ApplicationHandler: pump -> predict -> render, like CClient::Run
      storage.rs         C++ CStorage search order (read only)
      config.rs          our own config file, C++ console syntax subset (`name value`, `bind key cmd`)
      gfx/
        mod.rs           device/surface setup, offscreen world target, screenshot readback
        batch.rs         textured-quad batcher: the IGraphics QuadsBegin/SetColor/QuadsDraw subset
        texture.rs       2D textures and 2D array textures (tilesets, like TEXLOAD_TO_2D_ARRAY_TEXTURE)
      assets.rs          skins, game skin, mapres, fonts from storage
      render/
        map.rs           game/map/render_map.cpp + render_layer.cpp: groups, tile and quad layers
        envelope.rs      RenderEvalEnvelope + envelope_state.cpp (time source)
        tee.rs           render.cpp RenderTee, animstate.cpp, skin colours
        players.rs       components/players.cpp: interpolation, hook, weapon
        items.rs         components/items.cpp: projectiles, lasers (incl. doors), pickups, flags
      camera.rs          components/camera.cpp (zoom, center, deadzone/follow factor)
      controls.rs        components/controls.cpp (mouse pos, clamp) + binds.cpp
      ui.rs              egui menus
    tests/               parity and screenshot tests (§9.4)
```

Additions to the `ddnet` crate (logic, so it stays testable without a GPU):
- `net/client.rs`: `IntraGameTick`, `PredIntraGameTick` and `IntraGameTickSincePrev`
  exactly as `CClient::Update` (client.cpp:2934-2946); previous and current snapshot
  access; sending `Cl_ShowDistance` and `Cl_CameraInfo` (gameclient.cpp:2313-2392).
- `predict.rs`: keep every predicted character at `PredGameTick - 1` and `PredGameTick`
  (C++ `m_aClients[i].m_PrevPredicted`/`m_Predicted`); rendering interpolates between
  them.
- Code generation also reads `datasrc/content.py`: sprites (game.png/particles.png
  cells), animations (base, idle, inair, walk, run, hammer/ninja swing keyframes),
  weapon render specs. Committed like `generated.rs`.

Room for later effects and a movable HUD, without building them now:
- The world is drawn into an offscreen texture, then composited to the window, with
  egui drawn last. Screenshots read the same texture back. A later post-effect is one
  extra pass between the two; M6 ships none.
- Rendering is a fixed, ordered list of components in the C++ `m_vpAll` order (map
  background layers, items, players, map foreground, menus). A later effect or HUD
  element is one more entry with an on/off config value, default off; HUD entries also
  get a position in the config. M6 adds no trait or registry for this: the list is a
  plain sequence in `app.rs`.
- Config values keep the C++ names and defaults where a C++ setting exists
  (`inp_mousesens 200`, `cl_mouse_max_distance 400`, `cl_default_zoom 10`,
  `gfx_vsync 0`, ...), so later options are just more entries.

### 9.3 Sub-steps

**6.1 Window and quad renderer.** winit window, wgpu surface, the batcher (textured,
coloured quads incl. C++ `QuadsSetSubset`, `QuadsSetRotation` and free-form quads),
normal alpha blending like C++ `BlendNormal`, the offscreen world target, PNG
screenshots of it, resize and DPI handling, vsync per `gfx_vsync`.

**6.2 Storage and assets.** `storage.rs` ports the CStorage search order: user dir
(`%APPDATA%\DDNet`, read only: we never write there), then the data dir (a DDNet
install's `data/`), then the current dir, as `storage.cfg` does. The data dir comes from
`--data-dir`, then `DDNET_DATA_DIR`, then auto-detection (next to the exe, Steam's
default path), and for development the repo's `data/` (the same 20.3 assets). All
loaded at runtime, nothing copied into the repo: default and user skins (`skins/`,
`downloadedskins/`), game skin (`assets/game/<cl_asset_game>` or `game.png`), particles,
emoticons, hud, map images (embedded RGBA or `mapres/<name>.png`), fonts. Missing or
broken files fall back like C++ (default skin, default game skin) and never panic. Our
own settings live in `%APPDATA%\DDNet-rs\settings.cfg` (see question 2).

**6.3 Map rendering.** Groups (offset, parallax, clipping, `MapScreenToWorld`), tile
layers via 2D array textures (index 0 skipped, flip/rotate flags, layer colour, colour
envelope with offset), quad layers (corner colours, UVs, position envelope with
rotation, colour envelope), the game group at the C++ position (the game layer itself
is not drawn, as with the default `cl_overlay_entities 0`), background colour.
Envelopes: `RenderEvalEnvelope` (step, linear, slow, fast, smooth, bezier) with the C++
time source (`envelope_state.cpp`: synchronized envelopes from game ticks since round
start plus intra tick, the others from local time).

**6.4 Tees.** `RenderTee` (body, feet, hands, eyes and emotes, outline pass,
direction-dependent eye offset), `CAnimState` blending, skin colours
(`cl_player_use_custom_color`, HSL conversion with `UnclampLighting` like C++), the held
weapon, hook chain and head (`RenderHook`), and the freeze/ninja visuals the tee
renderer itself draws (frozen eyes, ninja skin when the server says so).

**6.5 Interpolation.** Other players: `mix(prev, cur, IntraGameTick)` of snapshot
characters. The local tee and predicted others: between the predicted
`PredGameTick - 1` and `PredGameTick` with `PredIntraGameTick`, as `CPlayers::OnRender`
does with prediction on (defaults `cl_predict 1`, `cl_antiping 0`). Ported from
players.cpp:120-240, including the angle interpolation and its wrap-around fix.

**6.6 Entities.** Projectiles (position from start tick, tuning curvature and speed,
`CalcPos`, with intra tick), lasers incl. door lasers (`RenderLaser`, doors in their own
colour), pickups (animated offset), flags. Only what `items.cpp` draws for objects the
server sends; weapon prediction stays out of scope.

**6.7 Camera, mouse, binds, camera messages.** Mouse: raw motion
(`DeviceEvent::MouseMotion`) with the cursor hidden and confined in game, the
`OnCursorMove` scaling (`inp_mousesens / 100`), `ClampMousePos`, `GetMaxMouseDistance`,
the dyncam settings, and the mouse position as the input target. Camera: centered on
the interpolated local tee plus the follow-factor/deadzone offset, zoom levels and
smooth zoom (`cl_default_zoom`, `cl_smooth_zoom_time`), `CalcScreenParams`. Binds: key
names as `keynames.cpp`, the 58 binds of `CBinds::SetDefaults`, `+left`, `+right`,
`+jump`, `+hook`, `+fire`, weapon keys and zoom keys; other commands are ignored for now.
Messages: `Cl_ShowDistance` (from `CalcScreenParams` with the window aspect and zoom)
and `Cl_CameraInfo` (`round_truncate(Zoom * 1000)`, deadzone, follow factor), sent when
they change, with the C++ rules for which zoom is reported while zoom smoothing runs.
`ddnet-headless` sends them too, with a fixed 16:9 aspect.

**6.8 Menus (egui).** Start screen: address field (loopback only), name and skin,
Connect, Settings, Quit. Settings: player name, skin (from storage), mouse sensitivity,
dyncam on/off, vsync, data-dir path, binds for the movement keys. In game, Esc opens a
small menu (Disconnect, Settings, Quit). Saved to our own config.

### 9.4 Verification per sub-step

Two kinds of reference, because pixels can't be bit-identical across GPUs and graphics
APIs (C++ draws with OpenGL, we draw with DX12/Vulkan):

1. **Numeric parity, 100% required.** The same `rs_parity`/`rs_golden` approach as
   M2-M5, no GPU: everything that decides what is drawn where.
2. **Screenshot comparison with a tolerance** against the C++ client built from this
   repo (`CLIENT=ON`, its own build dir, run from a scratch dir whose `storage.cfg`
   keeps it off `%APPDATA%\DDNet`, connected to the same local server). Same window
   size, HUD and names off (`cl_showhud 0`, `cl_nameplates 0`), fixed camera (free-view
   spectating with `set_view x y`, fixed zoom), screenshots via the C++ `screenshot`
   command and our readback. Metric: the share of pixels with a channel differing by
   more than 8/255; a diff image is saved on failure. Target per scene: at least 99.5%
   matching pixels, tuned on the first real scenes; any threshold change is logged in
   DECISIONS.md.

| Step | Numeric parity (C++ data, 100%) | Screenshots / live |
|---|---|---|
| 6.1 | none | offscreen test pattern read back pixel-exact against expected values |
| 6.2 | storage search order against the `storage.cpp` rules on temp dirs; map image items (embedded/external, sizes) on all 7 maps against `rs_parity map` data | a missing data dir and broken PNGs give the fallbacks, no panic |
| 6.3 | `rs_parity envelopes`: every envelope of the 7 maps at many times, all curve types; group screen rects (`MapScreenToWorld`) for set centers, zooms and parallax | fixed views on coverage, Tutorial, Gold Mine, ctf1, dm1 (animated quads sampled at the same tick or masked) |
| 6.4 | `rs_parity anims`: `CAnimState` for every animation and time; skin colour conversion vectors | standing tees with default and custom-colour skins, each emote, both directions |
| 6.5 | intra-tick formulas against `CClient::Update` vectors; interpolated position and angle for recorded snapshot pairs against the players.cpp formula | live: a per-frame position log shows no jumps at tick boundaries |
| 6.6 | `rs_parity projectiles`: `CalcPos` positions per tick and intra tick for all weapons and tunings | a scene with pickups, a door laser and flying projectiles (ctf1, coverage) |
| 6.7 | `ClampMousePos`, `GetMaxMouseDistance`, `CalcScreenParams`, zoom steps, default binds and key names dumped from C++; `Cl_ShowDistance`/`Cl_CameraInfo` bytes against C++ packing | the server accepts both (no invalid-message log); the same raw mouse input gives the same aim as the C++ client (checked by hand once) |
| 6.8 | none | manual checklist: connect to 127.0.0.1, change settings, restart (settings kept), disconnect, quit; non-loopback addresses refused |

The existing gates stay: `cargo build`, `clippy -D warnings`, `fmt --check`,
`cargo test`. Screenshot tests skip themselves when there is no GPU adapter or no C++
client binary, like the server tests do without a server binary.

### 9.5 Risks

1. **GPU differences.** Texture filtering, sub-pixel rasterization and blending differ
   between OpenGL and DX12/Vulkan. Numeric parity decides the geometry; screenshots use
   a tolerance, and tile edges are snapped to pixels wherever C++ snaps them.
2. **Animated content in screenshots.** Envelopes depend on server tick and intra
   tick. Compare static scenes, or sample both clients at the same tick; the envelope
   maths itself is covered by numeric parity.
3. **Text.** egui text won't match the C++ text renderer pixel for pixel. M6 has text
   only in menus, which aren't compared.
4. **SDL vs winit input.** Relative mouse scale and acceleration may differ. Raw motion
   with the same sensitivity formula, checked against the C++ client by hand once.
5. **Building the C++ client** (SDL2, freetype, png, Vulkan from `ddnet-libs`) may need
   CMake options (e.g. `-DVULKAN=OFF`), no code changes.

### 9.6 Questions for review

1. **Data dir.** I found your user dir `%APPDATA%\DDNet` (TClient settings, custom
   `assets/`), but no installed `DDNet.exe` on C:. Plan: the user dir first (read only),
   then the data dir from `--data-dir`/`DDNET_DATA_DIR`, falling back to the repo's
   `data/` (same 20.3 assets). Should a specific install path be the default?
2. **Settings.** Our own `%APPDATA%\DDNet-rs\settings.cfg`; we never write to
   `%APPDATA%\DDNet`. Should it also read your `settings_ddnet.cfg` once (read only) to
   pick up name, skin, binds and mouse sensitivity?
3. **C++ client build** for reference screenshots (separate build dir, scratch storage,
   never touching your DDNet folder): OK?
4. **Screenshot threshold:** start at 99.5% of pixels within 8/255?

### 9.7 Answers (2026-11-22)

1. **Data dir:** keep the proposed order (user dir read only, then `--data-dir` /
   `DDNET_DATA_DIR`, then the repo's `data/`). No specific install path.
2. **Settings:** yes, read `%APPDATA%\DDNet\settings_ddnet.cfg` once, read only, for
   name, skin, binds and mouse sensitivity. Never write to `%APPDATA%\DDNet`.
3. **C++ client build:** approved (separate build dir, scratch storage); already done
   in `build-rs-client`.
4. **Threshold:** 99.5% of pixels within 8/255. A step that needs a different
   threshold says so to the user with the reason, and logs it in DECISIONS.md.

---

## 10. M7 plan: visual identity and HUD (approved 2026-11-24, answers in §10.6)

Goal: DDNet-rs gets its own look (one style, no "classic DDNet" mode) and a HUD built on
it: a shared design system, movable HUD widgets with an editor, the Dynamic Island,
the watermark, a DDRace status bar, a position/speed widget and killfeed capsules.
Stock DDNet stays only where it must: physics, protocol and prediction; map rendering;
tee skins; and the gameplay meaning of every indicator (each new widget shows the same
information the C++ HUD shows, decided by ported C++ logic).

Reference material lives outside the repo (`C:\ddnet-hud-refs`: SPEC.md, screenshots,
a friend's partial Kotlin code). Design and behaviour are ported; no code, text or
image from it enters the repository.

Not in M7 (later milestones, §11): chat, scoreboard, votes, emote wheel, name plates,
spectator HUD, menu redesign, server browser, Lua scripting. The HUD API is shaped so
Lua can drive it later (7.3).

### 10.1 Stack

| Need | Choice | Why |
|---|---|---|
| Layout, text, shapes, input in widgets | egui 0.37 (already used by the 6.8 menus) | rounded shapes, text layout, hit testing, animation helpers; one UI stack for HUD, island and later menus |
| Glass / Liquid Glass | own wgpu passes on the offscreen world image, drawn behind egui shapes | the world is already an offscreen texture (6.1); egui cannot sample what is behind it |
| Font | Inter (SIL OFL) | SPEC: SF Pro is Apple-only; Inter is the look-alike |
| Icons | Phosphor (MIT) as an icon font | one consistent line-icon set; `egui-phosphor` does not support egui 0.37 yet, so the TTF is loaded directly |
| Now playing (Windows) | `windows` crate, `Media_Control` (SMTC session manager) on a background thread | the only way to read and control the OS media session; read-only except the user's own play/pause/next/previous/seek |
| Springs, easing | own few lines (critically damped spring) | no dependency for one formula |

Linux MPRIS (`mpris`) and macOS come later; without a backend the media parts hide.

### 10.2 Design rules (from SPEC.md §1, §5, §6)

- Tokens in one place (`client/src/style.rs`): colours (dark surfaces, high-contrast
  text, grey secondary text, ONE accent: blue), radii (fully rounded pills, card
  radius), spacing scale, type scale (Inter regular/medium/semibold), icon sizes,
  shadows, animation durations and spring constants. Light surfaces are a later theme;
  the tokens allow it.
- Glass strength setting `rs_glass` = off / subtle / full, default subtle:
  off = opaque panels; subtle = mostly opaque dark panels over a light blur of the
  game; full = stronger blur, and the Dynamic Island uses Liquid Glass (refraction,
  specular rim, adaptive tint). Everything inside panels stays flat.
- UI scale follows the window DPI plus `rs_ui_scale`; all widget sizes are in points.
- Nothing gives a gameplay advantage: widgets show only what stock DDNet already shows
  or what the client legitimately has (SPEC §3).

### 10.3 Steps

**7.1 Design system.** `style.rs` tokens; Inter and Phosphor loaded at start (egui
font families "regular", "medium", "semibold", "icons"); components on egui: glass
panel, pill/capsule, icon + value + unit stat, separator dot, button (plain, accent),
toggle, slider, segmented control, popover, tooltip; spring and ease helpers. A
component gallery `examples/gallery.rs` renders every component headless to PNG.
The 6.8 menus switch to these components (same functions, new look).
Verify: spring/ease unit tests (critically damped: no overshoot, settles within the
configured time); the gallery rendered on the GPU and compared pixel-exact with its own
committed reference image (catches regressions, like `tests/gfx.rs`); the gallery PNG
reviewed by you.

**7.2 Glass rendering.** Per frame: a blur pyramid of the world target (dual-Kawase
downsample/upsample, strength by `rs_glass`); a glass pass per panel rectangle
(rounded-rect SDF mask with an anti-aliased edge, blurred backdrop, tint, 1 px border);
Liquid Glass for the island (refraction offset along the SDF normal near the edge,
specular rim, tint from the backdrop's mean luminance). Components register their panel
rectangles; the frame draws world -> glass -> egui -> present.
Verify: GPU tests on synthetic backdrops: `off` gives the exact opaque colour, the blur
keeps the mean colour of a flat area and stays inside the SDF mask, the SDF corner
coverage matches the analytic area, refraction moves samples inward along the normal,
the tint follows a dark or bright backdrop; frame time of the passes measured at 1440p
(target under 0.5 ms on this GPU); screenshots of all three strengths for your review.

**7.3 HUD system and data.** `HudData`: a read-only snapshot built each frame from the
client (local or spectated tee: position, velocity, weapons, DDNet character flags,
freeze/ninja ticks, jumps; race time inputs; fps; ping = the local `PlayerInfo`
latency, as the C++ scoreboard shows it; server address; local time; now playing),
plus an event queue (broadcast, finish, record, kill/finish messages, track change).
`Widget`: id, name, icon, anchor (9 points) + offset, enabled, own settings
(bool / int / choice / ordered list), rendered through a small `Painter` (glass panel,
pill, text, icon, image, progress, opacity/transform for animations). Layout and
widget settings are stored as console lines in our `settings.cfg`
(`hud_widget <id> <anchor> <x> <y> <enabled>`, `hud_set <id> <key> <value>`), parsed by
the 6.7 console code. `HudData`, the events and `Painter` are the surface a later Lua
API exposes (read-only data in, drawing out, no input or commands).
Verify: anchor/offset maths and DPI scaling tests; settings round trip through the
console parser (like 6.8); a `tests/server.rs`-style check that the sent inputs are
byte-identical with every widget on and off (the HUD only draws).

**7.4 HUD editor.** Opened from the Esc menu or a bind (`hud_editor`): game input goes
to menu mode (6.8 rules), the cursor is free, widgets show outlines; drag to move with
snapping (screen edges and centre, other widgets' edges, 8 pt grid; Shift disables),
click for the widget's settings popover (toggles, order, compact mode), reset one /
reset all, enable/disable list. Changes are saved when the editor closes.
Verify: snapping and anchor-reassignment unit tests (a widget dragged to the right edge
gets the right anchor and keeps its screen position on resize); a scripted live run
(keyboard and mouse events sent to our window only) that moves a widget, closes the
editor, restarts and finds it at the saved place.

**7.5 Watermark widget** (SPEC §1; the look as close to 1:1 with the friend's HUD as
the two PNG references allow). One dark glass pill top-left (offset 16, 16); inside, a
white logo pill with a soft glow (icon + "DDNet-rs", icon only in compact mode); stats
as icon + white value + grey unit, separated by faint dots. Items and defaults as in
SPEC §1's table: user (on), uid = client id (off), fps (on), ping ms (on), server
(off), time HH:MM (off), tps 50 (off), speed in tiles/s (off); reorderable, with a
separator.
Verify: the stat values against their C++ sources (fps averaged like the C++ debug
FPS, ping as above, speed as the movement information of 7.6); layout measurements
(paddings, font sizes 10/9, gaps 8, offset 16) as unit tests; side-by-side captures
with `hud-watermark-dark-bg.png` and `hud-watermark-light-bg.png` at the same scale for
your review (the references are video crops, so no pixel threshold).

**7.6 Position/speed widget** (replaces the C++ movement information panel, bottom
right): position in tiles, speed in tiles/s, aim angle, each toggleable, watermark
style. Values ported from `CHud::GetMovementInformation` (hud.cpp:1475 ff.).
Verify: numeric parity with the real C++ function through the `rs_parity` harness used
for CCamera in 6.7 (CHud constructed in the zeroed CGameClient block), 100% identical.
If CHud cannot be run that way, I come back to you before choosing another method.

**7.7 DDRace status bar** (replaces `CHud::RenderPlayerState`, hud.cpp:842): one
compact bar with the same indicators and meaning: jumps left, weapons owned and active,
ninja and freeze progress, deep/live freeze, jetpack, endless hook and jump, solo,
collision/hook/hammer/gun/shotgun/grenade/laser disabled, teleport guns, practice and
the other DDNet character flags C++ shows; new icons, readability first.
Verify: the decision logic (which indicators, the jump count, progress values) as a
pure function ported line by line and tested on cases built from the C++ code; live on
the local server with test commands (jetpack, endless, solo, super, freeze tiles) while
the C++ client spectates our tee: in each state the set of indicators it shows equals
ours (checked from captures).

**7.8 Killfeed capsules** (replaces `CInfoMessages`): kill, team kill and finish
messages as capsules top right that slide in and fade out, with the same entries,
count limit and lifetime as C++ (`Sv_KillMsg`, `Sv_KillMsgTeam`, `Sv_RaceFinish`).
Verify: message handling (grouping, order, limit, expiry) as unit tests against the
C++ rules; live with two clients (kill command, a finish on a race map).

**7.9 Dynamic Island** (user decisions 2026-11-25: a separate widget from the
watermark; the local clock is in the watermark, the island has no clock; DECISIONS.md)
(top centre; states from `island-concept.png`): idle = race
timer + mini cover when music plays; track change = cover, title and artist for 3 s,
then back; broadcast = the server's centre message; finish/record = check icon, time
and the difference to the record; pause/spectate notices. A priority queue decides what
shows; size changes animate with springs. Liquid Glass when `rs_glass` is full.
Logic ported exactly: the timer (`CHud::RenderGameTimer`, hud.cpp:118, incl.
`GAMESTATEFLAG_RACETIME` and `Sv_DDRaceTime`), the broadcast (`CBroadcast`: duration and
colour codes), finish/record text (`Sv_RaceFinish`, `Sv_Record`) with C++ `str_time`
formatting.
Verify: `str_time` golden vectors from `rs_golden` (every format and edge value); timer
values against vectors from the same C++ formula with recorded GameInfo objects; live:
the island shows the same timer text as the C++ client on the same server at the same
tick (side-by-side capture), an rcon `broadcast`, a finish on a race map.

**7.10 Media player.** SMTC on a background thread: sessions, title, artist, cover
thumbnail, position, duration, playback state; previous / play-pause / next / seek.
In the island: the mini cover in idle, the track-change state, and the expanded player
(cover, title, artist, progress with elapsed/remaining, controls) opened by a bind,
which frees the cursor like a menu; a compact capsule; the "⋯" menu (source session,
compact, hide). Binds `media_play_pause`, `media_next`, `media_prev`. Without a session
or on other OSes everything media-related hides.
Verify: state mapping and time formatting unit tests; live with your player (Spotify,
Yandex Music or a browser): title/cover/progress update, the controls act on the chosen
session, closing the source hides the player (a checklist done by you). The keyboard
media keys are not handled in game (§10.6).

**7.11 Settings and finish.** Settings page entries (glass strength, UI scale, HUD
editor, media on/off), the default HUD layout, a frame time check with everything on
(target: M6 frame time + under 1 ms at 1440p), docs.
Verify: all gates; the M6 screenshot comparisons still pass with the HUD disabled
(map, tees and entities unchanged); a review session with you.

### 10.4 Risks

1. **Text quality.** egui's text is good but not pixel-identical to the references;
   the references are blurry crops, so the target is "same look", reviewed by eye.
2. **Glass cost.** Blur passes every frame at high resolutions; measured in 7.2 and
   scaled down (fewer pyramid levels) if needed.
3. **SMTC.** WinRT is asynchronous and some players report no timeline; the progress
   row then hides (SPEC §2). The `windows` crate is large; only the needed features.
4. **C++ HUD logic in harnesses.** CHud depends on more of CGameClient than CCamera
   did; 7.6/7.7 may need a different harness or parity method.
5. **Missing material from the friend** (MISSING.md: the rest of the watermark code,
   theme colours, glass parameters, icon names): values taken from the screenshots.

### 10.5 Questions for review

1. **Fonts and icons in the repo:** Inter (OFL) and Phosphor (MIT) are not DDNet
   assets. Commit the TTF files under `rust-client/assets/` with their licences
   (embedded with `include_bytes!`), or load them from a user folder?
2. **`windows` crate** for SMTC (Microsoft's official crate, large but feature-gated):
   OK?
3. **Default binds** for the HUD editor and the media player (C++ has none): e.g. F6
   editor, F7 expanded player, the keyboard media keys for play/pause/next/previous?
4. **Glass "subtle" level:** start with a small blur and ~85% opaque panels, tuned
   with your review in 7.2?
5. **Review points:** the gallery (7.1), the three glass strengths (7.2), the watermark
   next to the references (7.5) and the island states (7.9). Anything else you want to
   see before the code is final?

### 10.6 Answers (2026-11-24)

1. **Fonts and icons:** Inter and Phosphor are committed under `rust-client/assets/`
   with their licence files (SIL OFL 1.1, MIT).
2. **`windows` crate:** yes, with only the features SMTC needs.
3. **Binds:** F6 opens the HUD editor, F7 expands the media player. Checked: stock
   DDNet binds neither (its defaults, `CBinds::SetDefaults` + `SetDDRaceBinds`, use F1-F4
   and F10; the only hard-coded F key is F5 for refreshing lists in menus). They are
   added like C++ `SetDDRaceBinds(true)`: only when the key is still free. The keyboard
   media keys are NOT handled in game: Windows already sends them to the media app, so
   handling them too would toggle playback twice.
4. **Glass "subtle":** starts with a small blur and ~85% opaque panels, tuned with you
   in 7.2.
5. **Review material:** also a short screen recording (GIF or video) of the island
   transitions in 7.9, since animations cannot be judged from stills.

---

## 11. Roadmap after M7 (recorded 2026-11-24, updated 2026-11-26)

Current order (2026-11-26):
- **M8 antiping:** done and approved (§12).
- **M9 feature parity with stock DDNet, in our style:** planned in §13.
- **M10 effects and particles** (C++ behaviour; incl. the predicted events of
  `cl_predict_events` left out in M8).
- **M11 menu redesign + server browser + public servers** (SPEC §4-5; master servers,
  server info, HTTP map download, the in-game Browser tab, vanilla/old servers below).
- **M12 Lua scripting for visuals.**
- **M13 media player redesign** (last; it was not queued in PROGRESS.md before).

The notes below are the original 2026-11-24 entries.

- **Antiping: HIGH PRIORITY, required before playing on public servers.** Port the C++
  antiping exactly: `cl_antiping` and all its sub-settings (players, weapons, grenade,
  ...), the anti-ping smoothing, weapon and projectile prediction (FireWeapon, ninja,
  grenade and laser bounces) and the player prediction margins; verified against C++
  like the rest of the physics (`rs_parity` worlds, 100% identical).
- **Full feature parity with stock DDNet,** in the new style (not the stock look): every
  client feature a player uses: chat, scoreboard, votes, emotes and the emote wheel,
  spectating (incl. the spectator camera), dummy, demos (record and play), ghost,
  kill/pause, name plates, and all relevant settings.
- **Effects and particles** with C++ behaviour (explosions, hammer hits, freeze, death,
  trails, powerup shine, muzzle flashes, ...).
- **Menu redesign and server browser** (SPEC §4-5), together with public-server
  support (master servers, server info) and HTTP map download.
- **Lua scripting for visuals** (SPEC §7) after those: the 7.3 HUD API exposed to
  sandboxed scripts, read-only data in, drawing out, never input or commands.
- **Vanilla and very old DDNet servers** (no `CNetObj_DDNetCharacter`): the vanilla
  `CCharacter::Read` path, `DetectStrongHook`, guessing game info from the game type
  string. Until then other characters are not predicted there (§12.7).

The order between them is decided when M7 is done; antiping comes before any public
server. Antiping is planned in §12.

---

## 12. M8 plan: antiping (draft for review, 2026-11-25)

Goal: `cl_antiping` and every sub-setting behave exactly as in the C++ client: other
players, projectiles, lasers, pickups, draggers and plasma predicted ahead like the local
tee; the local player's weapons predicted (fire, reload, weapon switch, ninja dash,
hammer, grenade jumps, shotgun and laser bounces); the C++ smoothing of other players'
corrections; preinputs; the prediction margin and the antiping limit/percent. Reference:
the C++ client prediction (§8.1). Every physics part is verified against the real C++
prediction code tick for tick (raw `f32` bits), like M4/M5.

Not in M8 (each noted in DECISIONS.md when its code lands):
- **Dummy:** there is no dummy yet; the `PredictDummy()` branches are left out and
  marked `// C++ dummy branch not ported` at each site, so the dummy milestone finds them.
- **Predicted events** (`cl_predict_events`, default 0: sounds, explosions, hammer hits,
  damage indicators) and `CItems::ReconstructSmokeTrail` (only spawns particles; uses
  `random_float`): we draw no particles and play no sounds yet. The parent/child entity
  links of `CopyWorld` only feed `m_DestroyTick` for that trail, so they are left out too.
- **Demo playback:** the `STATE_DEMOPLAYBACK` checks are always false.

### 12.1 What the C++ client does

**Settings** (`config_variables.h:15-27, 730`, all `CFGFLAG_CLIENT | CFGFLAG_SAVE`):

| Setting | Default | Effect |
|---|---|---|
| `cl_predict` | 1 | prediction at all (`Predict()`) |
| `cl_antiping` | 0 | master switch for everything below except margin, freeze, preinput |
| `cl_antiping_players` | 1 | 0 off; 1 all other players predicted; 2 only players we interfere with (`IsInterfering`); 3 = 2, and frozen players keep being predicted |
| `cl_antiping_weapons` | 1 | `WorldConfig.m_PredictWeapons`: `FireWeapon` runs, and snapshot projectiles, pickups, lasers, draggers and plasma enter the prediction world |
| `cl_antiping_grenade` | 1 | snapshot projectiles drawn at the predicted tick (items.cpp:60) |
| `cl_antiping_gunfire` | 1 | with grenade and weapons on: projectiles, lasers and pickups drawn from the predicted world, the local tee's attack tick and weapon from prediction |
| `cl_antiping_smooth` | 0 | smooth corrections of mispredicted other players |
| `cl_antiping_preinput` | 1 | apply other players' `Sv_PreInput`s in prediction |
| `cl_antiping_limit` | 0 | ms; when set, others are predicted only that far ahead |
| `cl_antiping_percent` | 100 | how far ahead others are predicted when the limit is 0 |
| `cl_prediction_margin` | 10 | ms added to the predicted time; used only if the server has the `SyncWeaponInput` capability, else 10 (client.cpp:5454) |
| `cl_predict_freeze` | 1 | 0 off, 1 on, 2 "partial": the local tee may move in freeze for the last 1-2 predicted ticks (gameclient.cpp:2662) |

Gates (gameclient.cpp:955-997): `AntiPingPlayers/Grenade/Weapons` need `cl_antiping`,
their own setting and not spectating; `AntiPingGunfire` = Grenade && Weapons &&
`cl_antiping_gunfire`; `Predict` = `cl_predict` && world not paused && not spectating &&
a local character. The client also reads `sv_deepfly` and `sv_hit` in prediction
(character.cpp:296, projectile.cpp:96, laser.cpp:39, gameworld.cpp:379): the server never
sends them, so the client always uses its own defaults (1, 1); we use constants.

**Snapshot switch** (`UpdatePrediction`, gameclient.cpp:3487): world config incl.
`m_PredictWeapons = AntiPingWeapons()`; strong/weak order; the server-state world is
advanced to the snapshot tick with our stored inputs and, for others, `ApplyPreInputs`
(direct, then predicted input, 2540); every tick's positions go into
`m_aPredPos/m_aPredTick[tick % 200]`; then `NetObjBegin`, `NetCharAdd`, `NetObjAdd` for
every snapshot entity (gameworld.cpp:437) and `NetObjEnd`:
- projectiles: dropped if not local team or a non-shotgun direction is not unit length
  (ball mods); matched by id (`Keep`, DDRace shotgun lifespan reset); else matched to a
  locally fired projectile (id -1) which takes the snapshot id; else, if fired within 4
  ticks, the owner is guessed as the only character near the spawn point (`PrevPos` or
  `PrevPrevPos`, nearest < 200 and 1.2x closer than the second);
- pickups: skipped with `PICKUPFLAG_NO_PREDICT`; matched by type/subtype within 2 px
  (position taken over), inserted at the end of the list;
- lasers: rifle/shotgun matched by id or to a local one; a laser that stopped earlier
  than predicted gets energy 0 and the server's end point; draggers (strength > 0),
  doors (done in 5.3), plasma matched and re-read.

**Prediction** (`OnPredict`, 2580): copy the server-state world; destroy inactive
characters (`SnapTicks > 10`), other-team characters and other-team projectiles; for
each tick from `GameTick + 1` to `PredGameTick`: at `PredictionTick = GetPredictionTick()`
store every character's `m_PrevPredicted` before the tick and `m_Predicted` after it and
copy the world to `m_PrevPredictedWorld` (what the items are drawn from); the local tee's
pair is stored at `PredGameTick`; inputs in the order local direct, preinputs direct,
set tick, local predicted, preinputs predicted, then `Tick()`; positions into the
`m_aPredPos` ring. `GetPredictionTick` (client.cpp:5389): with limit 0 and percent 100
it is `PredGameTick`; otherwise `PredGameTick - PredictionMin`, at least `GameTick + 1`.

**World tick** (gameworld.cpp:200): entity lists in `ENTTYPE` order (projectile, laser,
door, dragger, light, gun, plasma, pickup, flag, character), each from its head
(`InsertEntity` puts new entities first unless `Last`); characters PreTick+Tick
(or TickDeferred first with `NoWeakHookAndBounce`); then TickDeferred of all;
`RemoveEntities` (marked for destroy); switch timers. Projectiles therefore move before
characters in the same tick.

**Weapons** (prediction/entities/character.cpp:99-536): `HandleWeapons` = `HandleNinja`
(expiry, the dash: `MoveBox` with the ninja velocity, hits within 2x radius, once per
player, not in solo or other teams) + `HandleJetpack` + pain-sound timer + reload timer
+ `FireWeapon`: needs 2 inputs, `m_PredictWeapons`, no reload; full-auto rules (grenade,
shotgun, laser, jetpack gun, frozen last tick); deep-frozen hammer; `CountInput` presses;
frozen = only the pain timer. Hammer (`FindEntities`, force from `normalize(Dir +
(0,-1.1)) * 10`, `ClampVel`, FNG variant, `Unfreeze`, interference, hit fire delay), gun
(projectile, not with jetpack), shotgun (vanilla: 5 spread projectiles; DDRace: a
shotgun laser), grenade (explosive projectile), laser, ninja (dash start, velocity
clamp 6000). Then attack tick and reload from the tuned fire delay. `TakeDamage` only
adds velocity through `ClampVel`.

**Entities:** `CProjectile::Tick` (`CalcPos` at previous/current time, `IntersectLine`,
`IntersectCharacter` with radius 6 or 1 for freeze, hit rules incl. `CanCollide` and
`GrenadeHitDisabled`, explosion, freeze projectiles per switch, bouncing, lifespan);
`CLaser` (`DoBounce`: `IntersectLineTeleWeapon`, `HitCharacter` incl. the shotgun push
and the stacked-shotgun `-2^31` velocity bug, a temporary `SetCollisionAt(TILE_SOLID)`
on tele-weapon tiles around `MovePoint`, the zero-distance bounce guard, bounce cost and
count, vanilla vs DDNet delay comparison); `CreateExplosion` (radius 135, inner 48,
strength from the owner's tune zone, team and hit rules, break when hits are disabled);
`CPickup::Tick` (movers via `MoverSpeed` every `(int)(50 * 0.15)` ticks, freeze, armor,
weapons, ninja effects, switch layer); `CDragger` and `CPlasma` ticks.

**Interference** (character.cpp:653, 1345): under `cl_antiping_players` 2/3 a player
becomes predicted when the local (or an already interfering) player hooks, bumps,
hammers, shotguns or blasts them; it stops when they change direction/jump or hook a wall
(unless frozen with 3). DDNet-rs already ports the hook/bump part (4.4).

**Smoothing** (`cl_antiping_smooth`, 2755-2812): on a new tick with a stable prediction
tick, for each other active player compare last frame's predicted position with the new
one; for a small error (0.05-5 px per ms of prediction time) start or extend a per-axis
smoothing window of `1000 * PredTime * (4 - 1.5 * PredTime / 800)` (capped at 700 ms and
300 ms between axes). `GetSmoothPos` (3951) then mixes the stored prediction history
`m_aPredPos` at a time between game time and predicted time (`GetSmoothTick`,
client.cpp:5418) with `1 - (1 - t)^1.2`.

**Rendering:** `UpdateRenderedCharacters` (3762): other players predicted per the mode
above, with their snapshot aim, and the smooth position when smoothing is on; the local
tee's attack tick and weapon from prediction with gunfire on. `players.cpp:216, 224,
615` (intra tick, hook from the predicted core, attack time). `items.cpp`: predicted
projectile time (60), predicted laser width timing (319), the `UsePredicted` path
(472-600: draw `m_PrevPredictedWorld`'s projectiles, the local player's lasers and
moving pickups, and skip snapshot items that `FindMatch` finds in the world).

**Messages that feed the prediction world:** `Sv_PreInput` (1236; stored by intended
tick % 200; the engine accepts it unreliable, client.cpp:2414; DDNet-rs already lets it
through), `Sv_KillMsg`/`Sv_KillMsgTeam` (1192-1228: `GiveWeak`, `ResetPrediction`,
`ReleaseHooked`), `Sv_TuneParams` (1053) with `UpdateLocalTuning` (3409, tune zones), and
`Sv_TeamsState` (1168: team membership for `IsOtherTeam`/`IsLocalTeam`/`CanCollide`).
The last three matter with antiping off too, but weapons and other players expose them
far more (gun speed, laser reach, explosion strength, team-only hits).

### 12.2 What DDNet-rs has today

`predict.rs`: OnNewSnapshot/UpdatePrediction/OnPredict for characters and doors, the
`cl_antiping 0` rendering, strong/weak order from the extended object, the input
history. `physics/world.rs`: characters with interference flags, jetpack, ninja expiry,
weapon switching; `FireWeapon` is a no-op and the ninja dash is never started.
`net/client.rs`: CSmoothTime with the margin fixed at 10 ms. `ddnet/src/items.rs`:
the Extract*Info functions and `CalcPos` (6.6). Missing: everything in 12.1 that
involves weapons, entities, `GetPredictionTick`, preinputs, smoothing, and the four
messages.

### 12.3 C++ → Rust mapping

| C++ | Rust | Notes |
|---|---|---|
| `config_variables.h:15-27, 730` | `client/src/config.rs` | same names, defaults and ranges; passed into `ddnet` as a plain `PredictConfig` struct (no globals, PLAN §7 risk 4) |
| `CClient::GetPredictionTime/GetPredictionTick/GetSmoothTick/PredictionMargin` | `net/client.rs` | pure functions of `now` and the smoothed times; margin from `PredictConfig` |
| `CCollision::IntersectLine`, `IntersectLineTeleWeapon`, `MovePoint`, `MoverSpeed`, `SetCollisionAt` | `physics/collision.rs` | the game tile grid becomes `Cell`-based like the door grid (5.3), so a laser can set and restore a tile through the shared `&Collision` exactly like C++ |
| `CGameWorld` entity lists, `InsertEntity(Last)`, `RemoveEntities`, `Tick` order, `FindEntities`, `IntersectCharacter(s)`, `CreateExplosion`, `CopyWorld`, `FindMatch`, `GetEntity` | `physics/world.rs` | one `Vec` per entity type in C++ list order (front insert = `insert(0)`), destroy flags removed after TickDeferred like `RemoveEntities` |
| `CCharacter::HandleWeapons`, `FireWeapon`, `HandleNinja` (dash), `TakeDamage`, `AntiPingInterference` call sites | `physics/world.rs` | weapon fire delays from tuning; `sv_deepfly`/`sv_hit` constants 1 |
| `prediction/entities/projectile.cpp` | `physics/projectile.rs` | `GetPos` = `items::calc_pos`; `Match`, `GetData`, `SetBouncing` |
| `prediction/entities/laser.cpp` | `physics/laser.rs` | `DoBounce`, `HitCharacter` incl. the stacked-shotgun bug, `Match`, `GetData` |
| `prediction/entities/pickup.cpp` | `physics/pickup.rs` | `Tick`, `Move`, `Match`, `FillInfo`, `InDDNetTile` |
| `prediction/entities/dragger.cpp`, `plasma.cpp` | `physics/dragger.rs`, `physics/plasma.rs` | as in C++ |
| `CGameWorld::NetObjAdd` (projectile/pickup/laser/dragger/plasma branches) | `physics/world.rs` | extends the door branch from 5.3 |
| `CGameClient::UpdatePrediction`, `OnPredict`, `ApplyPreInputs`, `UpdateRenderedCharacters`, `GetSmoothPos`, smoothing block | `predict.rs` | per-client `prev_predicted`/`predicted` at `PredictionTick`, the 200-slot `pred_pos/pred_tick` ring, `m_PrevPredictedWorld`, preinput ring, smoothing state; `time_get()` becomes a `now` argument |
| `Sv_PreInput`, `Sv_KillMsg(Team)`, `Sv_TuneParams` + `UpdateLocalTuning`, `Sv_TeamsState` | `predict.rs` (+ `net/client.rs` for tuning per connection) | messages already unpacked by the generated code |
| `CItems` antiping branches, `CPlayers` predicted attack time | `client/src/render/items.rs`, `render/players.rs` | `RenderLaser(IsPredicted)` timing, `RenderPickup(IsPredicted)` |
| `menus_settings_ddnet.cpp:150-205` | `client/src/ui.rs` | the same options in our style (§12.4 step 8.8) |

### 12.4 Steps

Two kinds of reference:
1. **Physics: the real C++ prediction code** linked into `rs_parity` (as for `world` and
   `doors`). Dumps hold raw `f32` bits of every character and entity field every tick.
   100% identical on all 7 maps (and the hook-teleporter variants), plus hand-made
   mutations of the Rust code that must be caught.
2. **Client-level functions** that are `CClient`/`CGameClient` members and cannot be
   linked into the `CLIENT=OFF` tool (`GetPredictionTick`, `GetSmoothTick`, the
   smoothing block, `GetSmoothPos`, `UpdateRenderedCharacters`' choice, the item
   timings): copied verbatim into `rs_parity` as `interp` did for `CClient::Update`,
   and a small script checks that each copy is textually identical (whitespace aside) to
   the C++ lines it names, so the copy cannot drift from the source. 100% identical.

**8.1 Messages the prediction world needs.** `Sv_TuneParams` (unpack stops at the first
error, jetpack strength defaults to 0 for that message, `m_ServerMode`), `UpdateLocalTuning`
(the tune-zone list, the expected-tuning state), `Sv_TeamsState` (+ legacy) into
`TeamsCore`, `Sv_KillMsg`/`Sv_KillMsgTeam` (GiveWeak in the C++ stable-sorted order,
ResetPrediction, ReleaseHooked), the `Sv_PreInput` ring.
Verify: `rs_parity msgs`: the real `CTuningParams`/`CTeamsCore` fed byte streams
(valid, truncated, garbage) give the same tuning arrays and teams; kill-message order on
constructed strong/weak states through the real `CCharOrder`. Live (local server):
`prediction_matches_server` variants after `tune gun_speed 3000`, in a tune zone
(coverage) and after joining a team: still 0 mismatched snapshots of the local tee.

**8.2 Collision for weapons.** `IntersectLine`, `IntersectLineTeleWeapon`, `MovePoint`,
`MoverSpeed`, `SetCollisionAt` (Cell grid).
Verify: `rs_parity collision` gains these queries on its random boxes and rays (incl.
the tele-weapon tiles turned on in the `hooktele` variant): 100%.

**8.3 Weapons and entities in the world.** `HandleWeapons`, `FireWeapon`, ninja dash,
`TakeDamage`, `CreateExplosion`, `FindEntities`, `IntersectCharacter(s)`, projectile,
laser, pickup, dragger, plasma, the entity lists and tick order.
Verify: new `rs_parity weapons`: per map, 4 characters with all weapons (and runs with
only some weapons, infinite and limited ammo), random aim/fire/weapon switch/next/prev
plus the 4.4 movement inputs, spawned next to each other and on effect tiles; runs with
`m_PredictWeapons` on, vanilla / DDRace / FNG world configs, `OldLaser`, changed tuning
and tune zones, solo, teams, hit-disabled flags, jetpack, deep freeze, ninja. Pickups
and draggers placed from the map's entities; plasma from turrets where a map has them,
otherwise synthetic. Every tick: every character and every entity (positions,
directions, energies, bounces, lifespans, eval ticks, interference flags). 100% on all
7 maps; every weapon and branch hit at least once (counted in the dump header).

**8.4 Snapshot entities into the world.** `NetObjAdd` for projectiles (all three object
types, with and without `EntityEx`), pickups, lasers, draggers, plasma; `Keep`/`NetObjEnd`,
the id -1 matching to locally fired entities, the owner guess, the shotgun lifespan,
the "stopped earlier" laser fix, `CopyWorld`, `FindMatch`.
Verify: new `rs_parity netents` (like `doors`): 60 snapshots per map that mix server-like
entities, locally fired ones predicted a few ticks earlier, odd objects (other team,
no-predict, ball-mod directions, unknown owners) and entities that leave and come back;
after every snapshot the world's entity lists and `FindMatch` answers for each item are
compared. 100%.

**8.5 Prediction loop.** `GetPredictionTick` (limit/percent), `PredictionMargin`
(configurable, `SyncWeaponInput`), per-client predicted cores at `PredictionTick`, the
`pred_pos` ring, `m_PrevPredictedWorld`, `ApplyPreInputs` in both worlds,
`cl_predict_freeze 2`.
Verify: `rs_parity antiping` timing vectors (copied C++ functions, checked by the
script): `GetPredictionTick`/`GetSmoothTick` over random game/predicted times, limits
and percents. The loop itself through the real `CGameWorld`: `rs_parity weapons` gets an
`OnPredict` mode that replays the 4.4 inputs from a snapshot world with preinputs for
the others and dumps `PrevPredicted`/`Predicted` for every client at random
`PredictionTick`s. Live: `prediction_matches_server` with `cl_antiping 1` and scripted
fire on dm1 and coverage (grenade jumps, hammer and shotgun on a second client, laser
bounces): 0 mismatched local snapshots, 0 prediction errors, every server projectile
and laser of ours matched to a locally fired one.

**8.6 Smoothing.** The `cl_antiping_smooth` block, `GetSmoothPos`, the smooth state per
client.
Verify: `rs_parity antiping` feeds the copied block recorded sequences (positions,
prediction times, `now` values, ticks) incl. the edge cases (error outside 0.05-5, window
overlap, the 700/300 ms caps, axis equalisation): mix amounts, windows and positions
identical. Live: a second scripted client walks/jumps with sudden direction changes;
with smoothing on, the largest per-frame step of the other tee's rendered position is
logged and shown to you next to smoothing off.

**8.7 Rendering.** `UpdateRenderedCharacters` (modes 0-3, interference, frozen, other
team, gunfire attack tick and weapon), players' predicted intra tick and attack time,
items: predicted projectile time, `m_PrevPredictedWorld` projectiles/lasers/pickups,
skipping matched snapshot items, predicted laser timing.
Verify: `rs_parity antiping` vectors for the copied choice and timing formulas (100%);
`rs_parity items` gains the predicted time branches. Live: a capture series with the
C++ client (build-rs-client, `cl_antiping 1`) next to ours on the same local server for
your review (a short recording of firing every weapon; moving scenes have no pixel
threshold, as in 6.5).

**8.8 Settings and finish.** An "Antiping" group on the settings page in our style,
matching C++'s structure: master toggle; when on: other players (Off / All / Interact,
plus "Always predict frozen players" for Interact), predict weapons, predict grenade
paths; secondary options (smooth, gunfire, preinput, limit, percent, prediction margin,
predict freeze) behind a detail view. Frame time with antiping on and 8 tees firing
(target: within 1 ms of antiping off at 1440p).
Verify: all gates; the UI review against the CLAUDE.md UI rules and frontend-design;
the M5/M6 prediction tests still pass with `cl_antiping 0` (default path unchanged).

### 12.5 Risks

1. **Shared collision mutation.** The laser's temporary `SetCollisionAt` writes the grid
   every prediction world shares; the `Cell` grid keeps that exact, and nothing runs
   between set and restore (single thread).
2. **Entity order.** Hits and explosions depend on list order (front inserts, pickups
   at the end, removal timing, copy order). Dumps compare lists in order, not as sets.
3. **Others' inputs are unknown.** Live checks for other players cannot reach 100%;
   they report counts. The 100% claims rest on the `rs_parity` worlds with the same
   inputs on both sides.
4. **Verbatim copies.** Client-level functions are compared with copies, not the linked
   C++ functions; the textual check is what makes that acceptable (question 3).
5. **Public servers:** older servers without `SyncWeaponInput` use the 10 ms margin
   (ported); servers without `DDNetCharacter` need `DetectStrongHook` and the vanilla
   `Read` path (question 2).
6. **Size of `world.rs`** (1,364 lines today): entities go to their own files like
   `door.rs`; character weapons stay in `world.rs` next to the rest of `CCharacter`.

### 12.6 Questions for review

1. **Default `cl_antiping`:** C++ ships 0. Keep 0, or default to 1 in DDNet-rs (since it
   is the reason for this milestone)? And import the `cl_antiping*` /
   `cl_prediction_margin` values from `settings_ddnet.cfg` once, like name/skin/binds?
2. **Servers without `DDNetCharacter`** (vanilla and very old DDNet): port
   `DetectStrongHook` and the vanilla `Read` path in M8, or keep refusing to predict
   others there until it is needed?
3. **Verbatim copies + textual check** for the `CClient`/`CGameClient` members (the
   alternative is building `rs_parity` against the full client sources with SDL etc.,
   a large CMake change): OK?
4. **Settings layout** as in 8.8 (C++ structure, secondary options in a detail view): OK?

### 12.7 Answers (2026-11-25; plan approved)

1. **Default:** `cl_antiping` stays 0 like C++. The `cl_antiping*` and
   `cl_prediction_margin` values are read once from `settings_ddnet.cfg`, like name,
   skin and binds.
2. **Vanilla / very old DDNet servers** (no `DDNetCharacter`): unsupported for now;
   later work in §11.
3. **Verbatim copies + textual check script:** approved.
4. **Settings layout:** approved (master toggle -> players/weapons/grenades -> detail
   view), in the design-system style.

---

## 13. M9 plan: feature parity with stock DDNet, in our style (draft for review, 2026-11-26)

Goal: every client feature a stock DDNet player uses, behaving like C++ (same
messages, timings, rules, files) and drawn in the DDNet-rs design system (§10.2,
CLAUDE.md UI rules). Parts in the order you set; each ends with the gates, a commit and
a short report. Localhost only, as before.

How "behaves like C++" is checked, as in M8:
- **Linked C++** in `rs_parity` where the code is in `game/` or `engine/shared` (demo
  recorder/player, ghost files, localization, console).
- **Word-for-word copies** (`scripts/gen_cpp_copies.py`, checked by
  `cpp_copies_match_source`) for `CGameClient`/component members that cannot be linked.
- **Message bytes** against the generated C++ packers (already golden-tested), and the
  local server's log/state in live tests.
- **UI**: the frontend-design skill, the CLAUDE.md UI rules, a gallery render per part for
  your review. No pixel thresholds for our own-style UI (it is not the stock look).

### 13.1 Chat (`components/chat.cpp`)

C++: `Sv_Chat` → `AddLine` (chat.cpp:680): filters (`cl_show_chat_system`, friends only,
team members only, foes), sanitising, repeat folding (same client/team/text → `[n]`
counter), highlight on the local name (`LineShouldHighlight`, 614), team (`Team == 1`)
and whisper (`Team >= 2`), 64-line ring, sounds rate-limited to one per 0.3 s per kind
(server/highlight/client, 891-936). Lines fade 14-16 s after arrival unless the chat is
open (1297-1306). Input: `+chat`/`+chat_team` modes, history, Tab completion of player
names and of server commands from `Sv_CommandInfo`/`Sv_CommandInfoRemove` (sorted),
`SendChatQueued` (1388: at most one `Cl_Say` per second, up to 3 queued), Esc cancels,
`cl_chat_reset`. Saves from `/save` lines into `ddnet-saves.txt` (`StoreSave`, 640).
`cl_showchat 0/1/2`, `cl_chat_old`, `cl_chat_size/width`, `cl_message_*` colours.

Rust: `client/src/chat.rs`: line ring + filter/repeat/highlight/queue logic (pure,
`now` as input), input box with history and completion; the server command list in
`net/client.rs` alongside the other messages. Look: a bottom-left rounded-rect glass panel
when open, plain lines with a soft shadow when closed; names in medium weight; team,
whisper and server lines marked by a small icon (neutral), highlight in the accent.
`StoreSave` writes to our own user dir (§13.12 q6).

Verify: copies of `AddLine` (filters, folding, highlight), `LineShouldHighlight`,
`SendChatQueued` and the completion code in a new `rs_parity chat`: random message
streams, names and keystrokes → line ring, sends and completions identical (100%).
Live: DDNet-rs and the C++ client on the local server chat both ways (all/team/whisper,
a highlight); `Sv_CommandInfo` completion lists `/pause`, `/team`, ...

### 13.2 Scoreboard (`components/scoreboard.cpp`)

C++: `IsActive` (1206): `+scoreboard` held, dead with `cl_scoreboard_on_death`, or game
over; hidden while the statboard is open. Order from `CGameClient::OnNewSnapshot`
(gameclient.cpp:2174-2225): stable sort by name, then by score with
`GetScoreComparator` (2479: time score, DDNet finish times, millis), then grouped by
DDRace team. Rows: score or finish time (`DDNetPlayer` finish times, `str_time`), name,
clan (same-clan colour), country flag, ping (clamped 0-999, ping colour); DDRace team
blocks with team colours; spectators list; title with map/score/time limit; red/blue
for team games; recording notification.

Rust: the sort and the finish-time state in `ddnet` (pure, beside the snapshot code);
`client/src/scoreboard.rs` draws a centred rounded-rect panel: rank, tee head
(`teehead.rs`), name, clan, time/score, ping; DDRace teams as grouped sections with a
neutral team label (team colour only as a small swatch); spectators as a compact row;
country flags from `countryflags/` (data dir) on hover only (secondary info).

Verify: `rs_parity scoreboard` with copies of the sort block and `GetScoreComparator`
over random player sets (scores, unset/real finish times, teams, spectators) → identical
orders (100%); `IsActive` cases as copies. Live: 4 headless bots in teams on a race map,
side by side with the C++ scoreboard (same rows, same order, same times) for your review.

### 13.3 Sound (`engine/client/sound.cpp`, `components/sounds.cpp`, `mapsounds.cpp`)

C++: SDL audio at `snd_rate` (48000); samples decoded once (WavPack for `audio/*.wv`,
Opus via opusfile for map sounds), resampled with `RateConvert` (424); `Mix` (58) mixes
up to 256 voices in integers with per-channel volume/pan, listener distance falloff and
circle/rectangle shapes for map sources, clamped to 16 bit. Channels and volumes
(sounds.cpp:51): GUI (`snd_chat_volume`), world 0.9×game, global (game), map sound
(`snd_ambient_volume`), music. Sample of a set: random but never the last one
(`GetSampleId`, 83). Sources: `Sv_SoundGlobal`, `NETEVENTTYPE_SOUNDWORLD` events
(gameclient.cpp:1469), the predicted jump/hook sounds (2728), chat sounds (13.1), weapon
switch/no-ammo, menu music. Map sounds (`CMapSounds`): sound layers, source positions
with position envelopes, loop, time delay from round-start game time (OnRender 150),
falloff and shapes, `gfx_high_detail` sources. Settings: `snd_enable`, `snd_volume`,
`snd_game/chat/servermessage/highlight/gun/long_pain`, `snd_nonactive_mute`,
`snd_enable_music`.

"The selected asset sounds": stock DDNet has no `cl_asset_*` for sounds; it loads
`audio/*.wv` through the storage search order, so files in the user dir's `audio/`
replace the defaults. We do the same (§13.12 q2).

Rust: `client/src/sound.rs`: decode + `RateConvert` + `Mix` ported, output through
`cpal` (WASAPI on Windows) in a callback that pulls from the mixer like SDL's;
`components/sounds.cpp` and `mapsounds.cpp` logic next to it; every `Play` call site of
C++ mapped to ours (a table in the commit). New dependencies (§13.12 q1).

Verify: `rs_parity sounds`: PCM of every `data/audio/*.wv` (real libwavpack) and of
every map sound in the 7 maps (real opusfile) → identical samples (100%; for Opus a
bit-exact decoder is required, otherwise I report the deviation before going on);
copies of `Mix`, `AdvanceVoice`, `RateConvert` and the map-sound source logic driven by
scripted voices, positions and envelopes → identical 16-bit buffers (100%). Live:
a listening check by you (jump, hook, weapons, chat, a map with ambient sounds).

### 13.4 Crosshair and asset packs (`hud.cpp:609`, `gameclient.cpp` `Load*Skin`, `menus_settings_assets.cpp`)

C++: `RenderCursor` draws `SPRITE_WEAPON_*_CURSOR` of the active (predicted) weapon from
the game skin at the target position; spectators see the spectated player's cursor
(`cl_spec_cursor`, `CCursorInfo`, clamped to the screen, half alpha). Asset packs:
`cl_asset_game/particles/emoticons/hud/extras` and `cl_assets_entities`, each a
`assets/<kind>/<name>.png` file or `assets/<kind>/<name>/<kind>.png` directory, default
fallback, per-sprite fallback for fully transparent cells (`assets.rs` already ports the
load chain); entities packs per mod (`gs_apModEntitiesNames`: ddnet, race, fng, vanilla,
...). Lists come from `ListDirectory` over all storage paths.

Rust: the cursor in `render/` (world space, like C++); `assets.rs` gains listing and
switching at runtime (textures reloaded); the five values and `cl_assets_entities` read
from `settings_ddnet.cfg` once, like name/skin. Entities packs only show with the
entities overlay, so this part also ports `cl_overlay_entities` / the game-layer
entities rendering (render_map's entities path with the mod's entities image) —
otherwise the entities pack would do nothing. Asset picker on the settings page
(thumbnails, like C++'s grid).

Verify: `rs_parity assets` over every stock pack in `data/assets` and the packs in your
`%APPDATA%\DDNet\assets` (read only): chosen file, fallbacks and per-sprite fallback
masks identical to the real `CGameClient` loader code (copies, 100%); the cursor
position/alpha maths as copies (100%). Screenshots against the C++ client with the same
`cl_asset_*` values: tees, weapons, cursor, HUD and entities overlay (99.5% within
8/255, as §9.7). Your folder also has a `cursor` pack: that is not a stock DDNet asset
kind and is ignored.

### 13.5 In-game Esc menu (`menus_ingame.cpp`, concept image)

C++: Game page (45): Disconnect (confirm when recording/dummy), dummy connect/disconnect,
record/stop demo, spectate / join red / blue / game, kill, pause (`say /pause`), auto
camera; Players (492: mute/ignore, friends); Server info (622) + MOTD; Call vote (946:
settings, kick, spec, reason, force with rcon); in-game Browser (1150); Ghost (1355).

Ours, per the concept (`C:\ddnet-hud-refs\screenshots\esc-menu-concept.png`, renamed
from an unnamed file on 2026-11-27): a compact rounded-rect glass bar across the top over the
running game (game input paused like the 6.8 menu). Row 1: segmented control Game,
Players, Info, Browser, Ghost, Vote; icon buttons on the right (demos, editor,
settings, quit). Row 2 (Game tab): pills with Material Rounded icons — Spectate,
Respawn (`Cl_Kill`), Pause, Record demo, Dummy control — a divider, Disconnect in muted
red. Other tabs open a
rounded panel under the bar. Review 2026-11-28: no "Esc to close" hint; larger controls, the
bar full width, tab panels large and centred. Tabs and actions never share a row. Each pill shows the C++ label state (Pause ↔ Join game,
Record ↔ Stop record, Spectate ↔ Join game) and the C++ visibility rules.
Tabs and pills whose feature lands in a later step stay hidden until then (Ghost → 13.8,
Dummy → 13.6, Vote → 13.7); Browser stays hidden until M11 (§13.12 q4). The editor
button is hidden: there is no editor.

Rust: `client/src/esc_menu.rs` replacing the 6.8 Esc menu; Players and Info tabs here
(mute/ignore/friend lists, server info, MOTD from `Sv_Motd`).

Verify: gallery render of every tab and state for your review; the C++ visibility
conditions as copies (`rs_parity` vectors over local-info/spectator/pause/team states,
100%); live: each pill against the local server sends the C++ message (server log /
state: spectator, kill, pause, chat `/pause`).

### 13.6 Dummy (`client.cpp:816` `DummyConnect`, `SendInput` 350, gameclient dummy paths)

C++: a second connection (`CONN_DUMMY`) with the same handshake and the dummy's own
name/skin (`dummy_name`, ...); `cl_dummy` switches which tee the controls drive; inputs
for both every tick (`cl_dummy_hammer`, `cl_dummy_copy_moves`, `cl_dummy_control`,
`cl_dummy_resetonswitch`); `m_aLocalIds[2]`; prediction of the dummy
(`PredictDummy()`, the "C++ dummy branch not ported" markers of M8); dummy-aware HUD
and camera; `dummy_connect`/`dummy_disconnect`/`toggle_dummy` binds.

Rust: `net/client.rs` holds two `Conn`s; the predictor fills the marked dummy branches;
binds and settings.

Verify: handshake and input packets of the dummy byte-identical to the main
connection's path (golden-style, injected tokens); copies of the dummy part of
`SendInput` (hammer/copy moves) → identical inputs (100%); `rs_parity antiping` gains a
dummy in the world → 100% on all 7 maps. Live: both tees on the local server, swap,
copy moves; `prediction_matches_server` for both.

### 13.7 Votes, spectate, kill, pause, emotes

C++: `CVoting` (voting.cpp:250): `Sv_VoteSet`/`Sv_VoteStatus`, option list
add/remove/clear (in order), `Cl_Vote`, `Cl_CallVote` (option, kick, spec with reason,
`CallvoteKick` 68), vote binds F3/F4, the vote HUD. `CSpectator`: `Cl_SetSpectatorMode`,
next/prev/closest (36, 655), free view, the spectator selection wheel, the spectator
camera, `cl_spec_cursor`. Kill: `SendKill` (`Cl_Kill`, kill bind). Pause: `say /pause`
and the spectating-while-paused state. Emotes: `CEmoticon` wheel (+emote, angle →
emoticon/eye emote, `Cl_Emoticon`, `/emote` eye emotes) and drawing the emoticons over
tees (`cl_asset_emoticons`).

Rust: `client/src/vote.rs` (vote state + an island-style vote notice with F3/F4 hints,
the Vote tab), `spectate.rs` (logic + our style wheel), `emote.rs` (our style wheel);
emoticon rendering over tees in `render/players.rs`.

Verify: copies of `CVoting::OnMessage` and the spectator next/prev/closest choice, and
of the wheel angle → selection maths → vectors (100%); message bytes for each `Cl_*`.
Live: the local server with `add_vote` options: call/yes/no, kick-vote a bot; spectate
next/free view; emote seen by the C++ client.

### 13.8 Demos and ghost (`engine/shared/demo.cpp`, `engine/client/ghost.cpp`, `components/ghost.cpp`, `menus_demo.cpp`)

C++: `CDemoRecorder` (header `TWDEMO`, version 6, map embedded with SHA256, snapshot
deltas and messages in Huffman-compressed chunks, tick compression, markers,
`cl_auto_demo_record/max`, race demos); `CDemoPlayer` (keyframes, seek, speed, pause,
slice); the demo browser and player UI. Ghost: `CGhostRecorder`/`CGhostLoader` files
(header, zlib-compressed `CGhostCharacter` chunks, map name/SHA check),
`CGhost` (start/stop on race start/finish, `cl_race_ghost*`, best-only save, render with
`cl_race_ghost_alpha`, mirror of the local tee's skin).

Rust: `ddnet/src/demo.rs` (writer/reader; reuses snapshot delta + Huffman),
`ddnet/src/ghost.rs`; the client plays demos through the same snapshot path as online
(state `DemoPlayback`, which also turns on the M8 `STATE_DEMOPLAYBACK` checks); demo
browser/player in our style (a bottom transport bar: play/pause, seek, speed, markers);
Ghost tab in the Esc menu. Files go to our own user dir (§13.12 q6).

Verify: `rs_parity demo` drives the real `CDemoRecorder` with a stream recorded from a
live session → our file byte-identical (timestamps injected); the real `CDemoPlayer`
reads our demo and ours reads C++ demos (recorded by `build-rs-client`) with identical
per-tick snapshots/messages (100%), incl. seeks. Same for ghosts with the real
`CGhostRecorder`/`CGhostLoader`. Live: the C++ client plays our demo and our ghost.

### 13.9 Console (`components/console.cpp`, rcon in `client.cpp:293-350`)

C++: local console (F1) and remote console (F2): backlog, history, search, completion of
commands/settings (`PossibleSettings` 197) and of rcon commands from
`Sv_RconCmdAdd/Rem`, rcon login (`RconAuth`, name+password), `Sv_RconLine`,
`Sv_RconAuthStatus`, `Sv_RconCmdGroupStart/End`.

Rust: `client/src/console_ui.rs` on top of `ddnet/src/console.rs` (already the C++ parser
subset); rcon messages in `net/client.rs`. Look: a top sheet panel, monospace backlog,
our tokens. Passwords are typed by you; never stored by us unless C++ stores them.

Verify: completion/history/search as copies (100%); rcon packets byte-identical to C++
packing; live: rcon login on the local server (scratch `sv_rcon_password`), command
completion list equals the server's, `status` output shown.

### 13.10 Full settings page (`menus_settings*.cpp`)

C++ pages: Language, General, Player, Tee, Appearance, Controls, Graphics, Sound, DDNet,
Assets. Ours: one settings page in our style with the same sections, holding every C++
setting that DDNet-rs implements by then (the M7/M8 rows move in), C++ names, defaults
and ranges; binds editor for all `CBinds` commands; tee/skin picker.

Verify: a test parses `src/engine/shared/config_variables.h` and checks every C++-named
variable we expose has the same default, range and save flag (100%); round trip through
`settings.cfg`; UI review against the rules, gallery render for you.

### 13.11 Localization (`game/localization.cpp`, `data/languages/`)

C++: `index.txt` (file, name, country, alternative locales), `SelectDefaultLanguage`
(124) from the OS locale, `Load` (183: `== translation` lines, `[context]` blocks),
`Localize(str, context)` looked up by hash; `cl_languagefile`; plural forms are separate
strings. Files loaded at runtime from the data dir.

Rust: `client/src/localize.rs` with the same file parser and lookup; every UI string goes
through `tr!("...")` with DDNet's English source text where DDNet has that string, so
DDNet's files translate it unchanged; strings only DDNet-rs has (island, watermark,
HUD editor, media) need our own file (§13.12 q5). Language picker in settings; Russian
first, then every language DDNet ships.

Verify: `rs_parity lang` links the real `CLocalizationDatabase`: every file in
`data/languages` → every (string, context) lookup and the index/default-language choice
for a list of locales identical (100%). A check that every `tr!` string we use either
exists in DDNet's `russian.txt` or in our own file (no untranslated strings in Russian).

### 13.12 Questions for review

1. **Sound crates:** `cpal` for output; Opus: a pure-Rust decoder (`opus-decoder` + `ogg`,
   decode-only, conformance-tested) or a libopus binding (C build); WavPack: no pure-Rust
   decoder is established (the `wavpack` crate binds the C library and needs CMake), so I
   would port a small WavPack decoder for the modes DDNet's files use (lossless, so 100%
   against libwavpack is a clear bar). OK, or prefer the C bindings?
2. **"Selected asset sounds":** stock DDNet has no sound asset setting; we use
   `audio/*.wv` through the storage order (user dir first), like C++. OK?
3. **Not in your list** but part of stock play: name plates, freeze bars, statboard,
   spectator HUD, MOTD. MOTD goes into the Info tab (13.5). I propose name plates and
   freeze bars with the scoreboard (13.2) and the statboard/spectator HUD in 13.7. OK?
4. **Esc menu items before their feature exists:** hidden until their step lands; the
   Browser tab hidden until M11 (it needs public server lists). OK?
5. **DDNet-rs-only strings:** our own `rust-client/assets/languages/russian.txt` in
   DDNet's format, written by me, loaded over DDNet's file. OK?
6. **Files we write** (demos, ghosts, `ddnet-saves.txt`, screenshots): our own
   `%APPDATA%\DDNet-rs\` (never `%APPDATA%\DDNet`); demos/ghosts in DDNet's dirs listed
   read only in the browsers. OK?
7. **Chat colours:** C++ colours team/server/whisper lines (`cl_message_*_color`). Our
   rule is one accent: icons instead of colours, accent only for highlights. OK, or keep
   the C++ colours as an option?

### 13.13 Answers (2026-11-27; plan approved)

1. **Sound crates:** approved. Our own WavPack decoder must match libwavpack byte for
   byte on every `.wv` in DDNet's data. Opus: if a pure-Rust decoder cannot be bit-exact
   with libopus, a tiny documented (inaudible) tolerance is allowed, verified against
   libopus output.
2. **Sounds** from `audio/` through the storage order: approved.
3. **Name plates, freeze bars, statboard, spectator HUD, MOTD** placement: approved.
4. **Hidden Esc-menu items** until their step (Browser until M11): approved.
5. **Own Russian file** for DDNet-rs-only strings: approved.
6. **Files we write** only under `%APPDATA%\DDNet-rs\`: approved.
7. **Chat:** icons for team/whisper/server lines, plus subtle colour tints like DDNet's
   so the kinds are recognisable at a glance.
- The TClient `cursor` pack stays ignored; custom crosshair packs may come later as a
  separate feature.
- Work order: 13.1 chat, then 13.2 scoreboard, then stop for review.

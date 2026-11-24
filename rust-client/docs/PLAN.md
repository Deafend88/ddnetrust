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

## 10. M7 plan: visual identity and HUD (draft for review, 2026-11-24)

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

**7.9 Dynamic Island** (top centre; states from `island-concept.png`): idle = race
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
session, closing the source hides the player (a checklist done by you).

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

---

## 11. Roadmap after M7 (recorded 2026-11-24, not planned in detail yet)

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

The order between them is decided when M7 is done; antiping comes before any public
server.

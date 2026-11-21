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
M6 adds `winit` + `wgpu` (+ an image decoder). Nothing else unless a milestone proves
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
Started only after M1–M5 are green. Separate crate; plan written then.

---

## 6. C++ changes (kept minimal)

| Change | Why |
|---|---|
| `src/tools/rs_golden.cpp` + 1 line in `TOOLS_SRC` | M2 fixtures |
| `src/tools/rs_parity.cpp` + `TOOLS_SRC` line + prediction sources for that tool | M4/M5 fixtures |

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

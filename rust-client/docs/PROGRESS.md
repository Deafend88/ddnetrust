# Progress

## Done
- M1: recon and `docs/PLAN.md` (reviewed; decisions in §8).
- Git: `origin` = Deafend88/ddnetrust, `upstream` = ddnet/ddnet (push URL disabled),
  branch `rust-client`.

### M2 — primitives (complete)
- `protocol/varint.rs`, `protocol/packer.rs` (incl. UTF-8, sanitize and `str_copy`
  truncation helpers from `base/str.cpp`): 7,300 golden vectors.
- `protocol/huffman.rs`: 5,610 compress/decompress vectors, incl. garbage input and the
  wrapped unsigned bit-counter quirk (hit 23 times).
- `protocol/packet.rs` (chunk header, packet encode/decode, connless, `IsSeqInBackroom`):
  13,299 vectors; packet encodings captured from the real `CNetBase::SendPacket*` over
  loopback UDP. 0.7 packets are rejected on receive (`docs/DECISIONS.md`).

### M3 — headless client (complete)
- Codegen: `scripts/gen_protocol.py` turns `datasrc/network.py` (+ the system ex messages
  in `protocol_ex_msgs.h`) into `protocol/generated.rs`; `protocol/netobj.rs` ports
  `SecureUnpackObj`. Verified: all 87 registered UUIDs, object sizes, 1,520 object
  clamp/fail cases, 2,240 message accept/reject cases.
- Snapshots: `protocol/snapshot.rs` (CSnapshot layout, UnpackDelta, CRC, external item
  types, UnpackAndValidateSnapshot) + `varint::decompress`. 60 snapshots, 978 delta
  decodes (every error path hit by mutated/garbage deltas) and 60 validations match C++
  byte for byte. A remote-triggerable C++ assert returns -302 instead (`DECISIONS.md`).
- Connection: `net/conn.rs`, sans-IO port of CNetConnection + CNetClient::Update/Send +
  CPacketChunkUnpacker (lazy, so acks advance chunk by chunk like C++). 150 C++
  scenarios (connect/accept with and without token, queue/flush, in-order/duplicate/
  future chunks, wrong token/ack, a resend request, keepalive, close with random
  reasons, disconnect) match line for line. Fixed on the way: a panic on a control
  packet that is only a security token.
- Client: `net/client.rs` (CClient/CGameClient protocol flow: clientver + info,
  capabilities, in-band map download with CRC check, ready, start info, enter game,
  multi-part snapshots with delta/CRC/resync, CSmoothTime game and predicted time,
  input sending as CControls::SnapInput, input timing, ping/ping-ex, periodic ping-ex,
  IsDDNetLegacy/ShowOthers/EnableSpectatorCount), `net/mod.rs` (UDP glue),
  `bin/ddnet-headless.rs` (loopback servers only, by design for now).
- `tests/server.rs`: full session against the local `DDNet-Server` (~2 s, stable over
  repeated runs): online, map matches the server's file, walking right moves the
  character, prediction runs ahead of the server, server logs entry, `version=20030`
  and a normal leave (no timeout); its teehistorian holds "DDNet-rs 0.1.0".

### M4 — map + physics parity (complete)
Maps: coverage, LearnToPlay, Gold Mine, Tutorial, Tsunami, ctf1, dm1 (read from
`data/maps/`). `tests/parity.rs` holds everything; data from `src/tools/rs_parity.cpp`.
- 4.1 Map loading: `map/datafile.rs` (CDataFileReader incl. Validate, zlib via `flate2`)
  and `map/mod.rs` (CMap::Load: version/group/layer checks, tilemap upgrades,
  EnsureTileLayerProperties, lazy tile-data validation/tileskip unpacking; CLayers
  physics layer selection). All items, data blobs and the six physics layers match
  C++ on all 7 maps, plus 1,050 mutated map files (same accept/reject, same layers).
- 4.2 Collision: `physics/vmath.rs` (vec2 ops in C++ order, x86 float->int casts) and
  `physics/collision.rs` (every CCollision function the core/prediction calls).
  `rs_parity collision`: 24 boxes x 300 ticks per map driven through MoveBox with random
  steering/jumps/kicks and elasticities, querying move restrictions (stoppers, doors),
  tile/front/tele/speedup/switch/tune lookups, map indices, hook rays and tele-outs
  every tick: 50,400 ticks, 100% identical (positions, velocities, query CRC). No bundled
  map has hook teleporters, so both sides also run coverage/LearnToPlay with all tele-in
  tiles turned into hook tele-ins (242 hook-tele hits): identical too.
- 4.3 Character core: `physics/tuning.rs` (47 params, int hundredths like CTuneParam),
  `physics/teams.rs` (CTeamsCore), `physics/core.rs` (CWorldCore + CCharacterCore: Tick,
  TickDeferred, Move, Write/Read/Quantize, hooks incl. player hooks and hook
  teleports, no PRNG so tele-out 0). `rs_parity core`: 4 interacting cores, 2 runs x 600
  ticks per map (default tuning; changed tuning + team split + solo), jumps 2/3/1/0:
  100% identical full core state on all 7 maps and both hook-teleporter variants
  (player hooks, attachments, ground/air jumps, retracts and hook teleports all hit).
- 4.4 DDNet tile effects: `physics/world.rs` ports the client prediction CGameWorld
  (characters only, insert-at-front tick order, NoWeakHookAndBounce, switch timers) and
  CCharacter (DDRaceTick, tune zones, freeze/deep/live freeze, all HandleTiles tiles and
  switch tiles, old and new speedups, stoppers, anti-skip, jetpack, ninja expiry, weapon
  switching, antiping interference, Read with CNetObj_DDNetCharacter, ResetPrediction).
  `rs_parity world` runs the real C++ prediction (CMake now compiles the prediction
  sources into `rs_parity`): 4 characters, 24 runs x 100 ticks per map, spawned by turn
  on every kind of effect tile/switch/speedup/tune/tele-checkpoint spot, 4 world-config
  modes, local player, team split, super/solo/jetpack/ninja/deep-frozen/tune-override
  characters, random direction/jump/hook/fire/weapon/chat/spec-cam inputs. Every second
  speedup is turned into the new type (no bundled map has one). 100% identical full
  character + core state and switch state on all 7 maps and both hook-teleporter
  variants; 14 hand-made mutations of world.rs are all caught.
  Limits: weapons are not predicted (`m_PredictWeapons` false: FireWeapon, ninja dash,
  projectiles are not ported), and Read needs the DDNet extended object (vanilla path
  not ported). The C++ client prediction never teleports through tele-in tiles.

### M5 — prediction (complete)
- 5.1 Snapshots into the prediction world: `predict.rs` ports the prediction parts of
  CGameClient::OnNewSnapshot (active characters, Evolve of dead-reckoned cores,
  GameInfoEx prediction flags, switch states, solo), UpdatePrediction (strong/weak
  order, advancing the server world with the sent inputs, NetObjBegin/NetCharAdd/
  NetObjEnd incl. keep-hooked) and OnPredict (copy, drop inactive/other-team
  characters, tick to PredGameTick with `Client::input_at` = CClient::GetInput), called
  like CClient::Update (each snapshot switch, then repredict). Stats: the C++
  "prediction error" debug check (re-predicting a tick changed the local core) and
  every snapshot of the local character compared with its prediction for that tick.
  `tests/server.rs prediction_matches_server`: scripted walk/jump/hook on `dm1` for
  10 s: 250 snapshots compared, 0 mismatched, 0 prediction errors (167 repredictions).
  `ddnet-headless` prints the counters.
- 5.2 Interference: `prediction_recovers_from_interference` (dm1): a second client walks
  to ours and hooks it for 8 s (closest 28-30 px, hook attached), then leaves. Over 3
  runs: 1 of 200 snapshots mispredicted while interfering (0-1 prediction errors), 1
  more right after it leaves (its removal), then 0 of the next ~149: prediction recovers
  at the next snapshot, by re-reading the server world like C++. Not compared against a
  live C++ client (none is built here; CLIENT=OFF).
- 5.3 Doors: `physics/door.rs` ports the prediction CDoor (Read/Match, ResetCollision
  incl. the map-configured length via MapSide/MapLength and the map-size cap, Destroy),
  `world.rs` NetObjAdd for laser objects (ExtractLaserInfo incl. EntityEx, IsLocalTeam,
  no-predict flag, doors gated by PredictTiles, insert at front, Keep on Match) and
  NetObjEnd (destroy unkept doors, ResetDoorCollision in map order with switch doors
  last). The collision door grid is `Cell`-based so the server-state world writes it
  through the shared `&Collision` like C++. `predict.rs` feeds lasers in
  SnapCollectEntities order (by id, with their EntityEx).
  `rs_parity doors`: per map 60 snapshots of the doors the server creates (server
  OnEntity/CDoor geometry), every door tile also copied to the front layer (half with a
  conflicting laser length), extra doors on the same origin tiles with other numbers and
  directions, and 200 odd lasers (vanilla + EntityEx, other types/owners/teams,
  no-predict, switch numbers -1/300, origins on solid/death/nolaser tiles, huge and
  zero lengths); items leave, return and move under the same id; PredictTiles off every
  10th snapshot. The door grid after every snapshot is 100% identical on all 7 maps
  (Tsunami/ctf1/dm1 have no switch layer, so C++ has no door grid there: all empty on
  both sides). 17 hand-made mutations of door.rs/world.rs are all caught.
  Live: `prediction_matches_server` now runs `dm1` and `coverage` (tee spawns next to a
  switch door the server closes): 250 of 250 snapshots match on each, 0 prediction
  errors, stable over 3 runs. Without the door reset, coverage had 94 mismatches.

M5 is complete. Not ported (see Later): weapons, teams/tuning messages, the dummy.

### M6 — rendering, input, basic UI (in progress)
- Plan in `docs/PLAN.md` §9, approved 2026-11-22. The four open questions (§9.6) were
  not answered; the plan's defaults apply until they are.
- 6.1 Window and quad renderer: new crate `client/` (binary `ddnet-rs`, wgpu 30.0.2,
  winit 0.30.13, png 0.18). `gfx/batch.rs` ports the C++ quad API (MapScreen,
  QuadsBegin, SetColor/SetColor4 with `NormalizeColor` rounding to 8 bits,
  QuadsSetSubset/SetSubsetFree, QuadsSetRotation around the quad center, QuadsDraw/
  DrawTL/DrawFreeform with the C++ vertex and corner-slot order incl. the freeform
  quirk, blend none/normal/additive, wrap repeat/clamp, Clear with alpha 0).
  `gfx/mod.rs`: wgpu device, one pipeline per blend mode with the GL blend factors,
  non-sRGB targets and textures so blending stays in gamma space like OpenGL, the
  offscreen world target composited 1:1 to the window, resize, `gfx_vsync`, and
  screenshots read back from the world target (RGB PNG, F12). Own `block_on` instead
  of a dependency.
  Verified: `tests/gfx.rs` draws a 64x32 test pattern on the GPU (all three blend
  modes, a 1:1 texture, a subset, repeat and clamp, a rotated quad, a freeform quad,
  corner colours) and every pixel matches the value the C++ GL backend computes for
  the same calls (gradient: corners); 8 hand-made mutations (blend factor, clear alpha,
  colour rounding, freeform order, subset/colour slots, y axis, wrap) are all caught.
  Screenshot round trip and resize tested. The window opens, renders and closes
  cleanly; what it looks like on screen was not checked by eye here (no C++ client yet
  to compare against; that comes with 6.3).
- 6.2 Storage and assets: `client/src/storage.rs` ports CStorage read-only (the
  `storage.cfg` add_path rules incl. CLineReader details, default paths `$USERDIR`
  then `$DATADIR` then `$CURRENTDIR`, data-dir discovery with `--data-dir`/
  `DDNET_DATA_DIR` first and this repo's `data/` as the compiled-in dir, the OpenFile
  path guard incl. the `mapres/../skins/` exception, de-duplicated listing). We never
  write to `%APPDATA%\DDNet`; screenshots go to `%APPDATA%\DDNet-rs\screenshots`.
  `image.rs` ports CImageLoader::LoadPng (memory and file variants: the file one
  rejects gray images) and ConvertToRgba (gray becomes white with alpha). `assets.rs`:
  the C++ fallback chain of game/particles/emoticons/hud/extras (name, name as a
  directory, default, plus the default image as fallback), skin listing and names
  (CSkin::IsValidName, `.png` files only), skins from `skins/` or already downloaded
  ones, and map images. `ddnet/src/map/images.rs` ports the OnMapLoadImpl decisions
  and GetDataString; `Map::client_data` ports CLayers::InitTilemapSkip, which the C++
  client runs at map load and which changes the `m_Skip` byte of game/design tile data
  (found because a map image pointing at tile data hashed differently).
  Verified: `rs_parity png` (rs_parity now also links engine-gfx) on 64 crafted PNGs
  (every colour type and bit depth, tRNS, Adam7, broken files from
  `scripts/gen_png_corpus.py`) and all 546 PNGs in `data/`: same success, size,
  format, pixels and RGBA conversion as C++ for all 610. `rs_parity mapimages`: the
  map image decisions on all 7 maps, unchanged and with 100 runs of changed image and
  layer item ints each (3,000+ image results): identical. Storage, asset chain and
  skin rules tested on temp dirs; every image the 7 maps use loads from the repo's
  `data/`, and without a data dir nothing panics. Deviations in DECISIONS.md.
  Debug builds now use `opt-level` 1 (dependencies 2): the asset test went from 41 s to
  7 s and the physics parity tests from 11 s to under 1 s, still 100% identical.
  The window shows the default game skin and tee skin loaded through storage.
- 6.3 Map rendering (in progress). Done so far: `render/envelope.rs` ports
  CMapBasedEnvelopePointAccess, FindPointIndex, SolveBezier, RenderEvalEnvelope and
  CEnvelopeState::EnvelopeEval (float/double steps kept); `render/mod.rs` ports
  CalcScreenParams and MapScreenToWorld. Verified: `rs_parity envelopes` (every
  envelope of the 7 maps plus the invalid indices around them, 200 samples each,
  through the real RenderEvalEnvelope; 10,000 results) and `rs_parity
  envelopes-synthetic` (500 random envelopes with every curve type, unknown ones,
  equal/decreasing times and Bezier tangents; 20,000 results) are bit-identical;
  `rs_parity screen` (5,000 random cameras/aspects/zooms/parallaxes through the real
  IGraphics::MapScreenToWorld) is bit-identical. 7 of 8 envelope mutations are caught
  (the Bezier-item size check is not reachable: no bundled map has Bezier data).

## Later
- Weapon prediction (`m_PredictWeapons`/AntiPingWeapons: FireWeapon, ninja dash,
  projectiles, lasers, pickups) is out of scope for now.
- Not ported for prediction yet: `Sv_TeamsState` (teams), `Sv_TuneParams` (non-default
  tuning), the dummy, antiping smoothing/pre-inputs, vanilla servers without
  `DDNetCharacter`/GameInfoEx.

## Not yet sent by the Rust client (a default C++ client sends these)
- `Cl_ShowDistance`, `Cl_CameraInfo`: need the camera/window (M6).
- `/timeout <code>` chat after ~50 snapshots: needs the timeout seed config.
- Connectionless server-info request after entering the game (server browser format).
- HTTP map download (deferred, PLAN §8).
- Not done: a byte-level capture of the C++ *client's* handshake to diff against ours
  (PLAN §5 M3). Needs a C++ client build (`CLIENT=ON`, SDL); the pieces are covered by
  golden vectors and the server accepts our session.

## Manual testing
See `docs/TESTING.md`: tests, local server, `ddnet-headless --script` and watching it
with the normal DDNet client.

## How to regenerate golden vectors
From a shell with the Visual Studio 18 x64 environment (`vcvars64.bat`), repo root:

    cmake -G Ninja -S . -B build-rs -DCMAKE_BUILD_TYPE=Release -DCLIENT=OFF -DDOWNLOAD_GTEST=OFF
    cmake --build build-rs --target rs_golden
    build-rs\rs_golden.exe rust-client\tests\golden

Golden files live in `rust-client/tests/golden/` (not `tests/data/`: the root
`.gitignore` ignores every `data/` directory).

## Local test server
`build-rs/DDNet-Server.exe` (CMake target `game-server`). Run it only with cwd set to a
scratch directory whose `storage.cfg` contains just `add_path $CURRENTDIR` and
`add_path $DATADIR`, so it never touches the real `%APPDATA%/DDNet`, and with
`sv_register 0`, `bindaddr 127.0.0.1`. `tests/server.rs` does exactly that. Never
connect to public servers until the user says so.

## Regenerate parity data
Build the `rs_parity` target like `rs_golden`, then per map (repo root):

    build-rs/rs_parity.exe map "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.txt <scratch dir>

(spaces in `<name>` become `_` in the output file name), and

    build-rs/rs_parity.exe collision "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.collision.txt
    build-rs/rs_parity.exe collision "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.collision-hooktele.txt hooktele

(hooktele only for coverage and LearnToPlay; add `verbose` to see every query value).
`core`, `world` and `doors` work like `collision` (`<name>.core.txt`, `<name>.world.txt`,
`<name>.doors.txt`; `-hooktele` variants of core and world for coverage and LearnToPlay;
`doors` takes no flags). M6.2 data (repo root):

    python rust-client/scripts/gen_png_corpus.py
    build-rs/rs_parity.exe png rust-client/tests/golden/png.txt rust-client/tests/golden/png data
    build-rs/rs_parity.exe mapimages "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.mapimages.txt
    build-rs/rs_parity.exe envelopes "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.envelopes.txt
    build-rs/rs_parity.exe envelopes-synthetic rust-client/tests/golden/envelopes.txt
    build-rs/rs_parity.exe screen rust-client/tests/golden/screen.txt

All modes are deterministic: the data regenerates byte for byte.

## Regenerate protocol code
    python rust-client/scripts/gen_protocol.py   # needs rustfmt on PATH

## Known issues / blockers
- Resend buffer is a byte budget, not C++'s ring buffer (differs only with ~32 KiB
  unacked data; `ponytail:` comment in `net/conn.rs`).

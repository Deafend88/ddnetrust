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

### M6 — rendering, input, basic UI (complete: 6.1-6.8)
- Plan in `docs/PLAN.md` §9, approved 2026-11-22. The open questions (§9.6) are
  answered in §9.7.
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
- 6.3 Map rendering. Part 1: `render/envelope.rs` ports
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
  Part 2: `render/map.rs` ports CMapRenderer::Load/Render (background up to the game
  layer, foreground from it), CRenderLayerGroup (clip rectangle with the C++ rounding,
  parallax via MapScreenToWorld), CRenderLayerTile (visible tiles, the flip/rotate
  texture-coordinate table, layer colour times colour envelope, the stretched edge
  tiles outside the map incl. the C++ gaps for 1-tile-wide/high layers) and
  CRenderLayerQuads (vertex order 0,1,3,2, 8-bit colours times envelope colour,
  rotation around point 4, position envelope). `gfx` gained 2D-array textures for
  tilesets (Texture2DTo3D), mipmaps, the C++ LOD bias (-0.5), float vertex colours and
  per-draw scissor; tile arrays always use clamp-to-edge like the C++ array sampler
  (using the wrap state instead caused a visible grid). The client computes
  IntraGameTick/IntraGameTickSincePrev/PredIntraGameTick like CClient::Update, and
  `ddnet-rs` connects to a local server and draws the map around the local tee
  (or `--view`/`--follow`).
  Verified against the C++ client built from this repo (`build-rs-client`, Vulkan,
  same GPU) with `scripts/compare_screenshots.ps1`: both clients on one local server
  at the same moment, 800x600, tee masked. Matching pixels (within 8/255): Gold Mine
  99.78%, Tsunami 99.64%, LearnToPlay 99.53%, Tutorial 99.50%, dm1 99.06% (99.966%
  with its tees and pickups masked; the rest is the edge of a moving cloud). Every
  remaining difference seen in the diff images is a tee, a pickup, a door laser, a
  turret explosion or animation captured a few ms apart. Not compared yet: ctf1 (the
  followed C++ tee is out of our snapshot range until 6.7 sends Cl_ShowDistance) and
  coverage (mostly entities; its map layers match, camera 2 px off until 6.5 adds
  prediction-based centering).
- 6.4 Tees: `scripts/gen_content.py` generates `client/src/render/content.rs` from
  `datasrc/content.py` (sprite sets, sprites, animations, weapon specs; floats written
  like the C++ literals). `render/anim.rs` ports CAnimState. `image.rs` gained the
  bicubic ResizeImage, CheckImageDivisibility (with its resize), ConvertToGrayscale and
  the sprite-cutting helpers; `skin.rs` ports CSkins::LoadSkinData (divisibility, RGBA
  and size checks, blood colour, CheckMetrics, the colourable grayscale variant) and
  LoadSpriteTexture's fallback for fully transparent sprites. `render/tee.rs`: skin and
  game-skin/emoticon sprite textures, CTeeRenderInfo::ApplyColors (UnclampLighting),
  the tee and weapon quad containers, RenderTee6 (outline pass, body, eyes per emote,
  feet incl. the air-jump indicator and flipped feet) and RenderHand6.
  `render/players.rs`: CPlayers::OnRender (hooks first, local last), RenderHook (head,
  chain, hand) and RenderPlayer (animation choice incl. in-air/sit/walk/run, hammer and
  ninja swing, weapons incl. recoil and hands, chat bubble and AFK "zzz"), skins from
  ClientInfo (invalid/x_ names -> default), team colours, the ninja skin for ninja and
  frozen tees. `Batch::container_quad` draws quad-container quads like the Vulkan
  backend. Drawn from the current snapshot (previous = current); 6.5 interpolates.
  Verified: `rs_parity anims` (idle, every animation set and added at every keyframe
  time and 100 random times, 5,000 RenderPlayer blends from random positions incl. the
  walk/run times: 8,621 states), `rs_parity colors` (5,269 packed colours with and
  without UnclampLighting) and `rs_parity skins` (all 101 skins in `data/skins`, as is,
  cropped so they need the bicubic resize, and as RGB: 404 results) are bit-identical
  (`tests/tees.rs`); 12 of 15 hand-made mutations are caught (the other 3 give
  identical floats or bytes). A GPU test draws a custom-coloured tee and checks the
  body texel times the colour. Screenshots against the C++ client (scene names in
  `compare_screenshots.ps1 -Name`, server `sv_max_afk_time 0`): Gold Mine with the
  C++ tee in 5 skins/colour sets, one per eye emote (pain/bluekitty, happy/default
  custom red, surprise/redstripe, angry/coala custom, blink/santa_cammo custom black):
  99.67-99.78% each, no mask; the only tee difference is a sparkle particle C++ draws
  around its own tee. Gold Mine AFK (zzz emote, sit): 99.75%. LearnToPlay 99.74%,
  Tutorial 99.45% (its keyboard quad edges, as in 6.3, plus the sparkle).
  Not checked by screenshot: a left-facing tee (the C++ tee's aim follows its mouse,
  which the script cannot move; comes with 6.7), hooks and walking tees (moving, so
  they need 6.5's interpolation to compare), dm1/ctf1 (the C++ tee spawns outside our
  snapshot range until 6.7 sends Cl_ShowDistance/Cl_CameraInfo).
  Found on the way: integration-test builds also rebuild `target/debug/ddnet-rs.exe`,
  so run the screenshot script only after a plain `cargo build`.
- 6.5 Interpolation: `predict.rs` keeps every active character's previous and
  current snapshot core, both evolved like OnNewSnapshot (Prev to PrevGameTick, Cur to
  GameTick, 3 s limit), with their DDNet objects; `predict` keeps the local core at the
  start and end of the last predicted tick (C++ m_PrevPredicted/m_Predicted); and
  `Predictor::rendered` ports UpdateRenderedCharacters for `cl_antiping 0`: others
  between the snapshots with IntraGameTick, the local tee (when C++ Predict() holds: a
  local character, not spectating/paused, world not paused) between its predicted cores
  with PredIntraGameTick. `net/client.rs` `intra_ticks` is the CClient::Update fraction
  code as a pure function. The renderer uses it: positions, hook and velocity with the
  predicted or plain intra tick, other players' aim between the previous and current
  DDNet target (`other_target_angle`, GetPlayerTargetAngle), the local tee's freeze from
  its prediction, and the camera on the rendered local (or followed) tee, keeping the
  last center while it is gone (C++ m_LocalCharacterPos). `ddnet-rs` now runs the
  predictor (its collision is leaked once per run so the predictor can borrow it).
  Verified: `rs_parity interp` (20,000 random tick pairs and game/predicted times
  through the CClient::Update fractions, 20,000 position mixes, 20,000 aim angles incl.
  crossing the top and DDNet targets with and without a previous one: 60,000 results)
  is bit-identical (`tests/tees.rs`); 5 of 7 mutations caught, the other 2 are
  equivalent rewrites. Live, `tests/server.rs rendered_positions_are_smooth`: two
  scripted tees on dm1, the renderer's positions sampled every ~1 ms for 6 s: largest
  step per frame 2.3 px for the predicted local tee (21.6 px without interpolation) and
  14.9 px for the other tee (32.8 px; snapshots come every 2 ticks), stable over runs.
  Screenshots unchanged: Gold Mine 99.73%, LearnToPlay 99.74%. coverage 95.7%: its
  entities (6.6) plus sub-pixel differences along tile borders (no global offset; a 6.3
  map issue, see Next).
  Not compared by screenshot: moving tees and hooks (the two windows are captured a few
  ms apart; the C++ client also ignores `+hook` on its command line).
- 6.3 follow-up, coverage tile borders: not a renderer bug. On coverage the C++ tee
  keeps moving after spawning (it slides, later swings on an endless hook), so the two
  clients, captured a few ms apart and centering on predicted vs interpolated
  positions, look at views a fraction of a pixel apart; tile edges then differ. Checked
  with a fixed reference: one C++ capture, and `examples/maprender.rs` (new: renders a
  map at a given center on the GPU without a server; `ddnet-rs --screenshot-after`
  now prints its camera center) at candidate centers. At the C++ view (577.7, 1105)
  the dense tiles match 99.68% and the whole picture 99.974% with only the entities
  (6.6) and the tee masked (96.9% unmasked); at the integer center 578 it is 85.8%.
  On the way, ruled out: LOD bias (-0.7 to 0), mip filters (box is best; floor,
  nearest, tent and shifted boxes are worse), PrintWindow capture (identical to our
  readback), display scaling (96 DPI), map group parallax.
- 6.6 Entities: `ddnet/src/items.rs` ports ExtractProjectileInfo (vanilla, legacy
  DDRace and DDNet objects), ExtractPickupInfo, ExtractLaserInfo (the door code now uses
  it too), CalcPos and SnapCollectEntities' order (by id, with the EntityEx of the same
  id). `client/src/render/items.rs` ports CItems for `cl_antiping 0`: projectiles (time
  from PrevGameTick and GameTickTime, CalcPos, rotation along the flight, grenade spin),
  pickups (interpolated, flips/rotation, the ninja offset, the bobbing), lasers with
  the C++ default colours (rifle/shotgun/door/freeze/dragger/gun, bounce-delay width,
  door bricks, dragger pulleys and orbs, freeze hexagon and snowflake, splat heads),
  the switch blinking (`Predictor::switch_inactive`: switch state team, super) and
  flags (at the carrier, no interpolation across capture/return). `tee.rs` `Sprites`
  replaces `GameSkin`: every sprite of the game, particles, emoticons and extras images
  by C++ sprite id (zero-size sprites have no texture, as in C++).
  Found and fixed on the way (a 6.3 bug): the map's border tiles (outside the map)
  use the C++ `tile_border` fragment shader, which wraps the coordinates (`fract`) and
  picks the mip level from the unwrapped ones; we had stretched the edge texels.
  Verified: `rs_parity items` on coverage (12,000 projectile/pickup/laser objects of
  every kind, with and without EntityEx and tune zones, through the real C++
  extractors, plus CalcPos at random times and tunings) is bit-identical
  (`tests/parity.rs`); 8 of 9 mutations caught, the other gives identical floats.
  Screenshots, with two new tools: the server option `sv_show_all_default 1` (the
  server sends everything to every client, so the other client's surroundings are in
  our snapshots) and `compare_screenshots.ps1 -Subject "<ddnet-headless args>"` (a
  ddnet-headless tee joins first, the C++ client spectates it with
  `cl_run_on_join spec subject`, DDNet-rs follows it: both cameras on the same tee even
  where the C++ tee would move; `ddnet-headless --fire 1` makes it shoot). Masked only:
  the tees, and pickups, whose bobbing runs on each client's own clock (C++ s_Time):
  ctf1 100.000% (armor and freeze pickups), dm1 99.980% (heart, shield, weapon
  pickups), coverage 99.998% (door lasers and their heads, the left map border;
  unmasked 98.2%, the rest being particles, which C++ draws and M6 does not). The gun
  bullet draws like C++ (checked by eye; bullets live ~0.1 s, so a capture rarely has
  one in both pictures). No regressions: Gold Mine 99.67%, LearnToPlay 99.73%, Tsunami
  99.95%, Tutorial 99.47% (its keyboard quad edges, as since 6.3).
  Also fixed in the compare script: with exactly one `-CppArgs` entry PowerShell
  concatenated it with the connect argument (why `+hook` failed before).
  Not verified live: flags (this repo's server has no CTF game type), dragger, freeze
  and gun lasers (none in view at the subject's spawn), projectile tune zones (no
  `Sv_TuneParams` yet).
- 6.7 Mouse, binds, camera, camera messages, settings. `client/src/config.rs` ports the
  C++ console line parsing (ExecuteLineStroked's `;`/`#`/quote splitting, ParseStart,
  ParseArgs with `str_toint`/`strtol`, ColorParse for packed numbers), the config
  variables we use (C++ names, defaults, clamping, `str_copy` UTF-8 truncation, colour
  pack with DARKEST_LGT) and CBinds (SetDefaults + SetDDRaceBinds, GetBindSlot with
  `str_next_token`'s quirks, GetKeyBindName, OnInput with modifier masks and active
  binds, the saved-config format). `keynames.rs` is generated from
  `src/engine/client/keynames.cpp` (`scripts/gen_keys.py`). `controls.rs` ports
  CControls (OnCursorMove, ClampMousePos, Get{Min,Max}MouseDistance, the `+left`,
  `+right`, `+jump`, `+hook`, `+fire`, `+weapon1-5`, `+nextweapon`, `+prevweapon`
  commands, SnapInput's target and input-type flags) and maps winit keys/buttons/wheel
  to C++ key ids. `camera.rs` ports CCamera for a playing player (CCubicBezier,
  ChangeZoom, `zoom+`/`zoom-`/`zoom`, UpdateCamera's zoom smoothing and dyncam offset,
  OnRender's center). `net/client.rs` sends `Cl_ShowDistance`/`Cl_CameraInfo` like
  CGameClient::OnNewSnapshot (zoom reported while smoothing, spectators keep the last
  values, only on change); `ddnet-headless` reports a default 16:9 camera and has
  `--aim X,Y`. `ddnet-rs` reads `settings_ddnet.cfg` once from the storage paths (read
  only, checked unchanged by hash) then `%APPDATA%\DDNet-rs\settings.cfg`, joins with
  that name/skin/colours, uses raw mouse motion with a hidden confined cursor, runs
  key/mouse/wheel events through the binds, zooms, and centers on tee + dyncam offset.
  Esc frees the mouse, a click grabs it (until the 6.8 menus); `--no-grab` for scripts.
  Verified: `rs_parity camera` runs the real CControls/CCamera
  (`src/tools/rs_parity_camera.cpp`: both components placement-constructed in a zeroed
  CGameClient-sized block, private members reached by explicit instantiation; only
  CGameClient::IsWorldPaused and CScoreboard::IsActive stubbed, not reached): 5,000
  mouse cases, 400 sequences x 80 frames of camera updates with mouse moves and zoom
  commands, 2,000 camera-message bodies packed with CNetMsg_*::Pack: 100% bit/byte
  identical (`tests/input.rs`). `scripts/cpp_settings.ps1` has the C++ client load
  `tests/golden/settings/corpus.cfg` (quotes, escapes, `;`, `#`, modifiers, `&N` keys,
  invalid and out-of-range values, a name cut mid-UTF-8, DDRace binds with level 0)
  as its settings file and save it: our variables and all 512+ bind lines are
  identical, as are the default binds. Mutations: all caught except 2 equivalent ones
  (10 camera/mouse, 9 settings/binds).
  Live (`tests/server.rs camera_messages_set_network_clipping`): two tees stand at their
  dm1 spawns (1280 px apart) while one steps its zoom 0.4..2.5; the other tee is in its
  snapshots exactly when C++ `UpdateNetworkClipRadius` + `IsSnappingCharacterInView`
  say so (flips between zoom 1.3 and 1.4; without camera info it would flip near 0.9,
  without the show distance never), and the teehistorian holds the exact bytes of
  every message sent. Fails with the messages switched off.
  `rendered_positions_are_smooth` had become flaky; two real causes, no bug: (1) the
  server now clips to our 16:9 view (radius 1035 px instead of its 1200 px default), so
  the other tee (~1250 px away) is seen less: the test runs 10 s; (2) rendering runs a
  tick behind the newest snapshot with snapshots every 2 ticks, so the other tee is
  extrapolated (IntraGameTick up to ~1.5, unclamped like C++) and snaps onto the new
  interval at each switch. The test now holds every non-switch frame to half a
  snapshot step (observed ~2-3 px, stricter than before) and switch frames (<5%) to one.
  Screenshots (plain `cargo build` first; both clients on default settings): Gold Mine
  99.63% (rotating gear, C++ cursor), LearnToPlay 99.81%, Tutorial 99.50%, Tsunami
  99.93% (88% when the C++ client happens to open its menu on losing focus: a capture
  artefact). New `compare_screenshots.ps1 -RustSubject`: DDNet-rs plays as "subject",
  the C++ client spectates it, no `sv_show_all_default`: dm1 99.34%, ctf1 99.74%,
  unmasked; every entity in view arrives (differences: pickup bobbing, sun rays). The
  left-facing tee (`-Subject "--aim -100,0"`) draws like C++. `ddnet-rs` against a local
  server with the real user dir joins as the configured name, colours applied.
  Not checked: the same raw mouse motion giving the same aim as the C++ client by hand
  (the scaling itself is covered numerically); the spectator camera is not ported.
- Golden/parity data is protected by `rust-client/.gitattributes` (`-text`): stored and
  checked out byte for byte (the C++-saved configs keep their CRLF).
- 6.8 Minimal menus (egui 0.37 + egui-wgpu/egui-winit 0.37 without default features, on
  wgpu 30 / winit 0.30.13; DDNet's `fonts/DejaVuSans.ttf` from the data dir, no menus
  without it). `client/src/ui.rs`: the egui glue (drawn onto the frame after the world
  composite) and three pages: a start screen (server address, loopback only, Connect,
  Settings, Quit, the last connection error), the in-game Esc menu (Back to the game,
  Disconnect, Settings, Quit) and the settings (name, skin from storage, mouse
  sensitivity, custom colours on/off, dyncam, vsync applied live, data dir for the next
  start, key readers for move left/right, jump, hook, fire: C++ menus_settings_controls.cpp
  rebinding). Every setting change is a C++ console line, so it gets the config
  clamping/truncation. Leaving the settings writes our `%APPDATA%\DDNet-rs\settings.cfg`
  (every variable and bind, so it overrides the C++ file); `%APPDATA%\DDNet` is never
  written. `main.rs` now has a `Session` per connection: connect from the start screen
  or the command line, disconnect back to the start screen (also on server errors),
  reconnect, quit. The address argument is optional.
  C++ menu input rules: SnapInput while a menu is open (PLAYERFLAG_IN_MENU, ResetInput
  unless the DDRace input bug, the sent copy keeps the previous target, sent once a
  second or on flag change; `net/client.rs` also gained the ninja send rule), binds
  while menus are open (presses go to the menu, releases end active binds without
  running them). The mouse is grabbed only while playing with the menu closed.
  Settings files: commands DDNet-rs does not use (other stock settings, TClient's) are
  skipped silently, one summary line with the count; binds to unknown commands are kept
  like stock DDNet does.
  Verified: `tests/input.rs` `menu_input_follows_cpp` (SnapInput menu branch, binds with
  menus open, the DDRace input bug) and `own_settings_file_round_trips` (our file
  reloads to the same settings, on its own and over the C++ corpus, incl. a value set
  back to its default); all gates. Live, driven by keyboard against a local server
  (Esc, Tab, Enter): start screen and in-game menu render, Disconnect -> server logs the
  leave, Connect from the start screen -> enters again, Quit -> leaves, process exits;
  the settings page renders the current values and nothing was saved without Back.
  Screenshots unchanged: LearnToPlay 99.86%, ctf1 `-RustSubject` 99.75%. Played by the
  user: not yet for 6.8 (the 6.7 build was).
#### M6 status (2026-11-24)
Done and pushed: 6.1-6.8, M6 complete. After it: the 6.8 map-texture leak is fixed (map
textures are freed when a connection ends; `tests/gfx.rs map_textures_are_freed`).
M7 (visual identity and HUD) is approved (PLAN.md §10, answers §10.6); the roadmap after
M7 (antiping first, before any public server) is in §11. M7 status: see below.

Open items from 6.6 (not checked live, code ported line by line):
- Flags: this repo's DDNet-Server only runs the "ddnet" game type, which never sends
  flags. Needs a server that does (not this repo's) or a test that feeds flag objects.
- Dragger, freeze and gun lasers: none were in view at the `-Subject` spawn on
  coverage. Next try: `-Subject` runs on maps/spawns where they are on screen.
- Projectiles (and laser widths) in tune zones: every zone uses the default tuning
  until `Sv_TuneParams` and the tune zone messages are handled.
- Not in M6 at all: particles (trails, explosions, powerup shine), muzzle flashes,
  emoticon messages, the hook collision line, HUD and cursor.
Open from 6.7/6.8: the spectator camera (free view, auto-sync, smoothing),
`+showhookcoll`, chat/emote/spectate/kill/vote binds (ignored), hex/named colours in
configs, checking the aim against the C++ client by hand. Menus: name/skin changes
apply on the next connect (no Cl_ChangeInfo yet), no colour pickers, no clipboard, the
key readers change only a command's first bind. Leaks fixed: map textures are freed when
a connection ends, and the collision is shared with `Rc` (GameWorld and Predictor own an
`Rc<Collision>`, no lifetime) and dropped with the last user (commit 4cf8235ef;
`prediction_matches_server` checks it with a `Weak`).

Screenshot workflow reminders: run a plain `cargo build` before
`scripts/compare_screenshots.ps1` (integration-test builds overwrite
`target/debug/ddnet-rs.exe`); use `-Subject` and `-ServerArgs "sv_show_all_default 1"`
for scenes that do not stay still, or `-RustSubject` to check what the server sends us;
`examples/maprender.rs` renders a map at a given
center for fixed-view checks; mask pickups (they bob on each client's own clock).

### M7 — visual identity and HUD (done: 7.1-7.11; 7.11 waiting for the user's review)

Pushed and verified: the plan and its review answers (PLAN §10, §10.6), the collision
leak fix, and 7.1 (fonts + icons, design system, menus + gallery, the review tweaks,
the UI-rules review).

#### 7.1 status (2026-11-24): design system, done, gallery approved by the user
- `assets/fonts/`: Inter 4.1 Regular/Medium/SemiBold (rsms/inter's Inter-4.1.zip,
  `extras/ttf`) and Phosphor 2.1.2 regular (npm `@phosphor-icons/web`), with
  `Inter-LICENSE.txt` (OFL 1.1) and `Phosphor-LICENSE.txt` (MIT); embedded with
  `include_bytes!` in `style.rs`.
- `scripts/gen_icons.py` -> `client/src/icons.rs` (40 icon constants + `ALL`); it needs
  Phosphor's `src/regular/style.css` from the same npm package as an argument (the CSS
  is not in the repo).
- `style.rs` (tokens: colours with one blue accent, radii, spacing, type scale, motion;
  `Glass` off/subtle/full with panel fills 100/85/70%; `fonts()`; `install()` builds the
  egui style), `anim.rs` (critically damped `Spring` with the exact solution,
  `ease_out`; unit tested), `widgets.rs` (panel, pill, icon, text, stat, dot,
  button/accent_button with an optional icon, toggle, slider, segmented, popover).
- `ui.rs`: `Painter` (egui + egui-wgpu, no window; used headless by the gallery) split
  from `Ui` (adds winit input); the menus always exist (embedded fonts); the 6.8 menus
  restyled with the components; the settings grid is in a vertical `ScrollArea`
  (window height - 200 pt), Back stays below it.
- `config.rs`: our variables `rs_glass` (0-2, default 1) and `rs_ui_scale` (50-200 %,
  default 100); `main.rs` reinstalls the style / zoom factor when they change.
- `gallery.rs` + `examples/gallery.rs` (every component on one 960x640 page, headless,
  byte-identical run to run) and `tests/gfx.rs gallery_matches_reference` against
  `tests/golden/gallery.png` (within 2/255). The gallery's buttons carry icons, so the
  icon bug below would fail the test.

Fixed in this session: (1) icons in button labels drew as "ÿ": Inter maps 745
private-use codepoints itself (U+E270 = `ydieresis.ss07`), so the Phosphor fallback
behind it was never reached for those; icons are now a separate atom in the `icons`
family (`egui::Button::new((icon, text))`) and the text families have no Phosphor
fallback. (2) The settings page overflowed 600 px: now scrolls.
`/ponytail-review` cut `paint_icon`, `text_size`, `ease_in_out` (unused) and a
duplicate `style::install` in the gallery.

Verified: `cargo build`, `clippy --all-targets -D warnings`, `fmt --check`,
`cargo test --workspace` (66 passed); the menu capture check at 800x600 (start screen,
Esc menu, settings: icons right, nothing cut) and the keyboard connect / Esc menu /
disconnect / reconnect / quit flow on a local server (port 8305, scratch dir).

User review round (applied, pushed): pill-shaped controls, card radius 24, more
padding, larger type (body 15) and icons (17), grey regular logo-pill text, popover
shadow and gap, fainter borders, no separators. Then the CLAUDE.md UI rules (spacing
scale 4..32 only, tokens only, one accent, essential info, frontend-design skill) and a
review of the gallery and menus against them: menus are one column of full-width
controls without the egui title bar; keyboard focus is a 1 px ring. Two focus bugs
fixed on the way: a reopened menu kept a stale focus (egui runs no passes while menus
are hidden; `Ui::hidden` drops it), and egui never got `Focused` events in a client
started straight into a server (now always forwarded). The frontend-design skill does
not load as a skill in this setup; its SKILL.md is read from the path in CLAUDE.md.

#### 7.2 status (2026-11-24): glass, done and approved by the user
- `glass.rs`: dual-Kawase pyramid of the world target (Rgba16Float; subtle 3 levels =
  1/8 size, full 5), recorded in `Painter::paint` before egui's pass; panel backdrops
  are egui-wgpu paint callbacks (`glass::shape(rect, radius, liquid)`), so they keep
  their panel's z-order and clip. Per-panel uniform buffer created in `prepare`
  (`ponytail:` note: shared buffer if panels ever number in the hundreds); `paint`
  resets the viewport egui sets to the callback rect.
- `widgets::glass_frame` (private): shadow, backdrop (inset 1 px under the fill, or the
  sharper cut showed as a bright rim), fill, border; used by `panel`, `glass_pill` and
  `popover`. The menu card is a glass panel (`Window::frame(Frame::NONE)`).
- Liquid Glass only for the island at full strength (`glass_pill(.., liquid: true)`):
  inward refraction near the edge (10 px over a 14 px band), specular rim from the top
  left, tint darkening 20-55 % by the backdrop's mean luminance (5 samples of the
  deepest level), light fill `Glass::liquid_fill()`.
- `Painter::set_glass` replaces the style reinstall in `main.rs`.
- Verified: `tests/glass.rs` (5 GPU tests, see the commit; both refraction and mask
  tests fail when the shader is broken on purpose); all gates (71 tests); live menu
  checks with glass. `examples/glass.rs <map> <x> <y> <out dir>` renders the gallery
  at each strength over a map (review: dm1 at 900 500) and times the passes at
  2560x1440, release build: subtle 0.16 ms, full 0.19-0.22 ms (target 0.5 ms).

Review round 2: a lighter, more saturated full (7 levels, x1.25 saturation, lighter
tint) was tried and reverted at the user's request (washed out); full stays 5 levels,
no saturation, SURFACE at 70 %. Kept: secondary text per strength via egui's
`weak_text_color` (`Glass::text_secondary`: 172, 204 at full; captions 8.0 / 5.9 / 5.6:1
off / subtle / full on the dm1 render) and the logo text 114 grey (4.6:1). Subtle is
unchanged, still the default. The user judges Liquid Glass live with the island
animations in 7.9.

#### 7.3 status (2026-11-24): HUD system, done and approved (commit 3831fc4a3)
- `hud.rs`: `Widget` trait (`id`, `default_placement`, `show(ui, &HudData,
  &HudConfig)`), `Anchor` (9 points, names `top_left` .. `bottom_right`, mapped to
  egui `Align2`), `Placement` (anchor + offset in points + enabled), `HudConfig`
  (`hud_widget <id> <anchor> <x> <y> <enabled>`, `hud_set <id> <key> <value>`, executed
  by `Settings::execute_line`, saved by `Settings::serialize`; unknown ids/keys kept),
  `HudData` (borrowed: client, predictor, followed tee, this frame's game messages),
  `hud::show` (an egui `Area` per enabled widget, `anchor(align, offset)`, not
  interactable, background order; egui keeps areas on screen).
- `main.rs`: `App::hud` (empty until 7.5); the frame drains `client.messages` into the
  HUD's events (fixes: the graphical client never drained them, so they piled up for
  the whole connection); the HUD draws under the menus once the map is loaded and
  only when there are widgets.
- Tests `tests/hud.rs` (4): config round trip, malformed lines, 9 anchors x 2 DPI
  against an independent table (a swapped anchor fails it), off-screen offsets clamp.
  Gates: 75 tests. Live menu check unchanged.

Deviations from PLAN 7.3 (accepted by the user on 2026-11-24; for 3 the user wants the
real HUD on/off input test added in 7.5):
1. No separate HUD `Painter` facade: widgets draw with egui + `widgets.rs` (panel,
   glass pill, text, icon, stat). A facade is the Lua surface; it is built with Lua
   (after M7), when its shape is known. Image and progress components come with their
   first widgets (7.9/7.10).
2. `HudData` holds what has a source today; fps (C++ averaging), server address,
   local time (needs a time zone source) and now playing/track change come with the
   widgets that read them (7.5, 7.10).
3. The "inputs byte-identical with every widget on/off" server test is replaced by
   construction: widgets get `&HudData` and an egui `Ui` only (no client, controls or
   settings), and the HUD pass runs after `snap_input`/`net::pump` in the frame. A
   real on/off comparison needs scripted, timing-identical runs; if the user wants it,
   it fits once widgets exist (7.5).
4. Widget `name`/`icon` (editor list) are added with the editor (7.4).

#### 7.5 part 1 status (2026-11-24): watermark widget (commit 29751f3cf)
Built before the editor (the user chose this order) so the editor drags a real widget.
- `watermark.rs`: `Watermark` widget (top left 16, 16) and `draw` (the gallery uses the
  same pill, so `gallery_matches_reference` guards its look). Items: user (local
  `ClientInfo` name), server (address as typed), ping (local `PlayerInfo` latency,
  clamped 0..999 like the C++ scoreboard), fps (`round_to_int(1 / m_FrameTimeAverage)`,
  average `* 0.9 + frame * 0.1` like `CClient::Run`, start 0.0001), uid (local id), tps
  (50). Defaults on: user, ping, fps. `hud_set watermark compact|<item>|order`.
- `HudData` gained `server`, `frame_time_average`, `glass`.
- Not yet (rest of 7.5): time (needs a time zone source) and speed (7.6 movement info);
  the side-by-side captures with the two reference PNGs; the HUD on/off input test.

#### 7.4 status (2026-11-25): HUD editor, done (commit f530ed00d), review fixes below
- `hud_editor.rs`: `drop` = clamp on screen, snap (screen edges and centre, other
  widgets' left/right/top/bottom edges, within 8 pt; Shift = free), nearest anchor by
  screen thirds with the offset recomputed (same screen position), offsets on the 8 pt
  grid on axes that did not snap. `Editor::show` draws enabled widgets in movable areas
  (selectable text off, or labels eat the drag), 1 px outline, accent outline when
  selected/dragged, accent guide lines, the settings popover (`Widget::settings` + Reset)
  and the panel ("Edit HUD": widget icon + name + on/off toggle, Reset all = the known
  widgets only, Done). Disabled widgets are not drawn; the panel toggles them.
- `Widget` gained `name`, `icon`, `settings` (default none); `HudConfig::set`/`reset`,
  `hud::area_id`.
- Opened by `hud_editor` (F6, bound by `Settings::add_our_binds` only while free; kept
  out of `Settings::load`, which `default_binds_match_cpp` checks against C++) and "Edit
  HUD" in the Esc menu. While open: menu-mode input (`App::ui_active`), cursor free; Esc
  or Done closes and saves settings.cfg; a session end closes it too.
- Watermark settings: compact, items on/off, order with up/down carets (`icons` +
  `caret-up/down`). Deviation: SPEC's drag-to-reorder list is up/down buttons, and its
  "separator" entry is not there (in the friend's code it only groups the settings
  list; it has no effect on the HUD). Accepted by the user on 2026-11-25: keep up/down,
  no separator.
- `gen_icons.py` now reads codepoints from the TTF's ligature table (fontTools), no
  Phosphor CSS needed; all 40 old codepoints came out identical. Gallery reference
  regenerated (two more icons, nothing else changed).
- Verified: tests/hud.rs (9): snapping cases, anchor reassignment keeps the place on a
  1920x1080 resize, free drops round-trip on every anchor, a headless egui drag through
  the real editor (fails if labels stay selectable); 80 tests total, clippy, fmt.
  Live: `scripts/hud_editor_check.ps1` (F6, drag, click, Esc, restart): saved
  `hud_widget "watermark" right 0 96 1`, found there after the restart. Synthetic mouse
  input needs the real cursor on our (topmost) window, or winit reports the pointer gone.

#### 7.4 review fixes (2026-11-25, commit d6ab63d2d): done, awaiting the user's look
1. Edit HUD panel: `hud_editor::panel_rect` puts it bottom centre, else the first free
   spot of top centre, centre, left, right and the four corners that covers no widget
   (with its outline) and not the open popover; kept still while a widget is dragged.
   Caption shortened to two lines, so the panel is narrower (444 -> about 270 pt).
2. Settings popover: `hud_editor::popover_align` picks the first of egui's menu sides
   that fits on screen and stays off the panel; if none, the first that fits, and the
   panel moves away (it avoids the popover). `widgets::popover` takes an optional
   `RectAlign` (None = egui's choice, the gallery) and returns its rect.
3. Outlines: hovered 1 px accent, selected/dragged 2 px accent; idle widgets keep a
   1 px neutral outline (it says "movable"; decided with the skill).
4. Name: the watermark shows the requested name (`player_info(..).name`, so
   `player_name` or C++'s "nameless tee") until the server's ClientInfo arrives
   (`HudData::player_name`). Checked once with the user's real settings (read-only,
   client killed, our settings.cfg md5 unchanged): showed "TeDeafe".
- Tests: `editor_panel_and_popover_keep_out_of_the_way`. Live: hud_editor_check.ps1
  (now also captures ed_open.png, logs per client); 800x600: panel bottom centre, after
  the drag and click the panel moved top right and the popover sits left of the widget,
  nothing overlaps. The live drag is flaky (about 1 in 3 runs the posted mouse
  messages do not start a drag; the same with the previous commit); rerun it.

#### 7.5 status (2026-11-25, commits 29751f3cf, b9b5164d2): done, awaiting review
- time item (HH:MM, 24 h, `chrono::Local`, `watermark::clock`): chrono with only
  `clock` (2 new crates; windows-link was already in the tree; jiff bundles a tz
  database on Windows). speed item (SPEC "bps"): the movement information's
  horizontal speed (7.6), tiles/s. Default order as SPEC: user, server, ping, fps,
  speed, time, uid, tps.
- `watermark_layout_measurements` (tests/hud.rs): measured from egui's output shapes:
  offset 16, outer and logo pill paddings 16 x 8, icon-value gap 4, 12 between items,
  16 per dot slot, sizes caption 12 (logo), body 15 (values, units), icons 14/17.
  These are our tokens, not SPEC's 10/9 px (the references are video crops at another
  scale). Mutating a padding or the icon gap fails it.
- HUD on/off input test: `scripts/hud_input_check.ps1` (+ `examples/teehistorian_
  inputs.rs`, reads the server's teehistorian). Two local-server runs, every widget and
  item on vs. HUD off, same posted keys (right, jump, left, hook): the input sequences
  the server received are identical (10 changes). `-Mutate` (one key fewer) fails it.
- Side-by-side captures with the two references (scaled so the logo pills match):
  made from `examples/glass` renders (dm1 900 500 light, Tsunami 1000 1000 dark),
  composed by a scratch PIL script; sent to the user. Visible differences for review:
  the reference logo is a rounded rectangle with a soft white glow (ours a plain pill,
  no glow: the UI rules forbid glows unless asked, PLAN 7.5 mentions one), its icons
  are filled (ours Phosphor regular), its stat gaps are wider, values look semibold.

#### 7.6 status (2026-11-25, commit 531ef1cf6): done, awaiting review
- `movement.rs`: `info` = `CHud::GetMovementInformation` for a snapshot character,
  `shown` = the 2 dp text, `of(&HudData)`; widget `movement` bottom right (-16, -16),
  off by default (C++ `cl_showhud_player_*` default 0), items position (map-pin,
  "x, y"), speed (gauge, "x, y" tiles/s), angle (compass, degrees), each toggleable.
  `render::players::target_angle` is now a free function (`&Rendered`).
- Parity: `rs_parity movement` (src/tools/rs_parity_hud.cpp) runs the real CHud and
  CPlayers in the zeroed CGameClient block (like camera); hud.cpp, players.cpp,
  skin.cpp and client_data7 linked; 17 render functions they reference are stubbed
  with `dbg_assert(false)` (never reached), `PredictDummy` copied. 20,000 cases (local
  aim from the mouse incl. the centre fix, other players with/without DDNet targets,
  velocities at the 1 and 128 thresholds, random velocity ramps): every float and
  every text identical (`movement_information_matches_cpp`).
- Not ported (deviations, for review): the spectator paths (freeview camera centre,
  `m_SpecChar`: our spectator camera is not ported, the widget hides without a tee);
  the speed colours (green/red on increase/decrease; one-accent rule); the server's
  tuning (not received yet, so default velramp, as for prediction).

#### User review of 7.5/7.6 (2026-11-25), applied in commit 889b0bd7a
- Icons: Phosphor Fill 2.1.2 (`assets/fonts/Phosphor-Fill.ttf`, same npm package and MIT
  licence as the regular font it replaces) everywhere; `gen_icons.py` lists one meaning
  per icon in a comment (ping wifi, fps monitor, speed gauge, position map pin, aim
  crosshair, time clock, connect plugs, back to game controller, ...). The logo is a
  drawn tee mark (`widgets::tee_mark`), not a glyph.
- Watermark like the reference: pill container with `Glass::hud_fill` (black 45% subtle
  / 35% full, opaque SURFACE when glass is off), compact logo (rounded rect `radius::SMALL`,
  padding 12 x 8) with a soft white glow (`style::LOGO_GLOW`), barely visible dots
  (`color::SEPARATOR`), gaps measured from the reference at the scale where its digits
  match body 15: logo-stat 12+32 (ref 33), stat-stat 48 (ref 48), icon-value 8 (ink 11 vs
  ref 13). Name item off by default (it shows above the tee); defaults ping and fps.
- Shape rule (CLAUDE.md): one line = full pill, several = rounded rectangle (`CARD`);
  `widgets::hud_bar(ui, glass, multi_line, margin, row_height, ..)`; reference-image
  effects count as asked for (CLAUDE.md).
- Movement widget: "X 57.53  Y 15.53", "X 0.00  Y 0.91t/s", "aim 0.00°", labels and units
  grey; `hud_set movement stacked 1` = values in two rows (rounded rect), single-line
  items centred. The watermark speed unit is "t/s" too.
- Editor: a popover taller than the room opens beside its widget
  (`tall_popover_opens_beside_the_widget`).
- egui gotcha: `ui.horizontal` centres items on a row `interact_size.y` tall (our style
  sets 32), and in a right-aligned column it runs right to left (the killfeed adds its
  parts reversed).

#### 7.7 status (2026-11-25, commit 8d0aa10cd): done, awaiting review
- `statusbar.rs`: `status()` = `CHud::RenderPlayerState` decisions line by line (jumps
  with every branch/quirk, weapons + active, ninja timer, capabilities, disabled
  interactions with the C++ gun quirk in DECISIONS.md, lock/practice/team0 modes,
  deep/live freeze); widget `status` top left under the watermark (16, 88): jump pips,
  the game skin's pickup weapon sprites (`WeaponIcons`, from game.png in the data dir,
  40% when not held), ninja timer bar, Phosphor fill state icons.
- Shown when C++ would: DDNetCharacter present and the server's `m_HudDDRace`
  (`Predictor::hud_ddrace`). Core: the predicted local core (`Predictor::predicted_core`),
  else one read from the DDNetCharacter (C++ also overwrites other tees' cores from the
  predicted world; flags are the same). Not shown: freeze progress (C++ draws it above
  the tee, not here).
- Verified: `status_bar_follows_render_player_state` (cases worked out from C++);
  live `scripts/statusbar_check.ps1`: DDNet-rs plays "subject", switches 4 states with
  rcon test commands (binds), the C++ client spectates it; indicator sets equal in all
  four (docs/review/7.7). For that the client got `rcon_auth`/`rcon` (C++ wire format;
  `ponytail:` no auth-status tracking yet).

#### 7.8 status (2026-11-25, commit 036591a4a): done, awaiting review
- `killfeed.rs`: `Feed` = `CInfoMessages` rules (ring of 5, oldest first, 500-tick
  lifetime, empty kills skipped, active-client names, finishes need an active client,
  C++ time/diff texts); capsules top right (-16, 16), slide in and fade out
  (`motion::NORMAL`); sample capsule in the editor (`HudData::editing`).
- `timefmt.rs`: `str_time`/`str_time_float`; `rs_golden` str_time.txt (11,142 vectors)
  all equal; 7.9 uses it for the race timer and finish texts.
- Not ported: team kill members (need `Sv_TeamsState`), DDTeam name colours, CTF flags.
- Verified: `killfeed_follows_info_messages`; live `statusbar_check.ps1 -Kills`: the same
  entries in the same order in both clients (docs/review/7.8). A live finish was not
  run: every teleport test command marks the run cheated, so a scripted finish needs a
  real route (unit tests cover the finish texts).

#### Review round 3 (2026-11-25, commits 2e2176888 and the next): awaiting the user's pick
- 2e2176888: killfeed tee heads (`teehead.rs`, real skins, custom colours), compact
  status block, watermark sliders (`hud_set watermark corner|logo_radius|logo_height|glow`,
  defaults 35 % / 8 / 41 / 100 until the user sends final values), DDNet visibility (off
  items not drawn, all off = hidden, editor placeholder), C++ HUD switches imported.
- Status icons (`statusview.rs`): one grammar, a filled base (Phosphor fill glyph or the
  weapon's silhouette cut from game.png, cropped to its opaque part) plus at most one
  mark: slash (off), "∞" badge (endless, Inter's sign), sparkle badge (teleport),
  snowflake badge (live freeze on a standing figure), ring (deep freeze). Every icon is
  labelled on the second gallery page and named on hover in the HUD editor (tooltip:
  not checked live yet).
- Three status bar directions (`statusview::Layout`, `hud_set status layout 0|1|2`,
  segmented control in the widget's editor popover; default Block): Block (card of
  rows: weapons, segmented jump meter, effects), Rail (narrow vertical card: held
  weapon, jump number, stacked effects), Chips (one pill for weapon + "2/2", one round
  chip per effect, ninja timer as a ring). Review page: `cargo run -p ddnet-rs --example
  gallery -- tests/golden/gallery.png docs/review/7.7/directions.png`.
- Watermark: HUD fill darker (black 55 % subtle / 45 % full, all HUD bars) and a soft
  dark glow around the watermark container (`style::HUD_SHADOW`, `w::hud_bar_with`);
  the white logo glow is kept.
- Verified: gates (88 tests), the live C++ comparison of the status states (Block,
  docs/review/7.7/cpp-vs-rust-states.png).

#### Review round 4 (2026-11-25): Chips chosen, states merged; awaiting review
- The user picked Chips; Block, Rail and the layout setting are gone. Rule (also for
  later widgets): a state that changes something already shown changes that element.
  `statusview.rs`: one weapon pill (owned weapons as bold silhouettes, held full, others
  40 %, then the jump counter); weapon marks instead of icons: strike-through (hits off),
  sparkle (teleport), flame on the gun (jetpack); "∞" in the counter (endless jump); the
  ninja ring around the sword; freeze tints the pill (`color::FREEZE`): frozen
  (`core.freeze_end > tick`), deep (stronger tint + border), live (light tint, figure).
  Only endless hook (a drawn hook + "∞"), hooking others off (hook struck through),
  solo, collision off and the modes keep small chips, plain glyphs without backgrounds.
  Everything is named on hover in the HUD editor (not checked live).
- With deep and live freeze both on, the pill shows deep only (C++ draws both icons).
- Gallery page: every state labelled (docs/review/7.7/states.png); live C++ comparison
  rerun (docs/review/7.7/cpp-vs-rust-states.png). Unused icons pruned.

#### Review round 5 (2026-11-25): reverted, the 18dc668d2 chips are final
- A redraw with flat vector weapons and a 25 % larger pill (1decdcbed) was reverted at
  the user's request: their friend liked the 18dc668d2 chips (sprite silhouettes, 24 pt
  icons), which are the one and only style (no style setting). The files equal
  18dc668d2 again.
- The user plays with the HUD for a while before more polish.

- Watermark container corner default 35 % -> 40 % of its height (user review: a bit
  rounder, closer to the old shape than to the pill); the logo keeps radius 8.

#### Current state and HUD decisions (2026-11-25, HEAD 10ad433d3): 7.1-7.8 done
Widgets: watermark (top left), race status chips (under it), position/speed (bottom
right, off by default), killfeed (top right); the HUD editor (F6 / Esc menu) moves them,
toggles them and opens their settings. All are reviewed by the user; these decisions
are final unless the user changes them:
- Shape rule (CLAUDE.md): single-line widgets (watermark, notifications, capsules) are
  full pills, multi-line widgets and panels rounded rectangles (`radius::CARD`).
  Exception by review: the watermark container's corner is 40 % of its height
  (`watermark::Look`, default; the slider `hud_set watermark corner` remains).
- Watermark: dark translucent container (`Glass::hud_fill`, black 55 % subtle / 45 %
  full), a soft dark glow around it (`style::HUD_SHADOW`), the white logo rounded
  rectangle with radius 8 and the white glow (`style::LOGO_GLOW`), the drawn tee mark +
  "DDNet-rs"; stats as glowing icon, medium value, grey unit. Since 7.9: compact
  spacing (logo height 33, bar 49, no dots, see the 7.9 status), defaults local time,
  ping, fps (name off: it shows above the tee). Look sliders (corner, logo
  radius, logo height, glow) stay in its editor popover; the user may send final values.
- Race status: the Chips look of commit 18dc668d2 is final, the only style, no setting
  (a flat vector redraw was reverted). Rule: a state that changes something already
  shown changes that element: one pill with the owned weapons (game-skin silhouettes,
  held full, others dim) and the jump counter; hits off strikes the weapon through,
  teleport adds a sparkle, jetpack a flame on the gun, endless jump makes the counter
  "∞", ninja has its timer ring, freeze tints the pill (frozen / deep with a border /
  live). Only endless hook (drawn hook + "∞"), hooking others off (struck hook), solo,
  collision off and the lock/practice/team 0 modes keep small chips.
- Icon rules: Phosphor Fill only (no outline icons anywhere), one meaning per icon
  (listed with comments in `scripts/gen_icons.py`), simple bold silhouettes without
  icon backgrounds; reference-image effects (the logo glow) are allowed (CLAUDE.md).
- Visibility like DDNet: an item that is off is not drawn, a widget with every item off
  is hidden (placeholder in the editor); the C++ `cl_showhud_*`, `cl_showkillmessages`,
  `cl_show_finish_messages` settings are imported as first values.
- Killfeed: capsules with the real skins' tee heads (killer tee, weapon, victim tee),
  the skull only for a self-kill or death, C++ `CInfoMessages` rules.
- Review images: `docs/review/7.4` .. `7.8`. Live C++ comparisons:
  `scripts/statusbar_check.ps1 [-Kills]`; inputs with the HUD on/off:
  `scripts/hud_input_check.ps1`.

#### 7.9 status (2026-11-25): Dynamic Island + HUD numbers, awaiting review
- History: the island was merged with the watermark (e9b529b29) at the user's request,
  then reverted (5f16930dc) when they changed their mind: separate widgets.
- `island.rs`: widget `island` (top centre 0, 16). Idle: the race timer (TITLE
  semibold, red/blinking in a time limit's last minute, "Sudden Death") + checkpoint
  difference, after a status when there is one ("Game paused", "Free-View",
  "Following <name>"). Events expand it: finish (check, or trophy for a personal /
  server record from the local `Sv_RaceFinish`, 6 s) and broadcasts (10 s, the timer at
  their end). Single-line = pill, multi-line = `radius::CARD`. Liquid Glass and
  `Glass::liquid_fill` at full strength, `hud_fill` otherwise, `HUD_SHADOW`.
- `Anim`: width/height/corner on underdamped springs (`anim::Spring::update_damped`),
  content crossfade (old out, swap, new in; the timer and the same broadcast update in
  place), events never smaller than the idle size, grows out of a small capsule.
- Items `hud_set island timer|checkpoint|broadcasts|spectator 0|1`; first values from
  C++ `cl_showhud_timer`, `cl_showhud_time_cp_diff`, `cl_show_broadcasts`,
  `cl_showhud_spectator` (now known to our config).
- Watermark: local time on by default and first (then ping, fps); the island has no
  clock (user decision).
- No jitter (user request): Inter's digits are tabular (`scripts/gen_tnum.py` edits the
  three bundled TTFs); `widgets::number` keeps numbers right-aligned in a minimum-width
  field (watermark stats, movement values); the watermark's fps refreshes every 0.5 s.
  Island timer and killfeed times get the tabular digits. Gallery reference regenerated
  (digits and stat widths only).
- Verified: unit tests (timer cases from RenderGameTimer, race effects, broadcasts,
  spectating, priority, damped spring), `hud_numbers_do_not_jitter` (equal digit
  advances in every weight, a field keeps what follows in place, fps 60 -> still 60 at
  0.2 and 0.49 s -> 144 at 0.5 s), all gates. Live (`scripts/statusbar_check.ps1
  -Island`, local server, C++ client spectating): timer text equal in all four states
  (00:15/18/20/22), "Free-View", "Game paused", the rcon broadcast.
  Not live-checked: a real finish (needs a race route, like 7.8).
- Review GIF: `cargo run -p ddnet-rs --example island -- data/maps/dm1.map 900 500 2
  <out dir>` (PNG frames, composed with PIL), sent to the user.
- Watermark review (user, 2026-11-25): compact by default: 8 inside the container edges
  and the logo (logo 33, bar 49 tall instead of 65), 16 from the logo to the first stat
  and after the last, 24 between stats, no separator dots; logo text kept (compact
  mode = tee mark only). Each stat icon has a soft white glow (`style::ICON_GLOW`, blur
  12, weaker than `LOGO_GLOW`, scaled by the same glow setting), drawn by `w::stat`.
  Race status default moved up to (16, 72) to keep its 7 pt gap under the shorter
  watermark; a saved placement (e.g. 16, 88) is kept (Reset in the HUD editor moves it).
  `watermark_layout_measurements` checks the new gaps, the height, no dots and a glow
  behind each icon. Before/after/reference comparison sent to the user.
- Accepted by the user (2026-11-25): watermark and island looks.
- Island items like the watermark's (user request): `hud::item_order`/`item_on`/
  `item_list` (shared with the watermark: toggle + up/down per item); island items
  status, timer, checkpoint, media, finish, broadcasts, track (`hud_set island order`);
  the idle row follows the order, events' order is their priority. media/track act in
  7.10. Key `spectator` became `status` (import from `cl_showhud_spectator`). Fixed on
  the way: turning the spectator text off no longer shows race effects while spectating.
- Known: at 800x600 the default watermark (with the clock) reaches under the island;
  from about 1000 pt width they do not touch.

#### 7.10 status (2026-11-25): media player, built; live check by the user pending
- `media.rs`: `Media::start()` spawns a thread that polls Windows SMTC every 0.5 s
  (`windows` 0.62, features Foundation(+Collections), Media_Control, Graphics_Imaging,
  Storage_Streams; WinRT `.join()` blocks on async calls) and takes `Command`s (play/
  pause, next, previous, seek, source). Covers are decoded by Windows' `BitmapDecoder`
  (JPEG/PNG, no image crate), once per track. Timeline: position moved on since
  `LastUpdatedTime` while playing, clamped; none without a duration (the progress row
  hides). Non-Windows: no thread, everything media hides. `examples/media.rs` prints
  what the thread sees (no session was open on this machine, so not seen live yet).
- Island: item "media" = mini cover (24) + accent equalizer in the idle row (any place
  in the order); event "track" = cover (40), title, artist for 3 s on a new
  title/artist; F7 (`media_player`, bound only while free) expands the island into the
  player (SPEC §2): cover 56, title/artist (truncated), "⋯" source menu (Automatic +
  each session, by app name; not saved), elapsed / bar (click or drag to seek) /
  -remaining, previous / play-pause / next (disabled when the source says so); "Nothing
  playing" without a track. Menu mode while open (cursor free), Esc or F7 closes. The
  island's area takes clicks only while the player is open (`Widget::interactive`).
  Binds `media_play_pause`, `media_next`, `media_prev` (no default keys).
- Not done (deviations): the "⋯" menu has no "compact" (the idle mini player is the
  compact form) and no "hide" (the HUD editor's item toggle does it); the chosen source
  is not saved across restarts.
- Verified: `media` unit tests (time format, timeline maths incl. hostile values, local
  extrapolation), gates, a live run on the local server (client starts and plays with
  the media thread). Review GIF sent (stand-in track, rendered by examples/island.rs).

#### 7.10 review round (2026-11-25)
- User's live check: title, artist and cover correct (Yandex Music, Spotify) in the
  island and the F7 player.
- Fixed: the dark edge on bright maps (a 55 % shadow with spread showed through the 1 px
  ring at the glass backdrop's edge): `HUD_SHADOW` blur 40, no spread, 14 %, on every HUD
  bar; the "⋯" button is three drawn dots (32 pt click area); Inter Display Medium/
  SemiBold (Inter 4.1 release, same OFL, tabular digits) for the timer, finish time,
  track title, player times (`style::display`, `island::job` for sizes >= LABEL);
  the source choice is saved (`rs_media_source`, given to `Media::start`).
- Icons: `examples/icon_compare.rs <material.ttf> <codepoints> <out.png>` renders Phosphor
  Fill next to Material Symbols Rounded filled (a static FILL 1/wght 400/opsz 24 instance
  made with fontTools from Google's variable font; not bundled). Waiting for the pick.
- Lyrics (`lyrics.rs`): LRCLIB `/api/search` by title and artist through WinRT
  `HttpClient` + `JsonArray` (UA "DDNet-rs/0.1.0"), best synced result within 3 s of
  the duration, LRC parsed, cached in `<save dir>/lyrics/<fnv>.lrc` (empty file = none
  found), failures not cached. Fetched on its own thread on a track change while
  `hud_set island lyrics 1` (off by default; toggle in the island's settings). Shown in
  the F7 player (previous, current in Inter Display, next) with notes for loading / none
  / offline / no position. Live check: 61 lines for a known track, NotFound for a fake.
- Widgets (`nowplaying.rs`, off by default): "Now playing" card (player look without
  controls, `hud_set nowplaying lyrics` adds the lyrics line), "Lyrics line" (current
  line in a pill, top 0, 64). `island::player_ui(.., controls, lyric_lines, ..)` and
  `lyrics_ui` are shared; `island::CoverCache` per widget.
- Not done: per-app volume (item 6). Windows offers it only through Core Audio COM
  (`IAudioSessionManager2`, `ISimpleAudioVolume`), which needs `unsafe` (CLAUDE.md: no
  `unsafe`); `wasapi` 0.27 lists sessions but has no session volume, and
  `volumecontrol-windows` is device-wide. Waiting for the user's decision.
- Verified: unit tests (LRC parsing, current line, result matching, cache incl. none
  found), gates, live runs (local server: real cover in the island, soft shadow).
  Review images: `examples/media_review.rs`, `icon_compare.rs` (sent to the user).

#### 7.10 review round 2 (2026-11-25)
- Per-app volume: `audio.rs`, the one `unsafe` module (user-approved exception, noted
  in CLAUDE.md and DECISIONS.md): Core Audio sessions on the default output device,
  matched to the SMTC app id by the session identifier's executable path / package name
  (`audio::matches`, tested); safe `volume` / `set_volume`, SAFETY comment on every
  call. The media thread reads the shown app's volume each poll (`State::volume`),
  `Command::Volume` sets it. F7 player: volume row (down icon, thin white slider, up
  icon), one command per drag or click. Live: the playing app's session was found
  (volume read 100 %, not changed).
- Icons: Material Symbols Rounded filled everywhere (Apache 2.0, `MaterialSymbols-
  LICENSE.txt`), except Phosphor's ghost (no collision) and barbell (practice).
  `scripts/gen_icons.py <variable ttf> <codepoints> <Phosphor-Fill.ttf>` instances
  (FILL 1, wght 400, GRAD 0, opsz 24) and subsets both fonts (10.6 KB + 1.7 KB);
  `Phosphor-Fill.ttf` removed; unused DOTS_THREE, X, EYE_SLASH dropped; new
  VOLUME_DOWN/UP, LYRICS. Material glyphs scaled 1.15 (`style::MATERIAL_SCALE`) to the
  old optical size. `examples/icon_compare.rs` removed (choice made). Gallery reference
  regenerated.
- Lyrics in ONE place (`hud_set island lyrics_place island|card|off`, default island,
  segmented control in the island's settings with the lrclib.net note; "off" = nothing
  fetched): subtitles under the island (`island::Subtitles`: current line only, BODY
  medium at 80 %, faint shadow, no box, crossfade over `motion::NORMAL`; hidden while
  the F7 player is open); the card shows one line only when the place is "card"; the
  F7 player is compact, its "Lyrics" button (accent when open) shows previous/current/
  next inside it. The lyrics line widget is gone. Live: subtitles followed a real track.
- Verified: unit tests (session matching), gates, live run (local server: Material
  icons, subtitles, cache file written in the scratch save dir). Review image:
  `examples/media_review.rs` (compact player with volume, player with lyrics, island
  with subtitles, card).

#### 7.10 review round 3 (2026-11-25): player accepted by the user
- Player bottom area has one fixed height (`BOTTOM_HEIGHT`): the controls, or in the
  lyrics view the current line (Inter Display) with the next one dimmed under it; the
  card never changes size. The old full-width volume row is gone.
- Previous / play-pause / next are Phosphor Fill again (`gen_icons.py`), placed at fixed
  centres symmetric around the card's centre and centred on their glyph ink
  (`mesh_bounds`); measured: -56.5 / +55.5 px around 640.
- Volume: speaker at the area's left (icon by level: off / down / up; wheel = 5 %
  steps); a click slides a narrow vertical slider (thin bar, small flat handle) out of
  the card's LEFT edge, level with the speaker, never over the card; on the right when
  there is no room (`pane_on_left`, tested); a click elsewhere closes it.
- Subtitles: the box shadow (diagonal streaks at the ends) became a soft shadow of the
  glyphs (two rings of faint dark copies); checked on dm1 (bright) and Tsunami (dark).
- Verified: gates, review renders (`examples/media_review.rs`, which also clicks the
  speaker once so the slider shows).

#### 7.10 polish (2026-11-25)
- The volume slider uses the player card's glass (`island::card_glass`, shared with the
  island: same shadow, blur / Liquid Glass, tint), not an opaque box.

#### 7.11 status (2026-11-25): settings and finish
- Settings page: glass (Off/Subtle/Full) and UI scale were there since 7.1; new: a
  "Media player" toggle (`rs_media`, default 1: 0 drops the media thread, so no SMTC
  polling, lyrics or volume; the HUD gets `media: None` and F7 does nothing) and bind
  rows for "HUD editor" (`hud_editor`, F6) and "Open media player" (`media_player`, F7).
  The HUD editor itself stays in the Esc menu ("Edit HUD").
- Default HUD layout: unchanged (watermark, status bar, killfeed, island on; movement
  and Now playing off; C++ HUD switches imported once).
- Frame time (`scripts/frame_time.ps1`; `--screenshot-after` now prints the mean frame
  time since the map showed): 2560x1440, vsync off, release, local server, dm1, RTX 4060
  Ti, 2 runs x 15 s each: HUD off 0.59 / 0.70 ms, everything on (every widget and
  watermark item, full glass, media on) 0.90 / 0.77 ms: about +0.2 ms (target < 1 ms).
  A live capture showed every enabled widget drawn.
- M6 screenshot comparisons with the HUD off: `compare_screenshots.ps1` now gives
  DDNet-rs a scratch APPDATA (all widgets off). Before, it read the real
  `%APPDATA%\DDNet-rs\settings.cfg`, whose name overrode "subject" (the C++ spectate
  failed) and whose HUD showed in the capture. Results: dm1 `-RustSubject` 99.39 %
  (M6: 99.34), ctf1 `-RustSubject` 99.74 % (99.74), dm1 follow mode with
  `-ServerArgs 'sv_show_all_default 1'` 99.42 % (99.06). Follow mode without it gets
  no snapshot of the far tee (0.08 %), as the script's header says.
- Verified: gates (103 tests).

#### HUD fixes after M7 (2026-11-25, user requests)
- Glass without a rim (62a1fbd88): no specular rim in the Liquid Glass shader; the blur
  covers the fill's exact shape (was shrunk 1 px, leaving a ring of unblurred game that
  read as a dark edge). Applies to every glass shape (island, player card, volume
  slider, HUD bars, panels).
- Height scaling (9aa8429cf, 260cedae0): pixels per point = window height / 1080 x
  `rs_ui_scale` (`style::ui_scale`), ignoring the OS DPI, no floor (user: the HUD
  scales like a picture; 800x600 = 1080p smaller). Points are 1080p reference pixels,
  so saved offsets keep working. Applies to menus too (one egui context). No
  auto-moving and no watermark wrapping (user decisions; both were tried and dropped).
- Race status stacks 8 pt under the watermark's actual bottom while both sit at their
  default places (`Widget::below`, `hud::placements`, used by the editor too); default
  y 73.
- HUD shadow without corner streaks (4c5abe48b): egui's blurred shadow mesh drew thin
  dark diagonals off 32 pt capsules' corners; `widgets::soft_shadow` stacks ten
  rounded rects instead.
- Verified: gates; live 800x600 / 1920x1080 / 640x480 window captures (local server,
  scratch APPDATA), kills via `statusbar_check.ps1 -Kills`.

### M8 — antiping (plan: PLAN.md §12, approved 2026-11-25, answers §12.7; done 8.1-8.8)

#### 8.1 (2026-11-25): messages the prediction world needs — done
- `Sv_TuneParams` (C++ reads ints until the first unpack error, jetpack strength 0 by
  default), `UpdateLocalTuning` (tune zones, expected-tuning state machine) and
  `LoadMapSettings` (every zone reset to the DDRace map defaults, then the map's
  `tune`/`tune_zone` settings through the console parser, now in `ddnet/src/console.rs`,
  moved from `client/src/config.rs`). `Sv_TeamsState` (+ legacy) into the predictor's
  `TeamsCore`, `Sv_KillMsg`/`Sv_KillMsgTeam` (GiveWeak in C++ stable-sort order,
  `ResetPrediction`, `ReleaseHooked`), the `Sv_PreInput` ring (applied in 8.5).
  `net/client.rs` queues them in arrival order (`Client::predict_msgs`), the predictor
  drains them before snapshot switches, as C++ handles messages before `Update`.
- Also: `m_IsSolo` and the strong/weak gate use the received tuning (collision/hooking),
  the gate needs the local character in the world like C++ (it did not), `m_IsFNG` from
  game info, item rendering uses the predictor's tuning per zone (`GetTuning`),
  `Client::say` (chat send only, for the live test).
- Verified: `rs_parity msgs` (new, `src/tools/rs_parity_msgs.cpp`): word-for-word copies
  of the CGameClient handlers, `UpdateLocalTuning`, `LoadMapSettings` and the tune
  console commands, driving the real CUnpacker, CTeamsCore, CCharOrder, CGameWorld/
  CCharacter and CConsole; 400 fuzzed settings blobs, 30 runs x 300 random events on
  coverage. `tests/parity.rs messages_match_cpp`: all 9,611 lines identical; 11 of 11
  hand-made mutations caught. `cpp_copies_match_source`: the 8 copies are identical
  (whitespace aside) to the C++ lines they name. Live: `prediction_matches_server_with_
  tuning_and_teams` (global tuning and 40 tune zones as a map config, which the server
  imports into the map's settings; team 7 joined by `/team 7` on coverage): 0 of 250
  mismatched, 0 prediction errors on dm1 and coverage, 3 runs; ignoring `Sv_TuneParams`
  gives 229 mismatches. The scripted path never enters a tune zone, so zone switching
  is covered by `rs_parity msgs` only.
- Found: with stronger tuning the dm1 path hits a death tile; the server (practice)
  freezes there, which C++ client prediction does not model either (DECISIONS.md).
- Regenerate: `build-rs/rs_parity.exe msgs data/maps/coverage.map rust-client/tests/golden/msgs.txt`.

#### 8.2 (2026-11-25): collision for weapons — done
- `Collision::intersect_line`, `intersect_line_tele_weapon` (without tele number, as the
  laser calls it), `move_point` (bounce count), `mover_speed`. `SetCollisionAt` is not
  needed: the laser's branch that calls it is unreachable in C++ (DECISIONS.md).
- Verified: `rs_parity collision` queries all four on every box tick (both line casts to
  the random ray end, `MovePoint` with that ray and with the box velocity x4,
  `MoverSpeed` at the box) plus every map tile's `MoverSpeed`; the "hooktele" variant
  also turns every 7th empty tile into a mover (`MakeMovers`: no bundled map has one).
  100% identical on all 7 maps and both variants (125 and 15,878 movers); 6 of 6
  mutations caught.

#### 8.3 (2026-11-26): weapons and entities in the prediction world — done
- `physics/weapons.rs`: HandleWeapons, HandleNinja (dash with hits), FireWeapon (hammer
  incl. FNG, gun, vanilla shotgun spread, DDRace shotgun laser, grenade, laser, ninja,
  reload and attack tick, pain timer), FindEntities, IntersectCharacter,
  CreateExplosion, AntiPingInterference (the core's queued interferences use it too).
  They run on the whole world; `Character::on_direct_input` now returns whether
  FireWeapon follows. `projectile.rs` (fired and snapshot projectiles, bouncing and
  freeze turrets), `laser.rs` (DoBounce, HitCharacter incl. the stacked-shotgun bug),
  `pickup.rs` (pickups with movers, draggers, plasma); `collision.rs` gained
  `intersect_no_laser(_no_walls)`. World tick in C++ `ENTTYPE` order, then
  `RemoveEntities`. New world config: `predict_weapons`, `is_vanilla`, `is_ddrace`,
  `old_laser`; `NetCharAdd` sets the game team.
- Verified: `rs_parity weapons` (new): per map 30 runs x 150 ticks, 4 tees with every
  weapon firing at each other (random aim, fire, weapon switches, chat), 8 pickups of
  every type (some on switches, mover tiles from `MakeMovers`), 2 draggers of every
  subtype, 3 turret projectiles (bouncing, freeze, without extra info), plasma from
  random ticks (some without target); six configs (DDRace; no weak hook + old laser +
  team split + super; vanilla; FNG with game teams; solo + limited ammo + hit-disabled
  tee; changed global tuning without zones), ninja, jetpack, deep freeze, zone weapon
  tuning. Every character (full state + reload, attack tick, queued weapon, pain timer,
  ninja hits, move restrictions) and every entity compared each tick: 100% identical on
  all 7 maps (31,500 ticks). 32 of 33 hand-made mutations caught; the other is
  equivalent here (vanilla laser delay of 7.5 ticks: `>` and `>=` never differ).
- Regenerate: `build-rs/rs_parity.exe weapons "data/maps/<name>.map"
  rust-client/tests/golden/maps/<name>.weapons.txt` (add `verbose` for entity values).

#### 8.4 (2026-11-26): snapshot entities into the prediction world — done
- `physics/snap.rs`: `GameWorld::net_obj_add` for projectiles (all three objects: Keep,
  DDRace shotgun lifespan, matching a locally fired one, guessing the owner from
  `PrevPos`/`PrevPrevPos`), pickups (appended, no-predict flag), lasers (rifle/shotgun
  matching incl. a local one, "stopped earlier"), draggers, doors, plasma; `find_match`
  (`EntityRef`); the entities' `Match`/`Read`; `Laser::from_data`. `net_obj_begin`/`end`
  cover every list; `copy_world` (C++ CopyWorld: no doors) replaces the clone in
  `predict`. `predict.rs` feeds every snapshot entity (`snap_entities` order).
- Verified: `rs_parity netents` (new): per map 4 configs x 60 snapshots; tees fire for
  real, and each snapshot sends server versions of their projectiles (vanilla objects,
  offsets 0/2/3/40 px) and lasers (DDNet objects, cut 0/1.5/3 px or in half), new
  server entities of every object type (owners local/other/other team/none/invalid,
  DDRace flags, DDNet flags incl. normalize, EntityEx classes) that stay, move, change
  or skip a snapshot, and a tee that leaves and comes back. After each NetObjEnd: every
  character and entity, `FindMatch` for every item, and a CopyWorld ticked once. 100%
  identical on all 7 maps (1,680 snapshots, ~33,000 items, about half matched); 17 of
  17 mutations caught.
- Regenerate: `build-rs/rs_parity.exe netents "data/maps/<name>.map"
  rust-client/tests/golden/maps/<name>.netents.txt`.
- Antiping stays off by default (`predict_weapons` false until 8.5 wires the settings),
  so the live behaviour is unchanged; the M5/M6 live tests still pass.

#### 8.5 (2026-11-26): prediction loop and the antiping settings — done
- Settings: the 12 `cl_predict*`/`cl_antiping*`/`cl_prediction_margin` variables in
  `client/src/config.rs` (C++ names, defaults, ranges), read from `settings_ddnet.cfg`
  like the others and handed to the predictor every frame (`PredictConfig`, C++ reads
  `g_Config` live); `Client::prediction_margin` (used with `SyncWeaponInput`, applied to
  the predicted time every frame like C++).
- `net/client.rs`: `prediction_time`, `prediction_tick` (limit/percent), `smooth_tick`,
  `Timing` (what `OnPredict` reads from `CClient`). `predict.rs`: the C++ gates
  (`AntiPingPlayers/Grenade/Weapons/Gunfire`, `Predict`), per-client `prev_predicted`/
  `predicted` at the prediction tick, the 200-tick `pred_pos` ring, `predicted_world`
  and `prev_predicted_world`, `ApplyPreInputs` in both worlds, `cl_predict_freeze 2`,
  other-team projectiles removed, world config from game info (`is_vanilla`,
  `is_ddrace`, `old_laser`, `predict_weapons`, `predict_freeze`), `last_pos`/
  `last_active`. `on_new_snapshot`/`predict` take slices and a `Timing` instead of the
  `Client`, so the parity test drives them directly.
- Fixed on the way: `IsWorldPaused` includes game over; every snapshot `DDNetCharacter`
  goes into `predicted[i]` and the solo state (only active ones did); see DECISIONS.md.
- Verified: `rs_parity antiping-timing` (3000 vectors of the copied `GetPredictionTime`/
  `GetPredictionTick`/`GetSmoothTick`) and `rs_parity antiping` (new
  `src/tools/rs_parity_antiping.cpp`: word-for-word copies of `UpdatePrediction`,
  `ApplyPreInputs`, `OnPredict` incl. smoothing, `GetSmoothPos`, `IsOtherTeam`, the
  gates and `GetGameInfo` in a stand-in `CGameClient`, driving the real `CGameWorld`):
  per map 12 runs x 60 snapshots from a server world with 4 firing tees, preinputs
  (some lost, some first wrong), all `cl_antiping*` combinations, limit/percent, freeze
  windows incl. deep freeze, team switches between snapshot and prediction, a 150-tick
  gap, pause/game over. After every UpdatePrediction and OnPredict: all predicted cores,
  the pred_pos ring, the smoothing state and the three worlds. 100% identical on all
  7 maps; `cpp_copies_match_source` now checks 15 copies; 14 of 14 mutations caught.
  Live: `prediction_matches_server_with_antiping` (dm1, coverage; `cl_antiping 1`,
  every weapon via the rcon test command `weapons`, scripted fire, grenade jumps on
  coverage): 0 of 250 mismatched, 0 prediction errors, every own projectile and laser
  of every snapshot in the prediction world (dm1 11/33/14/39, coverage 52/31/14/23 for
  gun/shotgun/grenade/laser).
- Regenerate: `build-rs/rs_parity.exe antiping-timing rust-client/tests/golden/antiping-timing.txt`
  and `build-rs/rs_parity.exe antiping "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.antiping.txt`
  (`RS_VERBOSE=1` prints each server-world character state to stderr).

#### 8.6 (2026-11-26): smoothing — done
- `predict.rs`: the `cl_antiping_smooth` block of `OnPredict` (`smooth`, run after the
  prediction with `GetSmoothPos` of every client from before it) and `GetSmoothPos`
  (`smooth_pos`, the per-axis mix into the `pred_pos` history); `std::min` with its NaN
  behaviour.
- Verified: `rs_parity antiping` with smoothing on in half the runs and a third of the
  frames under 2 ms apart; every frame also compares `GetSmoothPos` of all 128 clients.
  100% identical on all 7 maps (windows started in every map). 9 of 13 mutations
  caught; the other 4 (the 700/300 ms caps, the window-extension bound, the running-
  window minimum) cannot change anything: C++ sizes windows in µs against a ns clock,
  so they end within ~3 ms (DECISIONS.md, suspected C++ bug, kept).
- Live: `antiping_smoothing_steps` (dm1, a second tee changing direction/jumping at
  random every 50-300 ms): largest per-frame step of its drawn position 12.61 px with
  smoothing on and off alike, 12 windows in 10 s; the same consequence of that C++ bug.

#### 8.7 (2026-11-26): rendering — done, captures waiting for the user's look
- `Predictor::rendered(&Timing)` is the whole `UpdateRenderedCharacters`: other tees
  predicted per `cl_antiping_players` (1 all, 2/3 interfering) unless in another team,
  with their snapshot aim and `GetSmoothPos` when smoothing; the local tee's attack tick
  and weapon from prediction with gunfire (`Rendered::predicted_local`). Players: the
  predicted attack time (players.cpp:615). Items: with `Predict() && AntiPingGunfire()`
  projectiles, the local tee's lasers and moving pickups from `prev_predicted_world`,
  matching snapshot items skipped (`FindMatch` in the server-state world), predicted
  projectile flight time and laser timing (`Predictor::projectile_time`,
  `laser_ticks`); `Projectile::get_data`, `Laser::get_data`.
- Verified: `rs_parity antiping` gained word-for-word copies of
  `UpdateRenderedCharacters` and of the timing lines of `RenderProjectile`/`RenderLaser`
  (18 copies checked now); every frame compares every rendered character (render
  cur/prev objects, predicted flags, position) and the timings of every projectile and
  laser of both worlds: 100% on all 7 maps, 8 of 10 mutations caught (the other two
  cannot change anything: other-team tees are never in the predicted world, and without
  smoothing `GetSmoothPos` is the plain predicted mix).
- Live captures (dm1, DDNet-rs playing with `cl_antiping 1` and the new dev flag
  `--fire WEAPON`, the C++ client spectating it; `compare_screenshots.ps1` gained
  `-RustArgs`/`-RustSettings` and fractional `-Wait`): `docs/review/8.7/` (gun,
  grenade, laser; `*_cpp.png` is the C++ spectator view, which shows the server state:
  C++ disables antiping while spectating). The C++ client cannot fire by itself, so a
  C++ antiping capture would need synthetic mouse input; not done.

#### 8.8 (2026-11-26): settings and finish — done, waiting for the user's review
- Settings page (`ui.rs`, `antiping_rows`): "Antiping" switch; when on, indented
  "Other players" (Off / All / Interact), "Always predict frozen players" (Interact
  only), "Weapons", "Grenade paths" (C++ menus_settings_ddnet.cpp:160 order). Below,
  "Prediction details" (Show/Hide) with smoothing, predicted gunfire, early inputs
  (preinputs), prediction limit, amount (only while the limit is 0), margin and
  freeze; explanations on hover. Existing widgets and tokens only (toggle, segmented,
  pill buttons, `space::L` indent, one accent). Review render:
  `docs/review/8.8/settings_antiping.png` (gallery example, third path).
- Frame time (`scripts/frame_time.ps1 -Antiping`, 2560x1440, dm1, DDNet-rs and 7
  ddnet-headless bots all firing guns, 3 runs of 15 s each): antiping off 0.738 /
  0.741 / 0.757 ms, on 0.762 / 0.766 / 0.760 ms (+0.02 ms; target was within 1 ms).
  The script now raises the server's per-IP client and connection limits (4 and 5 by
  default refused the 6th and later clients).
- Verified: `antiping_gates` (every switch combination, spectating or not); the
  settings variables round-trip through `settings.cfg`; full `cargo test` incl. the
  M5/M6 live tests on the default `cl_antiping 0` path; UI checked against the
  CLAUDE.md UI rules (pills for single-line controls, tokens only, one accent,
  secondary text on hover).

#### Exact next actions (next session starts here)
State (2026-11-26): M8 (8.1-8.8) done and pushed; stopped for the user's review of
8.5-8.8. Review material: `docs/review/8.7/` (gun/grenade/laser, DDNet-rs predicted vs
the C++ spectator view) and `docs/review/8.8/settings_antiping.png`. Open points for
the user: the C++ smoothing unit quirk (DECISIONS 8.6: smoothing has no visible effect,
in C++ too), and whether a C++-player antiping capture (synthetic mouse input) is
wanted.
1. Read CLAUDE.md, PLAN §12 (incl. §12.7), this file, `git log`.
2. If the review asks for changes, do those first; then PLAN §11 (roadmap after M8).

#### Decisions and gotchas from 7.1
- egui 0.37 panics in debug if a frame's `TexturesDelta` is dropped unapplied: drain
  `set`/`free` (done in `Painter::paint`).
- egui Areas fade in over time: headless renders must advance `RawInput::time` between
  passes (the gallery uses 3 passes, time 0/10/20 s) or everything stays translucent.
- `egui::Frame` content inherits the parent layout: `widgets::panel` wraps it in
  `ui.vertical`, `widgets::pill` in `ui.horizontal` (`horizontal_centered` stretched
  pills to the full height).
- `egui::Id::new` is deprecated in 0.37: use `Id::unique` for areas.
- `egui-phosphor` 0.14 does not support egui 0.37 (only an unofficial fork does): the
  Phosphor TTF is loaded directly.
- Writing Rust/PowerShell/C++ through bash heredocs or Python string literals kept mangling
  `\n`, `\t`, `\r` and backslash paths (e.g. `C:\ddnetrust-master\rust-client` became a
  carriage return): use the Write/Edit tools for such text, or check for control
  characters afterwards.
- F6 (HUD editor) and F7 (expand player) are free in stock DDNet; they get bound only if
  still free, like `SetDDRaceBinds(true)`. Keyboard media keys are never handled in game.

## Later
- Weapon prediction (`m_PredictWeapons`/AntiPingWeapons: FireWeapon, ninja dash,
  projectiles, lasers, pickups) is out of scope for now.
- Not ported for prediction yet: `Sv_TeamsState` (teams), `Sv_TuneParams` (non-default
  tuning), the dummy, antiping smoothing/pre-inputs, vanilla servers without
  `DDNetCharacter`/GameInfoEx.

## Not yet sent by the Rust client (a default C++ client sends these)
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
`doors` takes no flags). M6.2-M6.6 data (repo root):

    python rust-client/scripts/gen_png_corpus.py
    build-rs/rs_parity.exe png rust-client/tests/golden/png.txt rust-client/tests/golden/png data
    build-rs/rs_parity.exe mapimages "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.mapimages.txt
    build-rs/rs_parity.exe envelopes "data/maps/<name>.map" rust-client/tests/golden/maps/<name>.envelopes.txt
    build-rs/rs_parity.exe envelopes-synthetic rust-client/tests/golden/envelopes.txt
    build-rs/rs_parity.exe screen rust-client/tests/golden/screen.txt
    build-rs/rs_parity.exe anims rust-client/tests/golden/anims.txt
    build-rs/rs_parity.exe colors rust-client/tests/golden/colors.txt
    build-rs/rs_parity.exe skins rust-client/tests/golden/skins.txt data/skins
    build-rs/rs_parity.exe interp rust-client/tests/golden/interp.txt
    build-rs/rs_parity.exe items data/maps/coverage.map rust-client/tests/golden/items.txt
    build-rs/rs_parity.exe camera rust-client/tests/golden/camera.txt
    build-rs/rs_parity.exe movement rust-client/tests/golden/movement.txt
    rust-client\scripts\cpp_settings.ps1   # needs build-rs-client\DDNet.exe (PowerShell)

All modes are deterministic: the data regenerates byte for byte.

## Regenerate protocol code and content data
    python rust-client/scripts/gen_protocol.py   # needs rustfmt on PATH
    python rust-client/scripts/gen_content.py
    python rust-client/scripts/gen_keys.py

## Known issues / blockers
- Resend buffer is a byte budget, not C++'s ring buffer (differs only with ~32 KiB
  unacked data; `ponytail:` comment in `net/conn.rs`).

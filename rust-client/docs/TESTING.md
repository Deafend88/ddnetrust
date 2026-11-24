# Testing on Windows

Copy-paste commands for Windows PowerShell, checked on this machine (VS 18 Community,
repo at `C:\ddnetrust-master`). Everything stays on localhost: the server binds to
`127.0.0.1` with `sv_register 0`, so it never appears in the public server list, and
`ddnet-headless` refuses non-loopback addresses.

## 1. Run all Rust tests

Needs Rust (`cargo`) on `PATH`. For the server test, also build `DDNet-Server.exe` first
(step 2); without it, that one test prints `SKIPPED` and passes.

```powershell
Set-Location C:\ddnetrust-master\rust-client
cargo test
```

Expected: every `test result:` line says `ok`, with no `FAILED` (about 25 tests: unit,
`golden` 12, `parity` 7, `server` 1). The parity tests read maps from
`C:\ddnetrust-master\data\maps` and C++ data from `rust-client\tests\golden`. The full
pre-commit check is:

```powershell
cargo build; cargo clippy --all-targets -- -D warnings; cargo fmt --check; cargo test
```

## 2. Build and start the local DDNet-Server

Open a **Developer PowerShell for VS 18**, or turn a normal PowerShell into one. It may
print "vswhere.exe is not recognized" first; that is harmless.

```powershell
& "C:\Program Files\Microsoft Visual Studio\18\Community\Common7\Tools\Launch-VsDevShell.ps1" -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
```

Build (the first configure takes a minute; later builds are incremental):

```powershell
Set-Location C:\ddnetrust-master
cmake -G Ninja -S . -B build-rs -DCMAKE_BUILD_TYPE=Release -DCLIENT=OFF -DDOWNLOAD_GTEST=OFF
cmake --build build-rs --target game-server
```

Start it from a scratch directory whose `storage.cfg` lists only that directory and the
build's `data` directory, so it never reads or writes your real `%APPDATA%\DDNet`. The
server runs only after building; it does not need the developer shell.

```powershell
New-Item -ItemType Directory -Force "$env:TEMP\ddnet-rs-server" | Out-Null
Set-Content -Encoding ascii "$env:TEMP\ddnet-rs-server\storage.cfg" "add_path `$CURRENTDIR`r`nadd_path `$DATADIR"
Set-Location "$env:TEMP\ddnet-rs-server"
C:\ddnetrust-master\build-rs\DDNet-Server.exe "sv_register 0" "bindaddr 127.0.0.1" "sv_port 8303" "sv_map dm1"
```

Leave this window open; stop the server with Ctrl+C. Expected output (abridged):

```
I storage: added path '$CURRENTDIR' ('C:/Users/<you>/AppData/Local/Temp/ddnet-rs-server')
I storage: added path '$DATADIR' ('C:\ddnetrust-master\build-rs/data')
E console: failed to open 'myServerconfig.cfg'
E net: Failed to bind socket with domain 23 and type 2 (10049 ...)
I server: server name is 'My DDNet server'
I server: version 20.3 on win64 amd64
I server: | rcon password: '......' |
```

The two `E` lines are expected. The first is an optional config file. The second is the
IPv6 socket, which cannot bind to the IPv4 address `127.0.0.1`. Any map in
`build-rs\data\maps` works for `sv_map`: `dm1` is small and open, so it is good for
watching. `coverage` is what the automated test uses.

## 3. Run ddnet-headless against it

In a second PowerShell window:

```powershell
Set-Location C:\ddnetrust-master\rust-client
cargo run --release --bin ddnet-headless -- 127.0.0.1:8303 --name "rust tee" --script --seconds 300
```

Flags:
- `--script` loops walk-right/jump/hook, then the same to the left, every 4 s.
- `--walk N` walks right for N seconds instead.
- `--seconds N` disconnects after N seconds (default 10).

Expected client output (positions differ by map and spawn):

```
DDNet-rs 0.1.0 connecting to 127.0.0.1:8303
state: Loading
motd: Testserver with DDraceNetwork Features!\nDon't forget to check server rules by using /rules
chat: 'rust tee' entered and joined the game
...
state: Online
map: dm1 crc=f2159e6e size=5805
tick 120 pos (1584, 305) | prediction: 0 snapshots checked, 0 mispredicted, 0 prediction errors
tick 170 pos (1841, 289) | prediction: 25 snapshots checked, 0 mispredicted, 0 prediction errors
tick 220 pos (1841, 243) | prediction: 50 snapshots checked, 0 mispredicted, 0 prediction errors
...
disconnected cleanly
```

There is one `tick ... pos` line per second, and the position changes as the script
moves. The prediction counters compare every server snapshot of your tee with what the
client predicted for that tick. Alone on `dm1` they stay at 0 mispredicted and 0
prediction errors; other players bumping or hooking you cause some, which is normal.
Maps with doors (e.g. `coverage`) also stay at 0: door lasers from the snapshot are
predicted like the C++ client does.

At the same time the server window shows:

```
I server: player has entered the game. ClientId=0 addr=<{127.0.0.1:65023}> sixup=0
I chat: *** 'rust tee' entered and joined the game
I ddnet: cid=0 version=20030
...
I game: kill killer='0:rust tee' victim='0:rust tee' weapon=-3 special=0
I chat: *** 'rust tee' has left the game
I game: leave player='0:rust tee'
```

`version=20030` is the DDNet-rs client version. The `kill ... weapon=-3` line is the
server removing the character when the player leaves (not a crash).

## 4. Watch it with the normal DDNet client

While the server and `ddnet-headless --script --seconds 300` run, start your normal
DDNet client. Press **F1** to open the local console, type the line below and press
Enter, then F1 again to close the console:

```
connect 127.0.0.1:8303
```

The client downloads nothing for bundled maps like `dm1` (it already has them) and
spawns you next to "rust tee", which runs, jumps and hooks in a 4 second loop.
`Spectate` mode (the spectate button or `/spec`) lets you follow it without
interfering. The server window logs your client as `ClientId=1` with your client's
version number.

Notes:
- The server list's LAN tab may not find the server, because it is bound to
  `127.0.0.1`; use the console `connect` instead.
- If port 8303 is taken (another server), use `sv_port 8304` in step 2 and
  `127.0.0.1:8304` in steps 3 and 4.
- Start `ddnet-headless` after the C++ client joins if you want to see the "rust tee"
  enter; it can run any number of times against the same server.

## 5. The graphical client (M6, work in progress)

```powershell
Set-Location C:\ddnetrust-master\rust-client
cargo run --release --bin ddnet-rs
```

It connects to a local server like `ddnet-headless`:

```powershell
cargo run --release --bin ddnet-rs -- 127.0.0.1:8303
```

and draws the map around your tee (tees, entities and HUD come in later steps).
Options: `--data-dir PATH` (else `DDNET_DATA_DIR`, else `data\` in the current folder,
else this repo's `data\`, else next to the exe or Steam's default install) and `--vsync`
(C++ `gfx_vsync`, off by default). At start it prints its storage paths in search
order, e.g. `%APPDATA%\DDNet` (your DDNet folder, only read, never written), the data
dir and the current folder. More options: `--size WxH`, `--zoom Z`, `--view X,Y` (fixed
camera), `--follow ID` (center on another player), `--screenshot-after SECONDS`.
**F12** saves a screenshot to `%APPDATA%\DDNet-rs\screenshots\`.

## 6. Compare with the C++ client

Build the C++ client once (VS 18 developer shell, repo root; it needs the Vulkan SDK):

```powershell
cmake -G Ninja -S . -B build-rs-client -DCMAKE_BUILD_TYPE=Release -DCLIENT=ON -DSERVER=OFF -DTOOLS=OFF -DVULKAN=ON -DDOWNLOAD_GTEST=OFF -DVIDEORECORDER=OFF -DUPNP=OFF -DDISCORD=OFF -DSTEAM=OFF -DAUTOUPDATE=OFF
```

```powershell
cmake --build build-rs-client --target game-client
```

Then, with `cargo build -p ddnet-rs --examples` done:

```powershell
rust-client\scripts\compare_screenshots.ps1 -Map dm1
```

It starts a local server, the C++ client (scratch folder, never your DDNet folder, its
web requests blocked by a dead local proxy) and DDNet-rs following the C++ player,
captures both windows and prints the share of matching pixels; the pictures and a diff
image land in `%TEMP%\ddnet-rs-compare\`. Two windows open for a few seconds.

The GPU test (`cargo test -p ddnet-rs --test gfx`) checks a renderer test pattern pixel by
pixel; it reports itself skipped when no graphics adapter is available.

## Play on the local server with DDNet-rs

Start the local server as in step 2, then in a second PowerShell:

```powershell
Set-Location C:\ddnetrust-master\rust-client
cargo build --release -p ddnet-rs
.\target\release\ddnet-rs.exe 127.0.0.1:8303 --size 1280x720 --vsync
```

DDNet-rs reads `%APPDATA%\DDNet\settings_ddnet.cfg` once (never writes it): your name,
skin, colours, mouse sensitivity, dyncam/zoom settings and binds. Working binds: `+left`,
`+right`, `+jump`, `+hook`, `+fire`, `+weapon1`-`5`, `+nextweapon`, `+prevweapon`,
`zoom+`, `zoom-`, `zoom` (others are ignored). Esc frees the mouse, a click grabs it
again, F12 saves a screenshot to `%APPDATA%\DDNet-rs\screenshots`. Only loopback
servers are accepted.

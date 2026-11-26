# M7.11 frame time check: DDNet-rs at 2560x1440 (vsync off) on a local server, the HUD
# off vs. every widget and watermark item on with full glass. With -Antiping (8.8):
# cl_antiping 0 vs. 1 while DDNet-rs and 7 ddnet-headless bots all fire guns. Needs
# `cargo build --release -p ddnet-rs -p ddnet` and build-rs\DDNet-Server.exe. Uses
# scratch folders only (server and APPDATA), never the real DDNet or DDNet-rs folders.
param([string]$Map = "dm1", [int]$Seconds = 20, [int]$Runs = 3, [switch]$Antiping)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$tmp = Join-Path $env:TEMP "ddnet-rs-frametime"
$server = Join-Path $tmp "server"
New-Item -ItemType Directory -Force $server | Out-Null
Set-Content -Encoding ascii (Join-Path $server "storage.cfg") "add_path `$CURRENTDIR`r`nadd_path `$DATADIR"
$srv = Start-Process -PassThru -WindowStyle Hidden -WorkingDirectory $server `
    "$repo\build-rs\DDNet-Server.exe" `
    -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8319"', "`"sv_map $Map`"",
    '"sv_max_clients_per_ip 16"', '"sv_connlimit 100"' # 8 local clients
Start-Sleep 2

# every widget at its default corner (anchor names from hud.rs)
$places = [ordered]@{ watermark = "top_left 16 16"; status = "top_left 16 64"; movement = "bottom_right 16 16"
    killfeed = "top_right 16 16"; island = "top 0 16"; nowplaying = "bottom_left 16 16" }
$off = ($places.Keys | ForEach-Object { "hud_widget $_ $($places[$_]) 0" }) -join "`n"
$on = @("rs_glass 2", "rs_media 1") + ($places.Keys | ForEach-Object { "hud_widget $_ $($places[$_]) 1" }) +
    ("time", "user", "server", "ping", "fps", "speed", "uid", "tps" | ForEach-Object { "hud_set watermark $_ 1" })
$cases = [ordered]@{ "hud off" = "rs_glass 1`n$off"; "everything on" = ($on -join "`n") }
$extra = @()
$bots = @()
if ($Antiping) {
    $cases = [ordered]@{ "antiping off" = "cl_antiping 0"; "antiping on" = "cl_antiping 1" }
    $extra = "--fire", "1"
    $bots = 1..7 | ForEach-Object {
        Start-Process -PassThru -WindowStyle Hidden "$repo\rust-client\target\release\ddnet-headless.exe" `
            -ArgumentList "127.0.0.1:8319", "--name", "bot$_", "--seconds", "600", "--script", "--fire", "1"
        Start-Sleep -Milliseconds 300
    }
    Start-Sleep 2
}
try {
    foreach ($name in $cases.Keys) {
        $appdata = Join-Path $tmp ($name -replace " ", "-")
        New-Item -ItemType Directory -Force "$appdata\DDNet-rs" | Out-Null
        Set-Content -Encoding ascii "$appdata\DDNet-rs\settings.cfg" $cases[$name]
        for ($i = 0; $i -lt $Runs; $i++) {
            $env:APPDATA = $appdata
            $out = & "$repo\rust-client\target\release\ddnet-rs.exe" 127.0.0.1:8319 --size 2560x1440 --no-grab --screenshot-after $Seconds @extra
            "{0}: {1}" -f $name, ($out | Select-String "frame time")
        }
    }
} finally {
    foreach ($b in $bots) { Stop-Process -Id $b.Id -ErrorAction SilentlyContinue }
    Stop-Process -Id $srv.Id
}

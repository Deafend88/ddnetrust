# M7.11 frame time check: DDNet-rs at 2560x1440 (vsync off) on a local server, the HUD
# off vs. every widget and watermark item on with full glass. Needs
# `cargo build --release -p ddnet-rs` and build-rs\DDNet-Server.exe. Uses scratch
# folders only (server and APPDATA), never the real DDNet or DDNet-rs folders.
param([string]$Map = "dm1", [int]$Seconds = 20, [int]$Runs = 3)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$tmp = Join-Path $env:TEMP "ddnet-rs-frametime"
$server = Join-Path $tmp "server"
New-Item -ItemType Directory -Force $server | Out-Null
Set-Content -Encoding ascii (Join-Path $server "storage.cfg") "add_path `$CURRENTDIR`r`nadd_path `$DATADIR"
$srv = Start-Process -PassThru -WindowStyle Hidden -WorkingDirectory $server `
    "$repo\build-rs\DDNet-Server.exe" `
    -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8319"', "`"sv_map $Map`""
Start-Sleep 2

# every widget at its default corner (anchor names from hud.rs)
$places = [ordered]@{ watermark = "top_left 16 16"; status = "top_left 16 64"; movement = "bottom_right 16 16"
    killfeed = "top_right 16 16"; island = "top 0 16"; nowplaying = "bottom_left 16 16" }
$off = ($places.Keys | ForEach-Object { "hud_widget $_ $($places[$_]) 0" }) -join "`n"
$on = @("rs_glass 2", "rs_media 1") + ($places.Keys | ForEach-Object { "hud_widget $_ $($places[$_]) 1" }) +
    ("time", "user", "server", "ping", "fps", "speed", "uid", "tps" | ForEach-Object { "hud_set watermark $_ 1" })
$cases = [ordered]@{ "hud off" = "rs_glass 1`n$off"; "everything on" = ($on -join "`n") }
try {
    foreach ($name in $cases.Keys) {
        $appdata = Join-Path $tmp ($name -replace " ", "-")
        New-Item -ItemType Directory -Force "$appdata\DDNet-rs" | Out-Null
        Set-Content -Encoding ascii "$appdata\DDNet-rs\settings.cfg" $cases[$name]
        for ($i = 0; $i -lt $Runs; $i++) {
            $env:APPDATA = $appdata
            $out = & "$repo\rust-client\target\release\ddnet-rs.exe" 127.0.0.1:8319 --size 2560x1440 --no-grab --screenshot-after $Seconds
            "{0}: {1}" -f $name, ($out | Select-String "frame time")
        }
    }
} finally {
    Stop-Process -Id $srv.Id
}

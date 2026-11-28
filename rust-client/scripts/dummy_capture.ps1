# Captures what the C++ client sends when it connects its dummy (13.6), for
# `ddnet/tests/server.rs dummy_handshake_matches_cpp`. Local only: a DDNet-Server bound to
# 127.0.0.1 with sv_register 0 in a scratch dir; the C++ client in a scratch dir (never
# your %APPDATA%\DDNet) behind a dead local HTTP proxy, talking to the server through
# the udp_tap example, which logs every datagram. The C++ client binds F9 to
# dummy_connect; the script presses F9 in its window once it has joined.
#
# Needs: build-rs\DDNet-Server.exe, build-rs-client\DDNet.exe and
# `cargo build -p ddnet --example udp_tap`.
#
# Usage (repo root): rust-client\scripts\dummy_capture.ps1 [-Out rust-client\tests\golden\dummy_cpp.tap]
param([string]$Out = "rust-client\tests\golden\dummy_cpp.tap", [string]$OutDir = "$env:TEMP\ddnet-rs-dummy")
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$srvDir = "$OutDir\server"; $cDir = "$OutDir\cpp-client"
foreach ($d in $srvDir, $cDir) {
    New-Item -ItemType Directory -Force $d | Out-Null
    Set-Content -Path "$d\storage.cfg" -Value "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
}
$tapLog = "$OutDir\dummy_cpp.tap"
$sargs = '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8303"', '"sv_map dm1"'
$srv = Start-Process -FilePath "$repo\build-rs\DDNet-Server.exe" -ArgumentList $sargs -WorkingDirectory $srvDir -RedirectStandardOutput "$srvDir\server.log" -RedirectStandardError "$srvDir\server.err" -PassThru
$tap = $null; $c = $null
try {
    Start-Sleep -Seconds 2
    $tap = Start-Process -FilePath "$repo\rust-client\target\debug\examples\udp_tap.exe" -ArgumentList "8304", "8303", $tapLog, "20" -PassThru
    $env:ALL_PROXY = "http://127.0.0.1:9"; $env:HTTPS_PROXY = "http://127.0.0.1:9"; $env:HTTP_PROXY = "http://127.0.0.1:9"
    $cargs = '"gfx_backend Vulkan"', '"gfx_fullscreen 0"', '"gfx_borderless 0"', '"gfx_screen_width 800"', '"gfx_screen_height 600"', '"snd_enable 0"', '"logfile cpp.log"', '"player_name main"', '"bind f9 dummy_connect"', '"connect 127.0.0.1:8304"'
    $c = Start-Process -FilePath "$repo\build-rs-client\DDNet.exe" -ArgumentList $cargs -WorkingDirectory $cDir -PassThru
    Start-Sleep -Seconds 5
    # F9 with its scan code (SDL reads scan codes); an Alt tap first lets
    # SetForegroundWindow through Windows' foreground lock
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Keys {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
}
"@
    [Keys]::keybd_event(0x12, 0x38, 0, [UIntPtr]::Zero); [Keys]::keybd_event(0x12, 0x38, 2, [UIntPtr]::Zero)
    [void][Keys]::SetForegroundWindow((Get-Process -Id $c.Id).MainWindowHandle)
    Start-Sleep -Milliseconds 500
    [Keys]::keybd_event(0x78, 0x43, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100; [Keys]::keybd_event(0x78, 0x43, 2, [UIntPtr]::Zero)
    Start-Sleep -Seconds 6
} finally {
    foreach ($p in $c, $srv) { if ($p) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
    if ($tap) { Wait-Process -Id $tap.Id -Timeout 30 -ErrorAction SilentlyContinue }
}
# keep what the dummy (the tap's client 1) sent
$lines = Get-Content $tapLog | Where-Object { ($_ -split ' ')[1] -eq '1' -and ($_ -split ' ')[2] -eq 'c2s' }
Set-Content -Path (Join-Path $repo $Out) -Value $lines -Encoding ascii
Write-Output "captured: $Out ($($lines.Count) datagrams the dummy sent)"

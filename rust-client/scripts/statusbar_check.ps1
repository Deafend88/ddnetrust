# M7.7 live check of the DDRace status bar against the C++ HUD (PLAN 7.7): DDNet-rs plays
# as "subject" and puts its tee into DDRace states with the server's test commands over
# rcon (binds in its scratch settings_ddnet.cfg, keys posted to its window only); the C++
# client spectates it (cl_run_on_join spec subject), and both windows are captured in
# each state. Local only, like compare_screenshots.ps1: DDNet-Server on 127.0.0.1 with
# sv_register 0 from a scratch dir, both clients from scratch dirs (never your
# %APPDATA%\DDNet or DDNet-rs), the C++ client behind a dead local HTTP proxy.
#
# Writes <OutDir>\status_<n>_cpp.png, status_<n>_rust.png and status_<n>.png (both top
# left HUD corners side by side) for the states:
#   0 spawn, 1 all weapons + jetpack + endless jump + endless hook + 5 jumps,
#   2 ninja + solo, 3 deep freeze + live freeze
#
# -Kills (M7.8): the C++ client plays instead (client 1); the states are kill messages
# (kill_pl 1, kill_pl 0, kill_pl 1) and the top right corners (killfeeds) are compared.
#
# Needs: build-rs\DDNet-Server.exe, build-rs-client\DDNet.exe, a debug ddnet-rs build.
# Usage (repo root): rust-client\scripts\statusbar_check.ps1 [-Kills]
param([string]$OutDir = "$env:TEMP\ddnet-rs-status", [switch]$Kills)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$srvDir = "$OutDir\server"; $cDir = "$OutDir\cpp-client"
foreach ($d in $srvDir, $cDir, $OutDir) {
    New-Item -ItemType Directory -Force $d | Out-Null
    Set-Content -Path "$d\storage.cfg" -Value "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
}
$env:APPDATA = "$OutDir\appdata"   # our own settings.cfg too: never your real one
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class StatusCap {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public static void Key(IntPtr h, int vk, int scan) {
        PostMessage(h, 0x100, (IntPtr)vk, (IntPtr)(1 | (scan << 16)));
        System.Threading.Thread.Sleep(80);
        PostMessage(h, 0x101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3 << 30)));
    }
}
"@
[void][StatusCap]::SetProcessDPIAware()
function Save-ClientArea([int]$ProcessId, [string]$Out) {
    $h = (Get-Process -Id $ProcessId).MainWindowHandle
    $wr = New-Object StatusCap+RECT; [void][StatusCap]::GetWindowRect($h, [ref]$wr)
    $cr = New-Object StatusCap+RECT; [void][StatusCap]::GetClientRect($h, [ref]$cr)
    $pt = New-Object StatusCap+POINT; [void][StatusCap]::ClientToScreen($h, [ref]$pt)
    $bmp = New-Object System.Drawing.Bitmap ($wr.R - $wr.L), ($wr.B - $wr.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc(); [void][StatusCap]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
    $rect = New-Object System.Drawing.Rectangle ($pt.X - $wr.L), ($pt.Y - $wr.T), $cr.R, $cr.B
    $bmp.Clone($rect, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb).Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
# the top left corners (C++ HUD, our watermark and status bar) side by side
function Save-Pair([string]$Cpp, [string]$Rust, [string]$Out) {
    $a = [System.Drawing.Image]::FromFile($Cpp); $b = [System.Drawing.Image]::FromFile($Rust)
    $img = New-Object System.Drawing.Bitmap 1000, 200
    $g = [System.Drawing.Graphics]::FromImage($img)
    # the C++ HUD at 2x; with -Kills the right halves (killfeeds) instead of the left
    $cx = if ($Kills) { 600 } else { 0 }; $rx = if ($Kills) { 200 } else { 0 }
    $g.DrawImage($a, (New-Object System.Drawing.Rectangle 0, 0, 400, 200), (New-Object System.Drawing.Rectangle $cx, 0, 200, 100), [System.Drawing.GraphicsUnit]::Pixel)
    $g.DrawImage($b, (New-Object System.Drawing.Rectangle 400, 0, 600, 200), (New-Object System.Drawing.Rectangle $rx, 0, 600, 200), [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose(); $a.Dispose(); $b.Dispose(); $img.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $img.Dispose()
}

# our client's binds: F9 logs in, F5/F7/F8 switch the states
$binds = @(
    'player_name "subject"',
    'bind f9 "rcon_auth test"',
    'bind f5 "rcon weapons; rcon jetpack; rcon infinite_jump; rcon endless_hook; rcon setjumps 5"',
    'bind f7 "rcon unweapons; rcon unjetpack; rcon uninfinite_jump; rcon unendless_hook; rcon setjumps 2; rcon ninja; rcon solo"',
    'bind f8 "rcon unninja; rcon unsolo; rcon deep; rcon livefreeze"'
)
if ($Kills) {
    $binds = 'player_name "subject"', 'bind f9 "rcon_auth test"', 'bind f5 "rcon kill_pl 1"', 'bind f7 "rcon kill_pl 0"', 'bind f8 "rcon kill_pl 1"'
}
Set-Content -Path "$OutDir\settings_ddnet.cfg" -Value $binds -Encoding ascii
$sargs = '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8303"', '"sv_map dm1"', '"sv_rcon_password test"', '"sv_test_cmds 1"'
$srv = Start-Process -FilePath "$repo\build-rs\DDNet-Server.exe" -ArgumentList $sargs -WorkingDirectory $srvDir -RedirectStandardOutput "$srvDir\server.log" -RedirectStandardError "$srvDir\server.err" -PassThru
$c = $null; $r = $null
try {
    Start-Sleep -Seconds 2
    $r = Start-Process -FilePath "$repo\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8303", "--size", "800x600", "--no-grab" -WorkingDirectory $OutDir -RedirectStandardOutput "$OutDir\rust.log" -PassThru
    Start-Sleep -Seconds 4
    $env:ALL_PROXY = "http://127.0.0.1:9"; $env:HTTPS_PROXY = "http://127.0.0.1:9"; $env:HTTP_PROXY = "http://127.0.0.1:9"
    $cargs = '"gfx_fullscreen 0"', '"gfx_borderless 0"', '"gfx_screen_width 800"', '"gfx_screen_height 600"', '"cl_showchat 0"', '"cl_motd_time 0"', '"cl_show_welcome 1"', '"snd_enable 0"', '"logfile cpp.log"', '"connect 127.0.0.1:8303"'
    if (-not $Kills) { $cargs = @('"cl_run_on_join spec subject"') + $cargs }
    $c = Start-Process -FilePath "$repo\build-rs-client\DDNet.exe" -ArgumentList $cargs -WorkingDirectory $cDir -PassThru
    Start-Sleep -Seconds 6
    $r.Refresh(); $h = $r.MainWindowHandle
    # our window on top and focused (winit drops keys while unfocused); keys go to it only
    [void][StatusCap]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 3); [void][StatusCap]::SetForegroundWindow($h)
    [void][StatusCap]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero); Start-Sleep -Milliseconds 300
    [StatusCap]::Key($h, 0x78, 0x43); Start-Sleep -Seconds 1   # F9: rcon_auth
    $keys = @($null, @(0x74, 0x3F), @(0x76, 0x41), @(0x77, 0x42))   # -, F5, F7, F8
    for ($n = 0; $n -lt $keys.Count; $n++) {
        if ($keys[$n]) { [StatusCap]::Key($h, $keys[$n][0], $keys[$n][1]) }
        Start-Sleep -Seconds 2
        Save-ClientArea $c.Id "$OutDir\status_${n}_cpp.png"
        Save-ClientArea $r.Id "$OutDir\status_${n}_rust.png"
        Save-Pair "$OutDir\status_${n}_cpp.png" "$OutDir\status_${n}_rust.png" "$OutDir\status_$n.png"
        "state $n captured"
    }
} finally {
    foreach ($p in $c, $r, $srv) { if ($p) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
}

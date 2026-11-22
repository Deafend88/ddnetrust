# Renders one map in the C++ client and in DDNet-rs at the same moment and compares
# the two pictures (PLAN.md section 9.4). Local only: a DDNet-Server bound to 127.0.0.1
# with sv_register 0, run from a scratch dir; the C++ client also runs from a scratch dir
# (never your %APPDATA%\DDNet) behind a dead local HTTP proxy, so its own web requests
# (server list, DDNet info) fail locally instead of reaching the internet.
#
# The C++ client joins first (client id 0); DDNet-rs joins second and centers on
# player 0 (--follow 0); both windows are captured back to back with PrintWindow.
#
# Needs: build-rs\DDNet-Server.exe, build-rs-client\DDNet.exe (CLIENT=ON, VULKAN=ON),
# a debug build of ddnet-rs and its shotdiff example (cargo build -p ddnet-rs --examples).
#
# Usage (repo root): rust-client\scripts\compare_screenshots.ps1 -Map dm1 [-Mask "340,240,460,340"]
param([string]$Map = "dm1", [string[]]$Mask = @("340,240,460,340"), [int]$Wait = 5, [string]$OutDir = "$env:TEMP\ddnet-rs-compare")
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$srvDir = "$OutDir\server"; $cDir = "$OutDir\cpp-client"
foreach ($d in $srvDir, $cDir) {
    New-Item -ItemType Directory -Force $d | Out-Null
    Set-Content -Path "$d\storage.cfg" -Value "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
}

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinCap {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@
# the client area of a process's main window as PNG (PW_RENDERFULLCONTENT)
function Save-ClientArea([int]$ProcessId, [string]$Out) {
    $h = (Get-Process -Id $ProcessId).MainWindowHandle
    $wr = New-Object WinCap+RECT; [void][WinCap]::GetWindowRect($h, [ref]$wr)
    $cr = New-Object WinCap+RECT; [void][WinCap]::GetClientRect($h, [ref]$cr)
    $pt = New-Object WinCap+POINT; [void][WinCap]::ClientToScreen($h, [ref]$pt)
    $bmp = New-Object System.Drawing.Bitmap ($wr.R - $wr.L), ($wr.B - $wr.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc(); [void][WinCap]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
    $rect = New-Object System.Drawing.Rectangle ($pt.X - $wr.L), ($pt.Y - $wr.T), $cr.R, $cr.B
    $bmp.Clone($rect, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb).Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
}

$name = $Map -replace ' ', '_'
$srv = Start-Process -FilePath "$repo\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8303"', "`"sv_map $Map`"" -WorkingDirectory $srvDir -RedirectStandardOutput "$srvDir\server.log" -RedirectStandardError "$srvDir\server.err" -PassThru
$c = $null; $r = $null
try {
    Start-Sleep -Seconds 2
    $env:ALL_PROXY = "http://127.0.0.1:9"; $env:HTTPS_PROXY = "http://127.0.0.1:9"; $env:HTTP_PROXY = "http://127.0.0.1:9"
    $cargs = '"gfx_backend Vulkan"', '"gfx_fullscreen 0"', '"gfx_borderless 0"', '"gfx_screen_width 800"', '"gfx_screen_height 600"', '"cl_showhud 0"', '"cl_showchat 0"', '"cl_nameplates 0"', '"cl_showkillmessages 0"', '"cl_motd_time 0"', '"cl_show_welcome 1"', '"br_indicate_finished 0"', '"snd_enable 0"', '"logfile cpp.log"', '"connect 127.0.0.1:8303"'
    $c = Start-Process -FilePath "$repo\build-rs-client\DDNet.exe" -ArgumentList $cargs -WorkingDirectory $cDir -PassThru
    Start-Sleep -Seconds 3
    $r = Start-Process -FilePath "$repo\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8303", "--size", "800x600", "--follow", "0" -WorkingDirectory $OutDir -PassThru
    Start-Sleep -Seconds $Wait
    Save-ClientArea $c.Id "$OutDir\${name}_cpp.png"
    Save-ClientArea $r.Id "$OutDir\${name}_rust.png"
} finally {
    foreach ($p in $c, $r, $srv) { if ($p) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
}
$maskArgs = foreach ($m in $Mask) { "--mask"; $m }
& "$repo\rust-client\target\debug\examples\shotdiff.exe" "$OutDir\${name}_cpp.png" "$OutDir\${name}_rust.png" "$OutDir\${name}_diff.png" @maskArgs

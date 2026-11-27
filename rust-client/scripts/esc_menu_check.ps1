# M9 13.5 Esc menu captures: build-rs/DDNet-Server.exe (127.0.0.1:8309, Sunny Side Up, a
# name and a two-line MOTD; scratch dir %TEMP%\ddnet-rs-esc\server), two ddnet-headless
# bots (Bob, Carol) for the Players tab, then target/debug/ddnet-rs.exe as "Alice"
# (APPDATA in the scratch dir) once per tab with --esc-tab; the window is captured
# (PrintWindow: the in-game screenshot has no UI) to <Out>\esc_<tab>.png.
# Run `cargo build` first.
param([string]$Out = "$PSScriptRoot\..\docs\review\13.5")
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@
function Capture($h, $file) {
  $wr = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$wr)
  $cr = New-Object W+RECT; [void][W]::GetClientRect($h, [ref]$cr)
  $pt = New-Object W+POINT; [void][W]::ClientToScreen($h, [ref]$pt)
  $bmp = New-Object System.Drawing.Bitmap ($wr.R - $wr.L), ($wr.B - $wr.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $dc = $g.GetHdc(); [void][W]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $rect = New-Object System.Drawing.Rectangle ($pt.X - $wr.L), ($pt.Y - $wr.T), $cr.R, $cr.B
  $bmp.Clone($rect, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb).Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
}

$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$dir = "$env:TEMP\ddnet-rs-esc"
$server = "$dir\server"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $server, "$dir\appdata\DDNet-rs", $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"', 'add_friend "Carol" ""' -Encoding utf8
$env:APPDATA = "$dir\appdata"   # never the real settings
$motd = 'Welcome to the DDNet-rs test server.\nBe nice and have fun.'
$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8309"', '"sv_map Sunny Side Up"', '"sv_name DDNet-rs local test"', '"sv_max_clients_per_ip 16"', "`"sv_motd $motd`"" -WorkingDirectory $server -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\server.log"
try {
  Start-Sleep 2
  foreach ($name in "Bob", "Carol") {
    Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8309", "--name", $name, "--seconds", "90" -WindowStyle Hidden | Out-Null
  }
  Start-Sleep 2
  foreach ($tab in 0, 1, 2) {
    $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8309", "--size", "1672x941", "--no-grab", "--esc-tab", $tab -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\client_$tab.log"
    Start-Sleep 7; $cl.Refresh()
    Capture $cl.MainWindowHandle "$Out\esc_$tab.png"
    # a clean leave (WM_CLOSE disconnects), so the next run is the only Alice
    [void]$cl.CloseMainWindow(); [void]$cl.WaitForExit(5000); Start-Sleep 1
    "saved $Out\esc_$tab.png"
  }
} finally {
  Get-Process ddnet-rs, ddnet-headless -ErrorAction SilentlyContinue | Stop-Process
  Stop-Process $srv -ErrorAction SilentlyContinue
}

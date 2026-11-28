# M9 13.7 vote captures: build-rs/DDNet-Server.exe (127.0.0.1:8311, Sunny Side Up, vote
# options from myServerconfig.cfg; scratch dir %TEMP%\ddnet-rs-vote\server), two
# ddnet-headless bots (Bob, Carol) so a vote stays open, then target/debug/ddnet-rs.exe as
# "Alice" (APPDATA in the scratch dir): one run calls a vote with --exec and captures the
# notice (vote_notice.png), one opens the Vote tab on each page (vote_tab_<page>.png).
# The window is captured with PrintWindow (the in-game screenshot has no UI).
# Run `cargo build` first.
param([string]$Out = "$PSScriptRoot\..\docs\review\13.7")
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
$dir = "$env:TEMP\ddnet-rs-vote"
$server = "$dir\server"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $server, "$dir\appdata\DDNet-rs", $Out | Out-Null
$Out = (Resolve-Path $Out).Path
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
Set-Content "$server\myServerconfig.cfg" @(
  'sv_vote_delay 0', 'sv_vote_time 60', 'sv_vote_majority 1', 'clear_votes',
  'add_vote "Change map to dm1" "change_map dm1"',
  'add_vote "Change map to Sunny Side Up" "change_map Sunny Side Up"',
  'add_vote "Restart round" "restart"',
  'add_vote "Turn solo off" "sv_solo_server 0"',
  'add_vote "Turn solo on" "sv_solo_server 1"'
) -Encoding ascii
# the caller votes yes at once; C++ hides the notice after voting unless
# cl_show_votes_after_voting is on
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"', 'cl_show_votes_after_voting 1' -Encoding utf8
$env:APPDATA = "$dir\appdata"   # never the real settings
$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8311"', '"sv_map Sunny Side Up"', '"sv_name DDNet-rs local test"', '"sv_max_clients_per_ip 16"' -WorkingDirectory $server -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\server.log"
try {
  Start-Sleep 2
  # other loopback addresses: the server counts one vote per IP
  foreach ($bot in @(@("Bob", "127.0.0.2"), @("Carol", "127.0.0.3"))) {
    Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8311", "--name", $bot[0], "--from", $bot[1], "--script", "--seconds", "120" -WindowStyle Hidden | Out-Null
  }
  Start-Sleep 2
  $runs = @(
    @{ File = "vote_notice"; Args = @("--exec", '"3:callvote option \"Change map to dm1\" fun"'); Wait = 8 },
    @{ File = "vote_tab_0"; Args = @("--esc-tab", "3"); Wait = 7 },
    @{ File = "vote_tab_1"; Args = @("--esc-tab", "3", "--esc-vote-page", "1"); Wait = 7 }
  )
  foreach ($run in $runs) {
    $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList (@("127.0.0.1:8311", "--size", "1672x941", "--no-grab") + $run.Args) -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\client_$($run.File).log"
    Start-Sleep $run.Wait; $cl.Refresh()
    Capture $cl.MainWindowHandle "$Out\$($run.File).png"
    [void]$cl.CloseMainWindow(); [void]$cl.WaitForExit(5000); Start-Sleep 1
    "saved $Out\$($run.File).png"
  }
} finally {
  Get-Process ddnet-rs, ddnet-headless -ErrorAction SilentlyContinue | Stop-Process
  Stop-Process $srv -ErrorAction SilentlyContinue
}

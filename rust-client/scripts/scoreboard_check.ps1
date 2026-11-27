# M9 13.2 scoreboard live check: build-rs/DDNet-Server.exe (127.0.0.1:8307, dm1,
# sv_test_cmds 1, practice on, scratch rcon password; scratch dir
# %TEMP%\ddnet-rs-scoreboard\server), target/debug/ddnet-rs.exe as "Alice" (APPDATA in the
# scratch dir) and three ddnet-headless bots: Bob idles, Carol and Dave say "/team 3" (a
# DDRace team in the scoreboard). Keys go to our window only: Alice says "/teleport Bob"
# (practice) and steps off him (his name plate beside her), then logs in to rcon (F8) and runs the
# test command "freeze" (F9) for her own freeze bar. Captures: the world, then the
# scoreboard while Tab is held. The server log must show the team joins.
# Run `cargo build` first.
param([string]$Out = "$env:TEMP\ddnet-rs-scoreboard")
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  public static void Key(IntPtr h, int vk) {
    int scan = (int)MapVirtualKey((uint)vk, 0);
    // arrow keys are extended keys (else they read as the number pad's)
    if (vk >= 0x25 && vk <= 0x28) scan |= 0x100;
    PostMessage(h, 0x100, (IntPtr)vk, (IntPtr)(1 | (scan << 16)));
    System.Threading.Thread.Sleep(40);
    PostMessage(h, 0x101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3 << 30)));
    System.Threading.Thread.Sleep(40);
  }
  // typed text, whatever the keyboard layout: each character as F13 (no character in
  // any layout) with the character's WM_CHAR, which winit reads as the key's text
  public static void Text(IntPtr h, string s) {
    int scan = (int)MapVirtualKey(0x7C, 0);
    foreach (char c in s) {
      PostMessage(h, 0x100, (IntPtr)0x7C, (IntPtr)(1 | (scan << 16)));
      PostMessage(h, 0x102, (IntPtr)c, (IntPtr)(1 | (scan << 16)));
      System.Threading.Thread.Sleep(30);
      PostMessage(h, 0x101, (IntPtr)0x7C, (IntPtr)(1 | (scan << 16) | (3 << 30)));
      System.Threading.Thread.Sleep(30);
    }
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@
function Capture($h, $file) {
  $wr = New-Object K+RECT; [void][K]::GetWindowRect($h, [ref]$wr)
  $cr = New-Object K+RECT; [void][K]::GetClientRect($h, [ref]$cr)
  $pt = New-Object K+POINT; [void][K]::ClientToScreen($h, [ref]$pt)
  $bmp = New-Object System.Drawing.Bitmap ($wr.R - $wr.L), ($wr.B - $wr.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $dc = $g.GetHdc(); [void][K]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $rect = New-Object System.Drawing.Rectangle ($pt.X - $wr.L), ($pt.Y - $wr.T), $cr.R, $cr.B
  $bmp.Clone($rect, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb).Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
}

$root = "C:\ddnetrust-master"
$dir = "$env:TEMP\ddnet-rs-scoreboard"
$server = "$dir\server"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $server, "$dir\appdata\DDNet-rs", $Out | Out-Null
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"', 'bind f8 "rcon_auth test"', 'bind f9 "rcon freeze"' -Encoding utf8
$env:APPDATA = "$dir\appdata"   # never the real settings
$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8307"', '"sv_map dm1"', '"sv_test_cmds 1"', '"sv_practice_by_default 1"', '"sv_rcon_password test"' -WorkingDirectory $server -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\server.log"
Start-Sleep 1
try {
  $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8307", "--size", "1280x720", "--no-grab" -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\client.log"
  Start-Sleep 2
  $bots = @(
    @("Bob", '"hi"'),
    @("Carol", '"/team 3"'),
    @("Dave", '"/team 3"')
  ) | ForEach-Object {
    Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8307", "--name", $_[0], "--say", $_[1], "--seconds", "40" -PassThru -WindowStyle Hidden
  }
  Start-Sleep 10; $cl.Refresh(); $h = $cl.MainWindowHandle
  [K]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 300
  [K]::Key($h, 0x54); Start-Sleep -Milliseconds 200; [K]::Text($h, "/teleport Bob"); [K]::Key($h, 0x0D); Start-Sleep 1.5
  # step off Bob (+right on D): on top of him, his tee and gun showed behind her frozen one
  $scan = [K]::MapVirtualKey(0x44, 0)
  [K]::PostMessage($h, 0x100, [IntPtr]0x44, [IntPtr](1 -bor ($scan -shl 16))) | Out-Null
  Start-Sleep -Milliseconds 250
  [K]::PostMessage($h, 0x101, [IntPtr]0x44, [IntPtr](1 -bor ($scan -shl 16) -bor (3 -shl 30))) | Out-Null
  Start-Sleep 1
  [K]::Key($h, 0x77); Start-Sleep -Milliseconds 500; [K]::Key($h, 0x78); Start-Sleep -Milliseconds 800
  Capture $h "$Out\world.png"
  Start-Sleep 2.5; Capture $h "$Out\world2.png"
  # hold Tab (+scoreboard)
  $scan = [K]::MapVirtualKey(0x09, 0)
  [K]::PostMessage($h, 0x100, [IntPtr]0x09, [IntPtr](1 -bor ($scan -shl 16))) | Out-Null
  Start-Sleep -Milliseconds 600
  Capture $h "$Out\scoreboard.png"
  [K]::PostMessage($h, 0x101, [IntPtr]0x09, [IntPtr](1 -bor ($scan -shl 16) -bor (3 -shl 30))) | Out-Null
  Start-Sleep -Milliseconds 400
  Capture $h "$Out\after.png"
} finally {
  Get-Process ddnet-rs, ddnet-headless -ErrorAction SilentlyContinue | Stop-Process
  Stop-Process -Id $srv.Id -ErrorAction SilentlyContinue; Start-Sleep 1
}
$log = Get-Content "$dir\server.log"
$log | Select-String "team|freeze" | Select-Object -First 10
"captures in $Out"

# M9 13.1 chat live check: build-rs/DDNet-Server.exe (127.0.0.1:8306, dm1, scratch dir
# %TEMP%\ddnet-rs-chat\server), target/debug/ddnet-rs.exe as "Alice" (APPDATA in the scratch
# dir) and ddnet-headless as "Bob", who says "hi Alice" every 5 s (a highlight, folded into
# one line). Keys go to our window only (posted messages): Enter opens the chat, "bo" + Tab
# completes Bob's name, Enter sends; Y opens the team chat; T + "/pa" shows the command
# hint, Tab completes it, Escape closes. Window captures go to -Out; the server log must
# show both lines with the right team.
# Run `cargo build` first.
param([string]$Out = "$env:TEMP\ddnet-rs-chat")
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
$dir = "$env:TEMP\ddnet-rs-chat"
$server = "$dir\server"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $server, "$dir\appdata\DDNet-rs", $Out | Out-Null
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"' -Encoding utf8
$env:APPDATA = "$dir\appdata"   # never the real settings
$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8306"', '"sv_map dm1"' -WorkingDirectory $server -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\server.log"
Start-Sleep 1
try {
  $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8306", "--size", "1280x720", "--no-grab" -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\client.log"
  Start-Sleep 3
  $bob = Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8306", "--name", "Bob", "--say", '"hi Alice"', "--seconds", "40" -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\bob.log"
  Start-Sleep 8; $cl.Refresh(); $h = $cl.MainWindowHandle
  [K]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 300
  Capture $h "$Out\chat_closed.png"
  # all chat: Enter, "bo", Tab -> "Bob: ", " hello", Enter
  [K]::Key($h, 0x0D); Start-Sleep -Milliseconds 200
  [K]::Text($h, "bo"); [K]::Key($h, 0x09); [K]::Text($h, "hello there")
  Start-Sleep -Milliseconds 300; Capture $h "$Out\chat_open.png"
  [K]::Key($h, 0x0D); Start-Sleep 1.2
  # team chat: Y, text, Enter
  [K]::Key($h, 0x59); Start-Sleep -Milliseconds 200; [K]::Text($h, "team plan"); [K]::Key($h, 0x0D); Start-Sleep 1.2
  # a long line wraps (a rounded rectangle instead of a pill)
  [K]::Key($h, 0x54); Start-Sleep -Milliseconds 200
  [K]::Text($h, "this is a long message that does not fit on one line of the chat, so it wraps onto a second line"); [K]::Key($h, 0x0D); Start-Sleep 1.2
  # command hint and completion: T, "/pa", capture, Tab, capture, Escape
  [K]::Key($h, 0x54); Start-Sleep -Milliseconds 200; [K]::Text($h, "/pa"); Start-Sleep -Milliseconds 300
  Capture $h "$Out\chat_hint.png"
  [K]::Key($h, 0x09); Start-Sleep -Milliseconds 300; Capture $h "$Out\chat_completed.png"
  [K]::Key($h, 0x1B); Start-Sleep -Milliseconds 500
  # history: T, Up -> the last sent line, Escape
  [K]::Key($h, 0x54); Start-Sleep -Milliseconds 200; [K]::Key($h, 0x26); Start-Sleep -Milliseconds 300
  Capture $h "$Out\chat_history.png"
  [K]::Key($h, 0x1B); Start-Sleep 2
  Capture $h "$Out\chat_after.png"
} finally {
  Get-Process ddnet-rs, ddnet-headless -ErrorAction SilentlyContinue | Stop-Process
  Stop-Process -Id $srv.Id -ErrorAction SilentlyContinue; Start-Sleep 1
}
$log = Get-Content "$dir\server.log"
$all = $log | Select-String -SimpleMatch "Alice: Bob: hello there"
$team = $log | Select-String -SimpleMatch "Alice: team plan"
"server chat lines:"; $log | Select-String "chat"
if ($all -and $team) { "OK: both lines reached the server"; "captures in $Out" } else { "MISSING chat lines"; exit 1 }

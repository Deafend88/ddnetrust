# M7.4 live check of the HUD editor: starts build-rs/DDNet-Server.exe (127.0.0.1:8305,
# dm1, scratch dir %TEMP%\ddnet-rs-menu\server with its own storage.cfg) and
# target/debug/ddnet-rs.exe with APPDATA in that scratch dir (the real settings are never
# touched). Opens the editor with F6, drags the watermark (messages posted to our window
# only; it is made topmost so the cursor stays on it), clicks it, closes with Esc, prints
# the saved hud lines, restarts and captures ed_restart.png. Run a plain cargo build first.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static POINT Origin;
  public static void Mouse(IntPtr h, uint m, int x, int y, int btn) { SetCursorPos(Origin.X + x, Origin.Y + y); PostMessage(h, m, (IntPtr)btn, (IntPtr)((y << 16) | (x & 0xffff))); }
  public static void Key(IntPtr h, int vk, int scan) {
    PostMessage(h, 0x100, (IntPtr)vk, (IntPtr)(1 | (scan << 16)));
    System.Threading.Thread.Sleep(80);
    PostMessage(h, 0x101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3 << 30)));
  }
  public struct RECT { public int L, T, R, B; }
  public struct POINT { public int X, Y; }
}
"@
[W]::SetProcessDPIAware() | Out-Null
$root = "C:\ddnetrust-master"
$dir = "$env:TEMP\ddnet-rs-menu"
$appdata = "$dir\appdata"
Remove-Item -Recurse -Force $appdata -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $appdata | Out-Null
$env:APPDATA = $appdata   # our settings.cfg goes here, never the real one

function Capture($h, $out) {
  $r = New-Object W+RECT; [W]::GetClientRect($h, [ref]$r) | Out-Null
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
  [W]::PrintWindow($h, $dc, 3) | Out-Null
  $g.ReleaseHdc($dc); $g.Dispose(); $bmp.Save($out); $bmp.Dispose()
}
function Start-Client($log = "rs.log") {
  $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8305", "--size", "800x600", "--no-grab" -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\$log"
  Start-Sleep 6; $cl.Refresh(); $cl
}

$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8305"', '"sv_map dm1"' -WorkingDirectory "$dir\server" -PassThru -WindowStyle Hidden
Start-Sleep 1
try {
  $cl = Start-Client; $h = $cl.MainWindowHandle
  # topmost (our window only), so the cursor placed on it stays inside it
  [W]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 3) | Out-Null
  $o = New-Object W+POINT; [W]::ClientToScreen($h, [ref]$o) | Out-Null; [W]::Origin = $o
  [W]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 300; [W]::Key($h, 0x75, 0x40); Start-Sleep 1   # F6
  Capture $h "$dir\ed_open.png"
  # drag the watermark from (60, 48) to (60 + 400, 48 + 350) in client pixels
  [W]::Mouse($h, 0x200, 60, 48, 0); Start-Sleep -Milliseconds 200
  $pt = New-Object W+POINT; $pt.X = $o.X + 60; $pt.Y = $o.Y + 48; "window under the cursor: $([W]::WindowFromPoint($pt)) (ours: $h)"
  [W]::Mouse($h, 0x201, 60, 48, 1); Start-Sleep -Milliseconds 100
  for ($i = 1; $i -le 20; $i++) {
    [W]::Mouse($h, 0x200, 60 + 20 * $i, 48 + [int](17.5 * $i), 1); Start-Sleep -Milliseconds 50
  }
  Start-Sleep -Milliseconds 300
  Capture $h "$dir\ed_drag.png"
  [W]::Mouse($h, 0x202, 460, 398, 0); Start-Sleep -Milliseconds 300
  # click it: settings popover
  [W]::Mouse($h, 0x201, 460, 398, 1); Start-Sleep -Milliseconds 80; [W]::Mouse($h, 0x202, 460, 398, 0); Start-Sleep 1
  Capture $h "$dir\ed_popover.png"
  [W]::Key($h, 0x1B, 0x01); Start-Sleep 1   # Esc
  Stop-Process -Id $cl.Id
  "settings.cfg hud lines:"; Get-Content "$appdata\DDNet-rs\settings.cfg" | Select-String "hud_|f6"
  $cl = Start-Client "rs2.log"; Capture $cl.MainWindowHandle "$dir\ed_restart.png"; Stop-Process -Id $cl.Id
  "done"
} finally {
  Get-Process ddnet-rs -ErrorAction SilentlyContinue | Stop-Process
  Stop-Process -Id $srv.Id -ErrorAction SilentlyContinue
}

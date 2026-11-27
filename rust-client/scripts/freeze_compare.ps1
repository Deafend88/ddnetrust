# Frozen tee, C++ vs DDNet-rs (13.2 review): build-rs/DDNet-Server.exe (127.0.0.1:8309, dm1,
# sv_test_cmds 1, scratch rcon password, 60 s freezes, no initial AFK; restarted for each
# client, so both spawn at the same spot) from a scratch dir;
# build-rs-client/DDNet.exe joins alone (scratch storage, dead proxy), F8 logs in to rcon
# and F9 runs the test command $Cmd (freeze, deep or livefreeze); captured, it quits;
# then target/debug/ddnet-rs.exe does the same. Captures in $Out: cpp_normal.png and
# rust_normal.png (before the freeze), cpp.png and rust.png. Run `cargo build` first.
param([string]$Out = "$env:TEMP\ddnet-rs-freeze", [string]$Cmd = "freeze", [switch]$Blocker)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K2 {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  public static void Key(IntPtr h, int vk) {
    int scan = (int)MapVirtualKey((uint)vk, 0);
    PostMessage(h, 0x100, (IntPtr)vk, (IntPtr)(1 | (scan << 16)));
    System.Threading.Thread.Sleep(60);
    PostMessage(h, 0x101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3 << 30)));
    System.Threading.Thread.Sleep(60);
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
  $wr = New-Object K2+RECT; [void][K2]::GetWindowRect($h, [ref]$wr)
  $cr = New-Object K2+RECT; [void][K2]::GetClientRect($h, [ref]$cr)
  $pt = New-Object K2+POINT; [void][K2]::ClientToScreen($h, [ref]$pt)
  $bmp = New-Object System.Drawing.Bitmap ($wr.R - $wr.L), ($wr.B - $wr.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $dc = $g.GetHdc(); [void][K2]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
  $rect = New-Object System.Drawing.Rectangle ($pt.X - $wr.L), ($pt.Y - $wr.T), $cr.R, $cr.B
  $bmp.Clone($rect, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb).Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
}
$root = "C:\ddnetrust-master"
$dir = "$Out\run"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$dir\server", "$dir\cpp", "$dir\appdata\DDNet-rs", $Out | Out-Null
foreach ($d in "$dir\server", "$dir\cpp", $dir) { Set-Content "$d\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii }
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"', 'bind f8 "rcon_auth test"', "bind f9 `"rcon $Cmd`"" -Encoding utf8
$appData = $env:APPDATA; $env:APPDATA = "$dir\appdata"
# one server per client: a killed client stays connected until it times out, and its
# tee would push the next one to another spawn (the cameras must match). -Blocker: a
# headless bot holds the first spawn, so both clients spawn at the second one.
function Start-Server($log) {
  $p = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8309"', '"sv_map dm1"', '"sv_test_cmds 1"', '"sv_rcon_password test"', '"sv_freeze_delay 60"', '"sv_max_afk_time 0"' -WorkingDirectory "$dir\server" -PassThru -WindowStyle Hidden -RedirectStandardOutput $log
  Start-Sleep 1
  if ($Blocker) {
    Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8309", "--name", "Blocker", "--seconds", "20" -WindowStyle Hidden
    Start-Sleep 2
  }
  $p
}
$env:ALL_PROXY = "http://127.0.0.1:9"; $env:HTTPS_PROXY = "http://127.0.0.1:9"; $env:HTTP_PROXY = "http://127.0.0.1:9"
$runs = @(
  @("cpp", { Start-Process "$root\build-rs-client\DDNet.exe" -ArgumentList '"gfx_backend Vulkan"', '"gfx_fullscreen 0"', '"gfx_borderless 0"', '"gfx_screen_width 1280"', '"gfx_screen_height 720"', '"snd_enable 0"', '"cl_show_welcome 0"', '"cl_motd_time 0"', '"player_name Cpp"', '"bind f8 \"rcon_auth test\""', "`"bind f9 \`"rcon $Cmd\`"`"", '"connect 127.0.0.1:8309"' -WorkingDirectory "$dir\cpp" -PassThru }),
  @("rust", { Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8309", "--size", "1280x720", "--no-grab" -WorkingDirectory $dir -PassThru })
)
foreach ($run in $runs) {
  $name = $run[0]
  $srv = Start-Server "$dir\server_$name.log"
  try {
    $c = & $run[1]
    Start-Sleep 6; $c.Refresh(); $h = $c.MainWindowHandle
    [K2]::SetForegroundWindow($h) | Out-Null; [K2]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 300
    Capture $h "$Out\${name}_normal.png"
    [K2]::Key($h, 0x77); Start-Sleep -Milliseconds 600; [K2]::Key($h, 0x78); Start-Sleep 1.5
    Capture $h "$Out\$name.png"
  } finally {
    Get-Process ddnet-rs, DDNet, ddnet-headless -ErrorAction SilentlyContinue | Stop-Process -Force
    Stop-Process -Id $srv.Id -ErrorAction SilentlyContinue; Start-Sleep 1
  }
}
$env:APPDATA = $appData
Get-Content "$dir\server_*.log" | Select-String "rcon|auth"

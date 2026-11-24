# M7.5 HUD on/off input check (PLAN 7.3): the inputs the server receives are the same
# with every watermark item on and with the HUD off. Each run starts
# build-rs/DDNet-Server.exe (127.0.0.1:8305, dm1, scratch dir %TEMP%\ddnet-rs-input\server,
# sv_tee_historian 1) and target/debug/ddnet-rs.exe with APPDATA in the scratch dir, sends
# the same keys to our window only (right, jump, left, hook... as posted messages), then
# reads the server's teehistorian with examples/teehistorian_inputs and compares the input
# sequences (each line is a change, so frame timing does not matter).
# Run `cargo build --example teehistorian_inputs` (and a plain cargo build) first.
param([switch]$Mutate)   # -Mutate: the HUD-off run skips one key, so the check must fail
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Runtime.InteropServices;
public class K {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static void Down(IntPtr h, int vk, int scan) { PostMessage(h, 0x100, (IntPtr)vk, (IntPtr)(1 | (scan << 16))); }
  public static void Up(IntPtr h, int vk, int scan) { PostMessage(h, 0x101, (IntPtr)vk, (IntPtr)(1 | (scan << 16) | (3 << 30))); }
}
"@
$root = "C:\ddnetrust-master"
$dir = "$env:TEMP\ddnet-rs-input"
$server = "$dir\server"
New-Item -ItemType Directory -Force "$server\teehistorian" | Out-Null
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
$env:APPDATA = "$dir\appdata"   # our settings.cfg goes here, never the real one

function Run($name, $hudLines, $skipJump) {
  Remove-Item -Recurse -Force $env:APPDATA, "$server\teehistorian\*" -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force "$env:APPDATA\DDNet-rs" | Out-Null
  Set-Content "$env:APPDATA\DDNet-rs\settings.cfg" $hudLines -Encoding utf8
  $srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8305"', '"sv_map dm1"', '"sv_tee_historian 1"' -WorkingDirectory $server -PassThru -WindowStyle Hidden
  Start-Sleep 1
  try {
    $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8305", "--size", "800x600", "--no-grab" -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\$name.log"
    Start-Sleep 6; $cl.Refresh(); $h = $cl.MainWindowHandle
    [K]::PostMessage($h, 0x0007, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 300
    # right 600 ms with a jump in it, left 600 ms, hook (right mouse) 300 ms
    [K]::Down($h, 0x44, 0x20); Start-Sleep -Milliseconds 300
    if (-not $skipJump) { [K]::Down($h, 0x20, 0x39); Start-Sleep -Milliseconds 100; [K]::Up($h, 0x20, 0x39) }
    Start-Sleep -Milliseconds 200; [K]::Up($h, 0x44, 0x20); Start-Sleep -Milliseconds 200
    [K]::Down($h, 0x41, 0x1E); Start-Sleep -Milliseconds 600; [K]::Up($h, 0x41, 0x1E); Start-Sleep -Milliseconds 200
    [K]::PostMessage($h, 0x204, [IntPtr]2, [IntPtr]((300 -shl 16) -bor 400)) | Out-Null; Start-Sleep -Milliseconds 300
    [K]::PostMessage($h, 0x205, [IntPtr]0, [IntPtr]((300 -shl 16) -bor 400)) | Out-Null; Start-Sleep 1
    Stop-Process -Id $cl.Id; Start-Sleep 1
  } finally {
    Get-Process ddnet-rs -ErrorAction SilentlyContinue | Stop-Process
    Stop-Process -Id $srv.Id -ErrorAction SilentlyContinue; Start-Sleep 1
  }
  $file = Get-ChildItem "$server\teehistorian\*.teehistorian" | Select-Object -First 1
  & "$root\rust-client\target\debug\examples\teehistorian_inputs.exe" $file.FullName
}

$all = 'hud_set "watermark" "server" "1"', 'hud_set "watermark" "time" "1"', 'hud_set "watermark" "uid" "1"', 'hud_set "watermark" "tps" "1"'
$on = Run "on" $all $false
$off = Run "off" 'hud_widget "watermark" top_left 16 16 0' $Mutate
"HUD on: $($on.Count) input changes"; $on
"HUD off: $($off.Count) input changes"
if (($on -join "`n") -eq ($off -join "`n") -and $on.Count -gt 3) { "SAME inputs" } else { "DIFFERENT inputs"; $off; exit 1 }

# M9 13.3 sound smoke run: build-rs/DDNet-Server.exe (127.0.0.1:8307, "Sunny Side Up", which
# has a map sound; scratch dir %TEMP%\ddnet-rs-sound\server), target/debug/ddnet-rs.exe as
# "Alice" firing the gun (--fire, predicted gun sounds), ddnet-headless as "Bob" saying
# "hi Alice" (highlight sound). Prints the client's stderr (the sound load line, any
# error) and the client's exit state. The listening check is done by a person: run the
# same with your speakers on.
# Run `cargo build` first.
param([int]$Seconds = 15)
$ErrorActionPreference = "Stop"
$root = "C:\ddnetrust-master"
$dir = "$env:TEMP\ddnet-rs-sound"
$server = "$dir\server"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $server, "$dir\appdata\DDNet-rs" | Out-Null
Set-Content "$server\storage.cfg" "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
Set-Content "$dir\appdata\DDNet-rs\settings.cfg" 'player_name "Alice"' -Encoding utf8
$env:APPDATA = "$dir\appdata"   # never the real settings
$srv = Start-Process "$root\build-rs\DDNet-Server.exe" -ArgumentList '"sv_register 0"', '"bindaddr 127.0.0.1"', '"sv_port 8307"', '"sv_map Sunny Side Up"' -WorkingDirectory $server -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\server.log"
Start-Sleep 1
try {
  $cl = Start-Process "$root\rust-client\target\debug\ddnet-rs.exe" -ArgumentList "127.0.0.1:8307", "--size", "1280x720", "--no-grab", "--fire", "1" -WorkingDirectory $dir -PassThru -RedirectStandardOutput "$dir\client.log" -RedirectStandardError "$dir\client.err"
  Start-Sleep 3
  $bob = Start-Process "$root\rust-client\target\debug\ddnet-headless.exe" -ArgumentList "127.0.0.1:8307", "--name", "Bob", "--say", '"hi Alice"', "--seconds", "$Seconds" -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\bob.log"
  Start-Sleep $Seconds
  $cl.Refresh()
  "client running: $(-not $cl.HasExited)"
} finally {
  Get-Process -Id $cl.Id -ErrorAction SilentlyContinue | Stop-Process -Force
  Stop-Process -Id $srv.Id -Force -ErrorAction SilentlyContinue
}
Get-Content "$dir\client.err"

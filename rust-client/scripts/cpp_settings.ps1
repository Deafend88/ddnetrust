# Lets the C++ client (build-rs-client\DDNet.exe) load settings files and save them again,
# for rust-client/client/tests/input.rs (settings and binds parity). Each run uses a fresh
# scratch dir whose storage.cfg keeps the client off %APPDATA%\DDNet, behind a dead local
# HTTP proxy (as in compare_screenshots.ps1); the client starts, runs `quit` and saves
# settings_ddnet.cfg into the scratch dir.
#
#   defaults.saved.cfg  what a client with no settings file saves (default binds)
#   corpus.saved.cfg    what it saves after loading corpus.cfg as its settings file
#
# Usage (repo root): rust-client\scripts\cpp_settings.ps1
$ErrorActionPreference = "Stop"
$repo = Resolve-Path "$PSScriptRoot\..\.."
$golden = "$repo\rust-client\tests\golden\settings"
$env:ALL_PROXY = "http://127.0.0.1:9"; $env:HTTPS_PROXY = "http://127.0.0.1:9"; $env:HTTP_PROXY = "http://127.0.0.1:9"
foreach ($run in @(@{ Name = "defaults"; In = $null }, @{ Name = "corpus"; In = "$golden\corpus.cfg" })) {
    $dir = "$env:TEMP\ddnet-rs-settings-$($run.Name)"
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory $dir | Out-Null
    Set-Content -Path "$dir\storage.cfg" -Value "add_path `$CURRENTDIR`nadd_path `$DATADIR" -Encoding ascii
    if ($run.In) { Copy-Item $run.In "$dir\settings_ddnet.cfg" }
    $p = Start-Process -FilePath "$repo\build-rs-client\DDNet.exe" -ArgumentList '"gfx_fullscreen 0"', '"snd_enable 0"', '"cl_show_welcome 0"', '"quit"' -WorkingDirectory $dir -PassThru
    if (-not $p.WaitForExit(60000)) { Stop-Process -Id $p.Id -Force; throw "the C++ client did not quit" }
    Copy-Item "$dir\settings_ddnet.cfg" "$golden\$($run.Name).saved.cfg"
    Write-Output "$($run.Name): $((Get-Content "$golden\$($run.Name).saved.cfg").Count) lines"
}

# Windows validation for a cross-built kit (docs/testing/windows-validation.md).
# Runs every test binary, the GPU tests with BUGATRON_REQUIRE_GPU=1 on the
# Direct3D 12 backend, and scripted game runs that dump frames and record audio.
# Results go to .\results (and are zipped to results.zip).
#
#   powershell -ExecutionPolicy Bypass -File run-validation.ps1 [-Game "C:\...\Bugatron Gold"]
param(
    [string]$Game = "C:\Program Files (x86)\Bugatron Gold"
)
$ErrorActionPreference = "Continue"
$kit = $PSScriptRoot
$out = Join-Path $kit "results"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
$env:BUGATRON_ROOT = (Join-Path $kit "tests")
# SDL reports the GPU driver and adapter (e.g. "D3D12 Adapter: Microsoft Basic Render Driver").
$env:SDL_LOGGING = "gpu=info"
$log = Join-Path $out "summary.txt"

function Note($s) { $s | Tee-Object -FilePath $log -Append }

Note "machine: $env:COMPUTERNAME  arch: $env:PROCESSOR_ARCHITECTURE  $(Get-Date -Format o)"
Note ((Get-CimInstance Win32_OperatingSystem).Caption + " " + (Get-CimInstance Win32_OperatingSystem).Version)
Get-CimInstance Win32_VideoController | ForEach-Object { Note "video: $($_.Name) ($($_.DriverVersion))" }

# 1. Test binaries
foreach ($t in Get-ChildItem (Join-Path $kit "tests") -Filter *.exe) {
    $name = $t.BaseName
    $env:BUGATRON_REQUIRE_GPU = $null
    $env:BUGATRON_TRACE = $null
    if ($name -eq "gpu") { $env:BUGATRON_REQUIRE_GPU = "1"; $env:BUGATRON_TRACE = "gpu" }
    $o = & $t.FullName 2>&1 | Out-String
    $o | Set-Content (Join-Path $out "test-$name.txt")
    $res = ($o -split "`n" | Where-Object { $_ -match "^test result" }) -join " | "
    Note ("{0,-28} exit {1}  {2}" -f $name, $LASTEXITCODE, $res)
}
$env:BUGATRON_REQUIRE_GPU = $null

# 2. Scripted game runs (the host keyboard and mouse are ignored during them)
$exe = Join-Path $Game "Bug3D.exe"
$bug = Join-Path $kit "bugatron.exe"
function Run($tag, $runArgs) {
    $dir = Join-Path $out $tag
    New-Item -ItemType Directory -Force $dir | Out-Null
    $env:BUGATRON_TRACE = "gpu"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & $bug --exe $exe --data $Game --save (Join-Path $dir "save") --dump-dir $dir @runArgs *> (Join-Path $dir "log.txt")
    $code = $LASTEXITCODE
    $env:BUGATRON_TRACE = $null
    Note ("game {0,-10} exit {1}  {2:n1} s  {3}" -f $tag, $code, $sw.Elapsed.TotalSeconds, ((Get-ChildItem $dir -Filter *.png | ForEach-Object Name) -join ","))
}
Run "title" @("--video-mode", "640x480x32", "--frames", "710", "--dump-frames", "139,450", "--record-audio", (Join-Path $out "title\title.wav"), "12")
$script = Get-Content (Join-Path $kit "score-input.txt") -Raw
Run "play" @("--video-mode", "640x480x32", "--input", $script.Trim(), "--frames", "8010", "--dump-frames", "2000,4000,6000,8000")
Run "title16" @("--video-mode", "640x480x16", "--frames", "710", "--dump-frames", "139,450")

Compress-Archive -Force -Path (Join-Path $out "*") -DestinationPath (Join-Path $kit "results.zip")
Note "done: $(Join-Path $kit 'results.zip')"

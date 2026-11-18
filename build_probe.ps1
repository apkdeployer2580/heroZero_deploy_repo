# Host build of hz_probe — the headless verification loop.
#
# `cargo build --release -p hz_probe` does NOT work on a stock windows-gnu host: rustc
# materialises `raw-dylib` imports (parking_lot_core -> windows-targets) by shelling out
# to `dlltool`, which the bundled gnu toolchain does not surface on PATH. This puts a
# working one there. Same block as hero2NativeAPP's build_probe.ps1; see
# tools/dlltool_shim.rs for why a renamed llvm-dlltool is not enough.
param([switch]$Run, [int]$Frames = 600, [string]$Jar = "")

$ErrorActionPreference = 'Stop'

$Sdk = @(
    $env:ANDROID_HOME
    "$env:LOCALAPPDATA\Android\Sdk"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
$NdkBin = if ($Sdk) { "$Sdk\ndk\25.1.8937393\toolchains\llvm\prebuilt\windows-x86_64\bin" } else { $null }

$Shim = Join-Path $env:TEMP 'wie-dlltool'
New-Item -ItemType Directory -Force $Shim | Out-Null
$RealDlltool = Join-Path $Shim 'real-llvm-dlltool.exe'
$llvmDlltool = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-dlltool.exe"
if (Test-Path $llvmDlltool) {
    Copy-Item $llvmDlltool $RealDlltool -Force
} elseif ($NdkBin -and (Test-Path "$NdkBin\llvm-ar.exe")) {
    # llvm-ar is a multicall binary keyed on argv[0]: a copy named llvm-dlltool IS one.
    Copy-Item "$NdkBin\llvm-ar.exe" $RealDlltool -Force
} else {
    throw "No llvm-dlltool available (looked in the rustup gnu toolchain and $NdkBin)."
}

& "$env:USERPROFILE\.cargo\bin\rustc.exe" -O --edition 2021 "$PSScriptRoot\tools\dlltool_shim.rs" -o (Join-Path $Shim 'dlltool.exe')
if ($LASTEXITCODE -ne 0) { throw "could not build the dlltool shim ($LASTEXITCODE)" }

$env:WIE_REAL_DLLTOOL = $RealDlltool
$env:PATH = "$Shim;$env:USERPROFILE\.cargo\bin;$env:PATH" + $(if ($NdkBin) { ";$NdkBin" } else { '' })

# PS 5.1 + 'Stop' turns a native exe's stderr into a terminating error even on exit 0.
# Every native call below checks $LASTEXITCODE itself.
$ErrorActionPreference = 'Continue'

& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release -p hz_probe
if ($LASTEXITCODE -ne 0) { throw "hz_probe build failed ($LASTEXITCODE)" }
Write-Output "==> target\release\hz_probe.exe"

if ($Run) {
    if (-not $Jar) { $Jar = "$PSScriptRoot\android_app\app\src\main\assets\herozero.jar" }
    if (-not (Test-Path $Jar)) {
        throw "no game jar at $Jar — stage it with:`n  python tools_re/make_hz_asset.py `"dist\영웅서기제로(최종버전).zip`""
    }
    $Out = Join-Path $env:TEMP 'hz-probe'
    New-Item -ItemType Directory -Force $Out | Out-Null
    & "$PSScriptRoot\target\release\hz_probe.exe" --jar $Jar --w 240 --h 320 --ms 20 --frames $Frames --every 100 --out $Out
    Write-Output "==> frames in $Out"
}

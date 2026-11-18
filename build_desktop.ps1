# Build 영웅서기 Zero for the PC (Windows): hz_desktop -> herozero.exe, staged with the game next to it.
#
#   .\build_desktop.ps1                 # release exe + dist\herozero_pc_<versionCode>\ (and its zip)
#   .\build_desktop.ps1 -Debug          # the dev profile (opt-level 3, debuginfo), no staging
#   .\build_desktop.ps1 -Run            # …and start what was built, from the staged folder
#
# ⛔ MSVC, not the windows-gnu host toolchain the probe uses: with the gnu toolchain + the dlltool
# shim the audio stack (cpal -> windows 0.62, raw-dylib) segfaults before `main` (2026-10-29). The
# msvc toolchain is in rustup (`stable-x86_64-pc-windows-msvc`); its own target dir keeps the gnu
# artefacts of `target\` untouched.
#
# The staged folder is a HAND-OFF of licensed content (the jar): it goes to the person who owns the
# game, never into git (`dist/` is ignored). Same rule as the APK's (SIDELOAD.md).
param(
    [switch]$Debug,
    [switch]$Run,
    # Build with a jar whose md5 is not a known build. hz_patches then applies nothing.
    [switch]$AnyJar
)
$ErrorActionPreference = 'Stop'

$Cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
if (-not (Test-Path $Cargo)) { throw 'cargo not found (rustup).' }
$Toolchain = 'stable-x86_64-pc-windows-msvc'
& "$env:USERPROFILE\.cargo\bin\rustup.exe" run $Toolchain rustc --version | Out-Null
if ($LASTEXITCODE -ne 0) { throw "the $Toolchain toolchain is not installed: rustup toolchain install $Toolchain" }
$TargetDir = "$PSScriptRoot\target_desk"

# ── 1) the game and its derived assets (the APK's slot; build_android.ps1's rules) ──────────
$Assets = "$PSScriptRoot\android_app\app\src\main\assets"
$GameAsset = "$Assets\herozero.jar"
if (-not (Test-Path $GameAsset)) {
    throw "no game jar at $GameAsset - stage it with:`n  python -I tools_re/make_hz_asset.py `"dist\영웅서기제로(최종버전).zip`""
}
if (-not $AnyJar) {
    $md5 = (Get-FileHash -Algorithm MD5 $GameAsset).Hash.ToLower()
    $known = Select-String -Path "$PSScriptRoot\hz_patches\src\lib.rs" -Pattern $md5 -SimpleMatch -Quiet
    if (-not $known) { throw "the jar's md5 $md5 is not a build hz_patches knows (pass -AnyJar to build anyway: no patches)." }
}
$Fanfare = "$Assets\hz_fanfare.wav"
$WorldMap = "$Assets\hz_worldmap.rgb565"
if (-not (Test-Path $Fanfare)) { Write-Warning "no $Fanfare (build_android.ps1 renders it when the SoundFont is found): the fanfare will be silent" }
if (-not (Test-Path $WorldMap)) { Write-Warning "no $WorldMap (build_android.ps1 draws it): the world map will be a flat sea" }

# ── 2) the version the APK carries, into the exe (진단 정보, the transfer file's app check) ──
$Props = Get-Content "$PSScriptRoot\android_app\version.properties" | Where-Object { $_ -match '^\s*version(Code|Name)\s*=' }
$VersionCode = ($Props | Where-Object { $_ -match '^\s*versionCode' }) -replace '^\s*versionCode\s*=\s*', '' | Select-Object -First 1
$VersionName = ($Props | Where-Object { $_ -match '^\s*versionName' }) -replace '^\s*versionName\s*=\s*', '' | Select-Object -First 1
$env:HEROZERO_VERSION_CODE = "$VersionCode".Trim()
$env:HEROZERO_VERSION_NAME = "$VersionName".Trim()
Write-Output "==> version $env:HEROZERO_VERSION_NAME ($env:HEROZERO_VERSION_CODE)"

# ── 3) cargo (msvc) ──────────────────────────────────────────────────────────
$ErrorActionPreference = 'Continue'
# (one array, built up: `if` unwraps a one-element array into a string, and a splatted string goes
# to cargo one character at a time — `-`, `-`, `r`… — measured 2026-10-29)
$CargoArgs = @("+$Toolchain", 'build', '-p', 'hz_desktop', '--target-dir', $TargetDir)
if (-not $Debug) { $CargoArgs += '--release' }
& $Cargo @CargoArgs
if ($LASTEXITCODE -ne 0) { throw "hz_desktop build failed ($LASTEXITCODE)" }
$ErrorActionPreference = 'Stop'
$Exe = if ($Debug) { "$TargetDir\debug\herozero.exe" } else { "$TargetDir\release\herozero.exe" }
Write-Output "==> $Exe"

# ── 4) stage: exe + the game next to it ──────────────────────────────────────
if (-not $Debug) {
    $Stage = "$PSScriptRoot\dist\herozero_pc_$env:HEROZERO_VERSION_CODE"
    if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
    New-Item -ItemType Directory -Force $Stage | Out-Null
    Copy-Item $Exe "$Stage\herozero.exe"
    Copy-Item $GameAsset "$Stage\herozero.jar"
    if (Test-Path $Fanfare) { Copy-Item $Fanfare "$Stage\hz_fanfare.wav" }
    if (Test-Path $WorldMap) { Copy-Item $WorldMap "$Stage\hz_worldmap.rgb565" }
    @"
영웅서기 Zero PC $env:HEROZERO_VERSION_NAME ($env:HEROZERO_VERSION_CODE)

herozero.exe 를 실행하면 됩니다 (같은 폴더의 herozero.jar 가 게임입니다).
세이브·설정: %APPDATA%\heroZero
내보내기·세이브 옮기기 폴더: 문서\heroZero (받을 zip 은 문서\heroZero\save_import 에)

키: 방향키 이동(두 개를 같이 누르면 대각선), X 공격, Space·Enter 대화·확인, Esc 취소(필드 = 메뉴, 창 = 뒤로),
    퀵슬롯 7칸 = Q W E R A S D, Tab = 퀵슬롯 다른 쪽(1/2 <-> 2/2),
    숫자 0~9 = 게임 메뉴의 번호 키(* = 숫자패드 *, # = 숫자패드 -),
    F5 저장, M 월드맵, F11 전체 화면. 마우스 왼쪽 = 터치(퀵슬롯 칸 클릭 = 발동), 오른쪽 = 뒤로, 휠 = 목록 스크롤.
    게임패드(XInput) 지원 - 폰과 같은 버튼 배치.
"@ | Set-Content -Encoding UTF8 "$Stage\읽어보세요.txt"
    $Zip = "$Stage.zip"
    if (Test-Path $Zip) { Remove-Item -Force $Zip }
    # Without a console (agent shells) Compress-Archive's progress bar makes it return with no zip
    # and no error (pc 50, pc 51 — 2026-10-30).
    $ProgressPreference = 'SilentlyContinue'
    Compress-Archive -Path "$Stage\*" -DestinationPath $Zip
    Write-Output "==> $Stage"
    Write-Output "==> $Zip ($([math]::Round((Get-Item $Zip).Length / 1MB, 1)) MB)"
    if ($Run) { Start-Process -FilePath "$Stage\herozero.exe" -WorkingDirectory $Stage }
} elseif ($Run) {
    Start-Process -FilePath $Exe -ArgumentList @('--assets', "`"$Assets`"")
}

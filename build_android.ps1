# Build the 영웅서기 Zero APK: Rust cdylib (hz_android, via cargo-ndk) -> Gradle GameActivity APK.
#
#   .\build_android.ps1                    # debug APK
#   .\build_android.ps1 -Install -Serial X # also adb-install + launch on device X
#   .\build_android.ps1 -FastDebug         # release-optimised native code in a debug-signed APK
#                                          # (measure performance with this; installs over debug)
#   .\build_android.ps1 -Release           # release (LTO) + versionCode bump + signing
#   .\build_android.ps1 -Release -NoBump   # rebuild the SAME versionCode
#
# Stages (port-shipping order): discover-tools -> host-shim -> bump-version -> asset gate + icons
# -> cargo-ndk -> strip -> verify-artifact -> gradle -> verify-signature -> install.
#
# Signing secrets are NEVER in this repo: HEROZERO_KEYSTORE / _KEYSTORE_PASSWORD / _KEY_ALIAS /
# _KEY_PASSWORD in the environment, or herozero.keystore / .keystorePassword / .keyAlias /
# .keyPassword in %USERPROFILE%\.gradle\gradle.properties (see android_app/app/build.gradle.kts).
# Stage 1b also looks in %USERPROFILE%\.android\keystores for a gradle.properties with those keys.
param(
    [switch]$Release,
    [switch]$FastDebug,
    [switch]$Install,
    [string]$Serial,
    [switch]$NoBump,
    # Build with a jar whose md5 is not a known build. hz_patches then applies nothing.
    [switch]$AnyJar
)
$ErrorActionPreference = 'Stop'

# ── 1) discover-tools ────────────────────────────────────────────────────────
$Sdk = @($env:ANDROID_HOME, "$env:LOCALAPPDATA\Android\Sdk") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $Sdk) { throw 'Android SDK not found. Set $env:ANDROID_HOME.' }
$Ndk = "$Sdk\ndk\25.1.8937393"
$NdkBin = "$Ndk\toolchains\llvm\prebuilt\windows-x86_64\bin"
if (-not (Test-Path $Ndk)) { throw "NDK 25.1.8937393 not found under $Sdk\ndk (gradle's ndkVersion pins this revision)." }
$env:ANDROID_HOME = $Sdk
$env:ANDROID_NDK_HOME = $Ndk
$env:JAVA_HOME = @(
    $env:JAVA_HOME
    'C:\Program Files\Eclipse Adoptium\jdk-21.0.11.10-hotspot'
    'C:\Program Files\Microsoft\jdk-21.0.11.10-hotspot'
    'C:\Program Files\Android\Android Studio\jbr'
) | Where-Object { $_ -and (Test-Path (Join-Path $_ 'bin\java.exe')) } | Select-Object -First 1
if (-not $env:JAVA_HOME) { throw 'No JDK 17+ found. Set $env:JAVA_HOME.' }
$Adb = @($env:HEROZERO_ADB, "$Sdk\platform-tools\adb.exe") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($Install -and -not $Adb) { throw 'adb not found. Set $env:HEROZERO_ADB.' }
$Python = @('python', 'py') | Where-Object { Get-Command $_ -ErrorAction SilentlyContinue } | Select-Object -First 1
if (-not $Python) { throw 'python not found (needed for the icon generator).' }

# ── 1b) signing-secrets (release only) ───────────────────────────────────────
# The keystore + its gradle.properties live OUTSIDE the repo.
# gradle reads HEROZERO_* from the environment first, so when neither the environment nor
# %USERPROFILE%\.gradle\gradle.properties has a complete herozero.* set, the first folder that has
# one is read here and handed over as environment variables. If the jks path inside that file does
# not exist, a file of the same name next to the properties file is used.
# Nothing is printed and nothing is written.
if ($Release -and -not ($env:HEROZERO_KEYSTORE -and $env:HEROZERO_KEYSTORE_PASSWORD -and $env:HEROZERO_KEY_ALIAS -and $env:HEROZERO_KEY_PASSWORD)) {
    $KeystoreDirs = @("$env:USERPROFILE\.gradle", "$env:USERPROFILE\.android\keystores")
    foreach ($dir in $KeystoreDirs) {
        $props = Join-Path $dir 'gradle.properties'
        if (-not (Test-Path $props)) { continue }
        $kv = @{}
        Get-Content $props | ForEach-Object {
            if ($_ -match '^\s*(herozero\.[A-Za-z]+)\s*=\s*(.*?)\s*$') { $kv[$matches[1]] = $matches[2] }
        }
        $need = @('herozero.keystore', 'herozero.keystorePassword', 'herozero.keyAlias', 'herozero.keyPassword')
        if (($need | Where-Object { -not $kv[$_] }).Count -gt 0) { continue }
        if ($dir -eq "$env:USERPROFILE\.gradle") { break }   # gradle reads this one itself
        $jks = $kv['herozero.keystore']
        if (-not (Test-Path $jks)) {
            $local = Join-Path $dir (Split-Path $jks -Leaf)
            if (Test-Path $local) { $jks = $local } else { Write-Warning "==> $props names $jks, which is not here; skipping"; continue }
        }
        $env:HEROZERO_KEYSTORE = $jks
        $env:HEROZERO_KEYSTORE_PASSWORD = $kv['herozero.keystorePassword']
        $env:HEROZERO_KEY_ALIAS = $kv['herozero.keyAlias']
        $env:HEROZERO_KEY_PASSWORD = $kv['herozero.keyPassword']
        Write-Output "==> release signing from $props (keystore $jks, alias $($kv['herozero.keyAlias']))"
        break
    }
}

# ── 2) host-shim (windows-gnu: rustc shells out to dlltool for raw-dylib build deps) ──
$Shim = Join-Path $env:TEMP 'wie-dlltool'
New-Item -ItemType Directory -Force $Shim | Out-Null
$RealDlltool = Join-Path $Shim 'real-llvm-dlltool.exe'
$llvmDlltool = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-dlltool.exe"
if (Test-Path $llvmDlltool) { Copy-Item $llvmDlltool $RealDlltool -Force }
elseif (Test-Path "$NdkBin\llvm-ar.exe") { Copy-Item "$NdkBin\llvm-ar.exe" $RealDlltool -Force }  # multicall binary keyed on argv[0]
else { throw "No llvm-dlltool available (rustup gnu toolchain or $NdkBin)." }
& "$env:USERPROFILE\.cargo\bin\rustc.exe" -O --edition 2021 "$PSScriptRoot\tools\dlltool_shim.rs" -o (Join-Path $Shim 'dlltool.exe')
if ($LASTEXITCODE -ne 0) { throw "could not build the dlltool shim ($LASTEXITCODE)" }
$env:WIE_REAL_DLLTOOL = $RealDlltool
$env:PATH = "$Shim;$env:USERPROFILE\.cargo\bin;$env:JAVA_HOME\bin;$env:PATH;$NdkBin"
# ⚠ A CARGO_TARGET_DIR left in this shell would send the .so elsewhere and package a stale one.
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue

$JniLibs = "$PSScriptRoot\android_app\app\src\main\jniLibs"
$VersionFile = "$PSScriptRoot\android_app\version.properties"
$GameAsset = "$PSScriptRoot\android_app\app\src\main\assets\herozero.jar"
$KnownMd5 = '850ad70748b9e1e48b30f6fa60164229'   # hz_patches::Build — the only build this app knows

# ── 3) bump-version (release only, never on debug) ───────────────────────────
if ($Release -and -not $NoBump) {
    $lines = Get-Content $VersionFile
    $codeLine = $lines | Where-Object { $_ -match '^\s*versionCode\s*=' }
    if (-not $codeLine) { throw "no versionCode= line in $VersionFile" }
    $cur = [int]($codeLine -replace '^\s*versionCode\s*=\s*', '')
    $next = $cur + 1
    Set-Content -Path $VersionFile -Encoding ascii -Value ($lines -replace '^\s*versionCode\s*=.*', "versionCode=$next")
    Write-Output "==> versionCode $cur -> $next"
}

# ── 4) asset gate + icons ────────────────────────────────────────────────────
if (-not (Test-Path $GameAsset)) {
    throw "missing game asset $GameAsset — stage it with:`n  python -I tools_re/make_hz_asset.py `"dist/영웅서기제로(최종버전).zip`""
}
$md5 = (Get-FileHash -Algorithm MD5 $GameAsset).Hash.ToLower()
if ($md5 -ne $KnownMd5) {
    if (-not $AnyJar) { throw "herozero.jar md5 $md5 is not the known build $KnownMd5 (pass -AnyJar to build it unpatched anyway)" }
    Write-Warning "==> UNKNOWN jar md5 $md5 — the app will run it with NO patches"
}
Write-Output ("==> game asset: herozero.jar ({0:N0} bytes, md5 {1})" -f (Get-Item $GameAsset).Length, $md5)
& $Python -I "$PSScriptRoot\tools_re\gen_app_icon.py" $GameAsset "$PSScriptRoot\android_app\app\src\main\res"
if ($LASTEXITCODE -ne 0) { throw "icon generation failed ($LASTEXITCODE)" }

# ── 4b) the MIDI fanfare, rendered (Phase 4b) ────────────────────────────────
# `/snd/b/15` through oxisynth + a GM SoundFont on THIS machine -> assets/hz_fanfare.wav. The font
# is not in the repo: set HEROZERO_SF3 to render it again. Without the font the rendered copy that
# comes with the repository is used.
$Fanfare = "$PSScriptRoot\android_app\app\src\main\assets\hz_fanfare.wav"
$Sf3 = @($env:HEROZERO_SF3) |
    Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($Sf3) {
    $prevTarget = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = "$PSScriptRoot\target_render"   # a host build; keeps the NDK target dir clean
    & cargo run --release -q -p hz_render -- --jar $GameAsset --sf3 $Sf3 --out $Fanfare
    $rc = $LASTEXITCODE
    if ($prevTarget) { $env:CARGO_TARGET_DIR = $prevTarget } else { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue }
    if ($rc -ne 0) { throw "hz_render failed ($rc)" }
} elseif (Test-Path $Fanfare) {
    Write-Output "==> MIDI fanfare: the rendered copy in the repository ($Fanfare)"
} else {
    Write-Warning "==> no GM SoundFont (set HEROZERO_SF3) and no rendered fanfare: it will be silent in this build"
}

# ── 4c) the world map's painted picture (Phase 7-4) ─────────────────────────
# Our own pixel art, procedural and seeded: tools_re/hz_worldmap_draw.py reads the places from
# hz_android/src/worldmap_view.rs and writes assets/hz_worldmap.rgb565 (7 MB, gitignored — derived).
# A detailed painting of the same composition replaces it when there is one (user decision
# 2026-10-23): HEROZERO_WORLDMAP_ART, or art\hz_worldmap.png — made from the generator's --prompt
# brief and --png layout. Redrawn when the source changes or is newer than the asset (which source
# made it is kept in app\build, never in assets). Without numpy/Pillow the app still builds; its
# world map then draws a flat sea.
$WorldMap = "$PSScriptRoot\android_app\app\src\main\assets\hz_worldmap.rgb565"
$WorldArt = @($env:HEROZERO_WORLDMAP_ART, "$PSScriptRoot\art\hz_worldmap.png") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
$WorldSrc = if ($WorldArt) { @($WorldArt) } else { @("$PSScriptRoot\tools_re\hz_worldmap_draw.py", "$PSScriptRoot\hz_android\src\worldmap_view.rs") }
$WorldStamp = "$PSScriptRoot\android_app\app\build\hz_worldmap.source"
$WorldFrom = if ($WorldArt) { "art $WorldArt" } else { 'generator' }
$stale = -not (Test-Path $WorldMap) -or -not (Test-Path $WorldStamp) -or ((Get-Content $WorldStamp -Raw).Trim() -ne $WorldFrom) -or
    ($WorldSrc | Where-Object { (Get-Item $_).LastWriteTime -gt (Get-Item $WorldMap).LastWriteTime })
if ($stale) {
    $ErrorActionPreference = 'Continue'   # a numpy warning on stderr must not stop the build (PS 5.1)
    if ($WorldArt) {
        Write-Output "==> world map picture from the painting $WorldArt"
        & $Python -I "$PSScriptRoot\tools_re\hz_worldmap_draw.py" --art $WorldArt --rgb565 $WorldMap
    } else {
        & $Python -I "$PSScriptRoot\tools_re\hz_worldmap_draw.py" --rgb565 $WorldMap
    }
    if ($LASTEXITCODE -eq 0) {
        New-Item -ItemType Directory -Force (Split-Path $WorldStamp) | Out-Null
        Set-Content -Path $WorldStamp -Encoding utf8 -Value $WorldFrom
    }
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "==> the world map picture could not be drawn ($LASTEXITCODE): the map will be a flat sea"
        Remove-Item $WorldMap -ErrorAction SilentlyContinue
    }
} else {
    Write-Output "==> world map picture up to date ($WorldMap)"
}

# PS 5.1 + 'Stop' turns a native exe's stderr into a terminating error even on exit 0; every
# native call below checks $LASTEXITCODE itself.
$ErrorActionPreference = 'Continue'

# ── 5) cargo-ndk  6) strip ───────────────────────────────────────────────────
$SoPath = Join-Path $JniLibs 'arm64-v8a\libmain.so'
$NativeRelease = $Release -or $FastDebug
Write-Output "==> cargo-ndk build ($(if ($NativeRelease) {'release'} else {'debug'}))$(if ($FastDebug) {' [FastDebug: release native code, debug-signed APK]'})"
Push-Location "$PSScriptRoot\hz_android"
try {
    $ndkArgs = @('-t', 'arm64-v8a', '--platform', '26', '-o', $JniLibs, 'build')
    if ($NativeRelease) { $ndkArgs += '--release' }
    & cargo ndk @ndkArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed ($LASTEXITCODE)" }
} finally { Pop-Location }
Write-Output '==> strip libmain.so'
& "$NdkBin\llvm-strip.exe" --strip-all $SoPath
if ($LASTEXITCODE -ne 0) { throw "llvm-strip failed ($LASTEXITCODE)" }

# ── 7) verify-artifact ───────────────────────────────────────────────────────
# What the .so must carry, read from the ARTIFACT, not from the source: the save key (renaming it
# orphans every save), the patch table (HZ-001's property key), the J2ME key-code profile
# (without it the soft keys never reach the game), the hook class the HZ-004..007 rewrites call
# and the 「추가 설정」 option file (without them a patched class meets NoClassDefFoundError / the
# page's values are not kept). NOT the build md5: at opt-level 3 the compare
# against that 32-byte literal is lowered to immediate loads and the hex string is not contiguous
# in the binary (measured 2026-10-22).
$ascii = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($SoPath))
$missing = @('herozero.jar', 'EA-Porting-Test-SMS-Mode', 'wie.keyCodes', 'net/wie/hz/Hooks', 'hz_exp_rate.txt') | Where-Object { -not $ascii.Contains($_) }
if ($missing) { throw "libmain.so does not contain: $($missing -join ', ')" }
Write-Output ("==> verified libmain.so ({0:N0} bytes): save key + patch table + hooks present" -f (Get-Item $SoPath).Length)

# ── 8) gradle ────────────────────────────────────────────────────────────────
$task = if ($Release) { ':app:assembleRelease' } else { ':app:assembleDebug' }
Write-Output "==> gradle $task"
Push-Location "$PSScriptRoot\android_app"
try {
    & .\gradlew.bat $task --no-daemon
    if ($LASTEXITCODE -ne 0) { throw "gradle failed ($LASTEXITCODE)" }
} finally { Pop-Location }

$apk = if ($Release) {
    $signed = "$PSScriptRoot\android_app\app\build\outputs\apk\release\app-release.apk"
    if (Test-Path $signed) { $signed } else { "$PSScriptRoot\android_app\app\build\outputs\apk\release\app-release-unsigned.apk" }
} else {
    "$PSScriptRoot\android_app\app\build\outputs\apk\debug\app-debug.apk"
}
Write-Output "==> APK: $apk"

# ── 9) verify-signature ──────────────────────────────────────────────────────
$bt = Get-ChildItem "$Sdk\build-tools" -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
if ($Release) {
    if ($apk -like '*unsigned*') { Write-Warning '==> This APK is UNSIGNED. It will not install; do not hand it out.' }
    $apksigner = if ($bt) { Join-Path $bt.FullName 'apksigner.bat' } else { $null }
    if ($apksigner -and (Test-Path $apksigner)) {
        & $apksigner verify --print-certs $apk 2>&1 | Select-String -Pattern 'SHA-256 digest|DOES NOT VERIFY|Signer #1 certificate DN' | ForEach-Object { "    $_" }
    }
}

# A named copy in dist/ (gitignored) so a handed-out file says what it is.
if ($apk -notlike '*unsigned*') {
    $props = Get-Content $VersionFile
    $code = ($props | Where-Object { $_ -match '^\s*versionCode\s*=' }) -replace '^\s*versionCode\s*=\s*', ''
    $name = ($props | Where-Object { $_ -match '^\s*versionName\s*=' }) -replace '^\s*versionName\s*=\s*', ''
    New-Item -ItemType Directory -Force "$PSScriptRoot\dist" | Out-Null
    $out = "$PSScriptRoot\dist\herozero_$name.$code$(if ($FastDebug) {'_fastdebug'} elseif (-not $Release) {'_debug'}).apk"
    Copy-Item $apk $out -Force
    Write-Output "==> $out"
}

# ── 10) install ──────────────────────────────────────────────────────────────
if ($Install) {
    if ($Serial) { $env:ANDROID_SERIAL = $Serial }
    if (-not $env:ANDROID_SERIAL) {
        $ready = @(& $Adb devices -l | Select-String -Pattern '^(\S+)\s+device\s' | ForEach-Object {
            $model = if ($_.Line -match 'model:(\S+)') { $Matches[1] } else { '?' }
            [pscustomobject]@{ Serial = $_.Matches.Groups[1].Value; Model = $model }
        })
        if ($ready.Count -eq 1) { $env:ANDROID_SERIAL = $ready[0].Serial }
        elseif ($ready.Count -gt 1) {
            $lines = $ready | ForEach-Object { "      $($_.Model)  ->  .\build_android.ps1 -Install -Serial $($_.Serial)" }
            throw "$($ready.Count) devices ready - refusing to choose which phone gets the build:`n$($lines -join "`n")"
        } else { throw 'no device in `device` state (unauthorized? accept the USB debugging prompt).' }
    }
    Write-Output "==> adb install on $env:ANDROID_SERIAL"
    & $Adb install -r $apk
    if ($LASTEXITCODE -ne 0) { throw "adb install failed ($LASTEXITCODE)" }
    # The launch component comes from the APK itself, never from applicationId.
    $badging = if ($bt) { & (Join-Path $bt.FullName 'aapt2.exe') dump badging $apk 2>$null } else { $null }
    $pkg = ($badging | Select-String -Pattern "^package: name='([^']+)'").Matches.Groups[1].Value
    $act = ($badging | Select-String -Pattern "^launchable-activity: name='([^']+)'").Matches.Groups[1].Value
    if ($pkg -and $act) {
        & $Adb shell am force-stop $pkg
        & $Adb shell am start -n "$pkg/$act"
    } else { Write-Warning '    could not read the launch component from the APK; start it by hand' }
}

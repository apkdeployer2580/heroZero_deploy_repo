import java.util.Properties

plugins {
    id("com.android.application")
}

// ── version ─────────────────────────────────────────────────────────────────
// ../version.properties, so `build_android.ps1 -Release` can bump the code without editing this
// file. Missing -> 1 / "0.1.0", so a fresh clone still builds.
val versionProps = Properties().apply {
    val f = rootProject.file("version.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
val appVersionCode = (versionProps.getProperty("versionCode") ?: "1").trim().toInt()
val appVersionName = (versionProps.getProperty("versionName") ?: "0.1.0").trim()

// ── release signing ─────────────────────────────────────────────────────────
// Secrets live OUTSIDE the repo, first complete set wins:
//   1. environment:  HEROZERO_KEYSTORE, HEROZERO_KEYSTORE_PASSWORD, HEROZERO_KEY_ALIAS, HEROZERO_KEY_PASSWORD
//   2. %USERPROFILE%\.gradle\gradle.properties:
//        herozero.keystore, herozero.keystorePassword, herozero.keyAlias, herozero.keyPassword
// ⚠ This project's OWN prefix. hero2 once read hero3's keys and signed com.hero2remake.app as
// `CN=Hero3 WIE` (caught before release) — one shared keystore file, one alias per project.
// Without a complete set the release build is UNSIGNED and cannot be installed; it says so below.
fun secret(env: String, prop: String): String? =
    (System.getenv(env) ?: providers.gradleProperty(prop).orNull)?.takeIf { it.isNotBlank() }

val ksPath = secret("HEROZERO_KEYSTORE", "herozero.keystore")
val ksPass = secret("HEROZERO_KEYSTORE_PASSWORD", "herozero.keystorePassword")
val ksAlias = secret("HEROZERO_KEY_ALIAS", "herozero.keyAlias")
val ksKeyPass = secret("HEROZERO_KEY_PASSWORD", "herozero.keyPassword")
val ksFile = ksPath?.let { file(it) }
val canSign = ksFile != null && ksFile.exists() && ksPass != null && ksAlias != null && ksKeyPass != null

android {
    // ★ `namespace` (the JAVA package) stays `net.wie.emulator`: the Rust side finds MainActivity
    // BY NAME over JNI (hz_android/src/java.rs). Moving it builds cleanly and breaks orientation,
    // vibration and exit at runtime. Only `applicationId` is this app's identity.
    namespace = "net.wie.emulator"
    compileSdk = 36
    // cargo-ndk 4.1.2 links with -z max-page-size=16384, so this NDK already meets the 16 KB page
    // requirement (hero2, measured on the artifact with llvm-readelf + zipalign -P 16).
    ndkVersion = "25.1.8937393"

    defaultConfig {
        // ★ PERMANENT once a build is handed out: it is the on-device data path
        // (/data/data/<applicationId>/ — the saves) and the store identity. Proposed in
        // HEROZERO_REMAKE_PLAN.md §9; changing it later makes a different app with no saves.
        applicationId = "com.herozeroremake.app"
        minSdk = 26
        targetSdk = 36
        versionCode = appVersionCode
        versionName = appVersionName
        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    if (canSign) {
        signingConfigs {
            create("release") {
                storeFile = ksFile
                storePassword = ksPass
                keyAlias = ksAlias
                keyPassword = ksKeyPass
                enableV1Signing = true
                enableV2Signing = true
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            if (canSign) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // libmain.so arrives stripped from cargo-ndk (build_android.ps1); AGP must not strip again.
    packaging {
        jniLibs {
            keepDebugSymbols += "**/*.so"
        }
    }

    // The 27 MB jar is read whole at boot (AAssetManager); storing it uncompressed avoids an
    // inflate on every launch.
    androidResources {
        noCompress += "jar"
    }
}

// games-activity 4.4.0 drags in the pre-1.8 split kotlin-stdlib-jdk7/jdk8 artifacts, which collide
// with appcompat's kotlin-stdlib (duplicate classes). Forcing them to 1.8.22 (empty stubs) fixes it.
configurations.all {
    resolutionStrategy {
        force(
            "org.jetbrains.kotlin:kotlin-stdlib-jdk7:1.8.22",
            "org.jetbrains.kotlin:kotlin-stdlib-jdk8:1.8.22",
        )
    }
}

if (gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }) {
    if (canSign) {
        logger.lifecycle("==> release signing: ${ksFile!!.absolutePath} (alias $ksAlias), versionCode $appVersionCode")
    } else {
        logger.warn(
            "==> WARNING: release build will be UNSIGNED and CANNOT BE INSTALLED.\n" +
                "    Set HEROZERO_KEYSTORE / _KEYSTORE_PASSWORD / _KEY_ALIAS / _KEY_PASSWORD, or\n" +
                "    herozero.keystore / .keystorePassword / .keyAlias / .keyPassword in\n" +
                "    %USERPROFILE%\\.gradle\\gradle.properties." +
                (if (ksPath != null && ksFile?.exists() != true) "\n    (configured keystore not found: $ksPath)" else "")
        )
    }
}

dependencies {
    // com.google.androidgamesdk.GameActivity, whose native half is inside libmain.so
    // (android-activity 0.6.1 bundles the matching GameActivity 4.4.0 C++).
    implementation("androidx.games:games-activity:4.4.0")
    // GameActivity extends AppCompatActivity -> appcompat on the classpath, AppCompat theme.
    implementation("androidx.appcompat:appcompat:1.7.0")
}

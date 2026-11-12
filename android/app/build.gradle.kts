import java.util.zip.ZipFile

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val sagaIrohCoreDir = rootProject.file("../saga-iroh-core")
val sagaIrohNdkLibs = sagaIrohCoreDir.resolve("target/ndk-libs")

/**
 * True when a release packaging task was requested.
 * Release builds ALWAYS require libsaga_iroh_core.so — cannot be relaxed by
 * SAGA_REQUIRE_NATIVE / -Psaga.requireNative (those flags only tighten debug).
 */
fun sagaIsReleaseRequested(): Boolean {
    return gradle.startParameter.taskNames.any { raw ->
        val t = raw.lowercase().substringAfterLast(':')
        t.contains("assemblerelease") ||
            t.contains("bundlerelease") ||
            t.contains("installrelease") ||
            t.contains("publishrelease") ||
            (t.contains("release") && (
                t.startsWith("assemble") ||
                    t.startsWith("bundle") ||
                    t.startsWith("install") ||
                    t.startsWith("package") ||
                    t.startsWith("publish")
                ))
    }
}

/** Debug may opt into fail-hard via CI / env / -P; release always fails hard. */
fun sagaRequireNative(): Boolean {
    if (sagaIsReleaseRequested()) return true
    if (System.getenv("CI") == "true") return true
    if (System.getenv("SAGA_REQUIRE_NATIVE") == "1") return true
    if (project.findProperty("saga.requireNative")?.toString() == "true") return true
    return false
}

android {
    namespace = "org.saga"
    compileSdk = 35
    // Pinned for CI / F-Droid. Keep in sync with .github/workflows/ci.yml (r27c).
    ndkVersion = "27.2.12479018"

    defaultConfig {
        applicationId = "org.saga"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        viewBinding = true
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs(sagaIrohNdkLibs)
        }
    }
}

tasks.register<Exec>("cargoNdkBuild") {
    group = "build"
    description = "Build saga-iroh-core native library for Android ABIs"
    workingDir = sagaIrohCoreDir
    commandLine(
        "cargo", "ndk",
        "-o", "target/ndk-libs",
        "-t", "arm64-v8a",
        "-t", "x86_64",
        "build", "--release", "--features", "iroh-transport,mock-token"
    )
    val requireNative = sagaRequireNative()
    isIgnoreExitValue = !requireNative
    doLast {
        val exit = executionResult.get().exitValue
        val abis = listOf("arm64-v8a", "x86_64")
        val missing = abis.filter { abi ->
            !sagaIrohNdkLibs.resolve("$abi/libsaga_iroh_core.so").isFile
        }
        if (exit != 0 || missing.isNotEmpty()) {
            val detail = buildString {
                if (exit != 0) append("cargo-ndk exit=$exit. ")
                if (missing.isNotEmpty()) {
                    append("Missing libsaga_iroh_core.so for: ${missing.joinToString()}. ")
                }
            }
            if (requireNative) {
                throw GradleException(
                    "Native saga-iroh-core build required but failed. $detail" +
                        "Install cargo-ndk and the Android NDK (ndkVersion ${android.ndkVersion}). " +
                        "See saga-iroh-core/README.md"
                )
            }
            logger.error(
                "WARNING: cargo ndk build failed or cargo-ndk not installed; " +
                    "APK will use Kotlin StubIrohCallSession fallback. $detail" +
                    "This is allowed only for local debug builds. " +
                    "CI and release builds must set SAGA_REQUIRE_NATIVE=1 / -Psaga.requireNative=true. " +
                    "See saga-iroh-core/README.md"
            )
        }
    }
}

tasks.register("verifyNativeLibsInApk") {
    group = "verification"
    description = "Fail if libsaga_iroh_core.so is missing from the debug APK"
    dependsOn("assembleDebug")
    doLast {
        val apk = layout.buildDirectory
            .file("outputs/apk/debug/app-debug.apk")
            .get()
            .asFile
        if (!apk.isFile) {
            throw GradleException("Debug APK not found at ${apk.absolutePath}")
        }
        ZipFile(apk).use { zip ->
            for (abi in listOf("arm64-v8a", "x86_64")) {
                val entry = "lib/$abi/libsaga_iroh_core.so"
                if (zip.getEntry(entry) == null) {
                    throw GradleException(
                        "APK missing $entry — native build did not package libsaga_iroh_core.so"
                    )
                }
            }
        }
        logger.lifecycle("OK: libsaga_iroh_core.so present for arm64-v8a and x86_64 in ${apk.name}")
    }
}

tasks.named("preBuild") {
    dependsOn("cargoNdkBuild")
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.constraintlayout:constraintlayout:2.2.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.activity:activity-ktx:1.9.3")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.5.2")
    androidTestImplementation("androidx.test:rules:1.5.0")
}

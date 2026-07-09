plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val sagaIrohCoreDir = rootProject.file("../saga-iroh-core")
val sagaIrohNdkLibs = sagaIrohCoreDir.resolve("target/ndk-libs")

android {
    namespace = "org.saga"
    compileSdk = 35

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
    isIgnoreExitValue = true
    doLast {
        if (executionResult.get().exitValue != 0) {
            logger.warn(
                "cargo ndk build failed or cargo-ndk not installed; " +
                    "APK will use Kotlin StubIrohCallSession fallback. " +
                    "See saga-iroh-core/README.md"
            )
        }
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

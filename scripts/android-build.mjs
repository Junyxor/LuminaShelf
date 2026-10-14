import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { prepareAndroid, root, run, tauriCli } from "./android-prepare.mjs";

export const architectures = {
  aarch64: { abi: "arm64-v8a", arch: "arm64", triple: "aarch64-linux-android", clang: "aarch64-linux-android" },
  armv7: { abi: "armeabi-v7a", arch: "arm", triple: "armv7-linux-androideabi", clang: "armv7a-linux-androideabi" },
  x86_64: { abi: "x86_64", arch: "x86_64", triple: "x86_64-linux-android", clang: "x86_64-linux-android" },
  i686: { abi: "x86", arch: "x86", triple: "i686-linux-android", clang: "i686-linux-android" },
};

export function buildAndroid(targets, debug = false) {
  process.env.CARGO_BUILD_JOBS ||= "2";
  if (!targets.length || targets.some((target) => !architectures[target])) throw new Error("Invalid Android targets.");
  const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
  if (!sdk || !process.env.JAVA_HOME) throw new Error("Set ANDROID_HOME and JAVA_HOME before building.");
  const android = resolve(root, "src-tauri/gen/android");
  if (process.platform === "win32" && !process.env.CARGO_TARGET_DIR) {
    const id = createHash("sha256").update(root).digest("hex").slice(0, 12);
    process.env.CARGO_TARGET_DIR = join(tmpdir(), `luminashelf-${id}`, "target");
  }
  if (process.platform === "win32" && /[^\x20-\x7e]/.test(process.env.CARGO_TARGET_DIR || "")) {
    throw new Error("Windows NDK requires an ASCII CARGO_TARGET_DIR (for example D:/tmp/luminashelf-target).");
  }
  prepareAndroid();

  if (process.platform !== "win32") {
    run(process.execPath, [tauriCli, "android", "build", "--ci", "--apk", ...(debug ? ["--debug"] : []), "--target", ...targets]);
  } else {
    // Build and copy JNI libraries directly so Windows Developer Mode is optional.
    const ndks = readdirSync(join(sdk, "ndk")).filter((name) => /^\d/.test(name));
    ndks.sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    const ndk = process.env.NDK_HOME || join(sdk, "ndk", ndks.at(-1));
    const bin = join(ndk, "toolchains/llvm/prebuilt/windows-x86_64/bin");
    const nativeEnv = {
      ...process.env, NDK_HOME: ndk, ANDROID_NDK_HOME: ndk,
      TAURI_ANDROID_PROJECT_PATH: android,
      WRY_ANDROID_PACKAGE: "app.luminashelf.client",
      WRY_ANDROID_LIBRARY: "lumina_shelf_desktop",
      WRY_ANDROID_KOTLIN_FILES_OUT_DIR: join(android, "app/src/main/java/app/luminashelf/client/generated"),
    };
    mkdirSync(nativeEnv.WRY_ANDROID_KOTLIN_FILES_OUT_DIR, { recursive: true });
    run(process.execPath, [resolve(root, "node_modules/typescript/bin/tsc"), "--noEmit"]);
    run(process.execPath, [resolve(root, "node_modules/vite/bin/vite.js"), "build"]);
    for (const target of targets) {
      const { abi, triple, clang } = architectures[target];
      const cargoTarget = triple.toUpperCase().replaceAll("-", "_");
      nativeEnv[`CARGO_TARGET_${cargoTarget}_LINKER`] = join(bin, `${clang}24-clang.cmd`);
      nativeEnv[`CC_${triple.replaceAll("-", "_")}`] = join(bin, `${clang}24-clang.cmd`);
      nativeEnv[`CXX_${triple.replaceAll("-", "_")}`] = join(bin, `${clang}24-clang++.cmd`);
      nativeEnv[`AR_${triple.replaceAll("-", "_")}`] = join(bin, "llvm-ar.exe");
      run("cargo", [
        "build", "--locked", "--package", "lumina-shelf-desktop", "--lib", "--target", triple,
        "--features", "tauri/custom-protocol", ...(debug ? [] : ["--release"]),
      ], { env: nativeEnv });
      const destination = join(android, "app/src/main/jniLibs", abi);
      mkdirSync(destination, { recursive: true });
      copyFileSync(join(process.env.CARGO_TARGET_DIR, triple, debug ? "debug" : "release", "liblumina_shelf_desktop.so"), join(destination, "liblumina_shelf_desktop.so"));
    }
    const { version } = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
    const [major, minor, patch] = version.split(".").map(Number);
    writeFileSync(join(android, "app/tauri.properties"), `tauri.android.versionCode=${major * 1000000 + minor * 1000 + patch}\ntauri.android.versionName=${version}\n`);
    const profile = debug ? "Debug" : "Release";
    const gradleArgs = [
      "-classpath", join(android, "gradle/wrapper/gradle-wrapper.jar"), "org.gradle.wrapper.GradleWrapperMain",
      `:app:assembleUniversal${profile}`, `-PabiList=${targets.map((target) => architectures[target].abi).join(",")}`,
      `-ParchList=${targets.map((target) => architectures[target].arch).join(",")}`, `-PtargetList=${targets.join(",")}`,
      ...targets.flatMap((target) => {
        const arch = architectures[target].arch;
        return ["-x", `:app:rustBuild${arch[0].toUpperCase()}${arch.slice(1)}${profile}`];
      }), "--console=plain", "--max-workers=2",
    ];
    run(join(process.env.JAVA_HOME, "bin/java.exe"), gradleArgs, { cwd: android });
  }
  return join(android, "app/build/outputs/apk/universal", debug ? "debug/app-universal-debug.apk" : "release/app-universal-release-unsigned.apk");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const debug = args.includes("--debug");
  const targets = args.filter((arg) => arg !== "--debug");
  console.log(`APK: ${buildAndroid(targets.length ? targets : ["aarch64"], debug)}`);
}

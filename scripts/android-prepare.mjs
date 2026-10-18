import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = fileURLToPath(new URL("../", import.meta.url));
export const tauriCli = resolve(root, "node_modules/@tauri-apps/cli/tauri.js");

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`);
}

export function prepareAndroid() {
  const android = resolve(root, "src-tauri/gen/android");
  if (!existsSync(resolve(android, "app/build.gradle.kts"))) {
    run(process.execPath, [tauriCli, "android", "init", "--ci"]);
  }
  const manifest = resolve(android, "app/src/main/AndroidManifest.xml");
  let xml = readFileSync(manifest, "utf8");
  if (!xml.includes("android.permission.ACCESS_NETWORK_STATE")) {
    xml = xml.replace(
      '<uses-permission android:name="android.permission.INTERNET" />',
      '<uses-permission android:name="android.permission.INTERNET" />\n    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />',
    );
    writeFileSync(manifest, xml);
  }
  for (const permission of ["FOREGROUND_SERVICE", "FOREGROUND_SERVICE_DATA_SYNC", "WAKE_LOCK"]) {
    if (!xml.includes(`android.permission.${permission}"`)) {
      xml = xml.replace("<application", `<uses-permission android:name="android.permission.${permission}" />\n    <application`);
    }
  }
  if (!xml.includes('android:name=".DownloadService"')) {
    xml = xml.replace("</application>", '<service android:name=".DownloadService" android:exported="false" android:foregroundServiceType="dataSync" android:stopWithTask="false" />\n    </application>');
  }
  writeFileSync(manifest, xml);
  const properties = resolve(android, "gradle.properties");
  let content = readFileSync(properties, "utf8");
  if (!content.includes("android.overridePathCheck=")) {
    content += "\n# Support Windows checkouts with non-ASCII directory names.\nandroid.overridePathCheck=true\n";
  }
  if (!content.includes("org.gradle.workers.max=")) content += "\norg.gradle.workers.max=2\n";
  if (process.platform === "win32" && !content.includes("kotlin.incremental=")) {
    content += "\n# Kotlin caches cannot relativize Cargo sources across Windows drives.\nkotlin.incremental=false\nkotlin.compiler.execution.strategy=in-process\n";
  }
  writeFileSync(properties, content);

  // Tauri's Android runtime uses Kotlin's compilerOptions DSL (Kotlin 2+).
  const gradle = resolve(android, "build.gradle.kts");
  const buildScript = readFileSync(gradle, "utf8").replace(
    /org\.jetbrains\.kotlin:kotlin-gradle-plugin:[\d.]+/,
    "org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10",
  );
  writeFileSync(gradle, buildScript);

  // Compress native libraries in APK downloads; Android extracts them when installing.
  const appGradle = resolve(android, "app/build.gradle.kts");
  let appBuildScript = readFileSync(appGradle, "utf8");
  if (!appBuildScript.includes("jniLibs.useLegacyPackaging = true")) {
    const anchor = "    buildFeatures {";
    if (appBuildScript.split(anchor).length !== 2) {
      throw new Error("Unknown Android Gradle template: cannot configure compressed native libraries.");
    }
    appBuildScript = appBuildScript.replace(anchor, `    packaging {\n        jniLibs.useLegacyPackaging = true\n    }\n${anchor}`);
    writeFileSync(appGradle, appBuildScript);
  }

  const icons = resolve(root, "src-tauri/icons/android");
  if (existsSync(icons)) cpSync(icons, resolve(android, "app/src/main/res"), { recursive: true });
  cpSync(resolve(root, "src-tauri/android/MainActivity.kt"), resolve(android, "app/src/main/java/app/luminashelf/client/MainActivity.kt"));
  for (const source of ["DownloadPlugin.kt", "DownloadService.kt"]) {
    cpSync(resolve(root, "src-tauri/android", source), resolve(android, "app/src/main/java/app/luminashelf/client", source));
  }
  mkdirSync(resolve(android, "app/src/main/res/drawable"), { recursive: true });
  cpSync(resolve(root, "src-tauri/android/lumina_download.xml"), resolve(android, "app/src/main/res/drawable/lumina_download.xml"));
  cpSync(resolve(root, "src-tauri/android/file_paths.xml"), resolve(android, "app/src/main/res/xml/file_paths.xml"));
  for (const resource of ["values", "values-night"]) {
    const directory = resolve(android, "app/src/main/res", resource);
    mkdirSync(directory, { recursive: true });
    cpSync(resolve(root, "src-tauri/android/themes.xml"), resolve(directory, "themes.xml"));
  }
  const assets = resolve(android, "app/src/main/assets");
  mkdirSync(assets, { recursive: true });
  cpSync(resolve(root, "OPEN_SOURCE_NOTICES.md"), resolve(assets, "open-source-notices.md"));
  console.log("Android project prepared (API 36, light theme, system insets, back navigation).");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  prepareAndroid();
}

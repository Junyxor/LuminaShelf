import { spawnSync } from "node:child_process";
import { cpSync, existsSync, readFileSync, writeFileSync } from "node:fs";
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
  const properties = resolve(android, "gradle.properties");
  let content = readFileSync(properties, "utf8");
  if (!content.includes("android.overridePathCheck=")) {
    content += "\n# Support Windows checkouts with non-ASCII directory names.\nandroid.overridePathCheck=true\n";
  }
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

  const icons = resolve(root, "src-tauri/icons/android");
  if (existsSync(icons)) cpSync(icons, resolve(android, "app/src/main/res"), { recursive: true });
  console.log("Android project prepared (API 36, app icons, network state permission).");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  prepareAndroid();
}

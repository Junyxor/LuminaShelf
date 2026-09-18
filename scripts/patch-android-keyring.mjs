import { promises as fs } from "node:fs";
import path from "node:path";

const javaRoot = path.resolve("src-tauri/gen/android/app/src/main/java");

async function findMainActivity(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      const found = await findMainActivity(target);
      if (found) return found;
    } else if (entry.name === "MainActivity.kt") {
      return target;
    }
  }
  return null;
}

const activityPath = await findMainActivity(javaRoot);
if (!activityPath) {
  throw new Error("Tauri Android MainActivity.kt was not found. Run `tauri android init` first.");
}

let source = await fs.readFile(activityPath, "utf8");
const packageMatch = source.match(/^package\s+([\w.]+)/m);
if (!packageMatch) throw new Error("Could not determine MainActivity package");
if (packageMatch[1] !== "app.luminashelf.client") {
  throw new Error(`Unexpected Android package ${packageMatch[1]}; update the Rust JNI symbol before changing the Tauri identifier.`);
}

if (!source.includes("import android.content.Context")) {
  if (source.includes("import android.os.Bundle")) {
    source = source.replace("import android.os.Bundle", "import android.content.Context\nimport android.os.Bundle");
  } else {
    source = source.replace(packageMatch[0], `${packageMatch[0]}\n\nimport android.content.Context`);
  }
}

const classAnchor = "class MainActivity : TauriActivity() {";
if (!source.includes(classAnchor)) {
  throw new Error("MainActivity shape is not recognized; refusing to patch it automatically.");
}

if (!source.includes("private external fun initNdkContext")) {
  source = source.replace(
    classAnchor,
    `${classAnchor}\n  private external fun initNdkContext(context: Context)`,
  );
}

if (!source.includes("initNdkContext(this.applicationContext)")) {
  const superCall = "    super.onCreate(savedInstanceState)";
  if (source.includes(superCall)) {
    source = source.replace(
      superCall,
      `${superCall}\n    initNdkContext(this.applicationContext)`,
    );
  } else {
    const declaration = "  private external fun initNdkContext(context: Context)";
    source = source.replace(
      declaration,
      `${declaration}\n\n  override fun onCreate(savedInstanceState: Bundle?) {\n    super.onCreate(savedInstanceState)\n    initNdkContext(this.applicationContext)\n  }`,
    );
  }
}

await fs.writeFile(activityPath, source);
for (const name of ["DownloadService.kt", "DownloadRuntimePlugin.kt"]) {
  await fs.copyFile(path.resolve("android", name), path.join(path.dirname(activityPath), name));
}
const manifestPath = path.resolve("src-tauri/gen/android/app/src/main/AndroidManifest.xml");
let manifest = await fs.readFile(manifestPath, "utf8");
for (const permission of ["FOREGROUND_SERVICE", "FOREGROUND_SERVICE_DATA_SYNC", "POST_NOTIFICATIONS", "WAKE_LOCK"]) {
  if (!manifest.includes(`android.permission.${permission}"`)) {
    manifest = manifest.replace("<application", `<uses-permission android:name="android.permission.${permission}" />\n    <application`);
  }
}
if (!manifest.includes('android:name=".DownloadService"')) {
  manifest = manifest.replace("</application>", '<service android:name=".DownloadService" android:exported="false" android:foregroundServiceType="dataSync" android:stopWithTask="false" />\n    </application>');
}
await fs.writeFile(manifestPath, manifest);
await fs.copyFile(path.resolve("android/proguard-lumina.pro"), path.resolve("src-tauri/gen/android/app/proguard-lumina.pro"));
// These are regenerated build inputs, kept in sync with the application version.
const appConfig = JSON.parse(await fs.readFile("src-tauri/tauri.conf.json", "utf8"));
const [major, minor, patch] = appConfig.version.split(".").map(Number);
await fs.writeFile("src-tauri/gen/android/app/tauri.properties",
  `tauri.android.versionName=${appConfig.version}\ntauri.android.versionCode=${major * 1000000 + minor * 1000 + patch}\n`);
console.log(`Android secure-storage activity patch ready: ${path.relative(process.cwd(), activityPath)}`);

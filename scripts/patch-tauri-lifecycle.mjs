import { promises as fs } from "node:fs";
import path from "node:path";
import os from "node:os";

// Patch a generated copy, never the shared Cargo registry or upstream package.
// The pinned Tauri 2 Android PluginManager assumes the process dies with its
// first activity. A foreground download requires rebinding after activity loss.
const project = path.resolve("src-tauri/gen/android");
const cargoRoot = process.env.CARGO_HOME || path.join(os.homedir(), ".cargo");
const registryRoot = path.join(cargoRoot, "registry/src");
const lock = await fs.readFile("Cargo.lock", "utf8");
const version = lock.match(/\[\[package\]\]\s+name = "tauri"\s+version = "([^"]+)"/)?.[1];
if (!version) throw new Error("Cannot locate the locked Tauri version.");
let source;
for (const registry of await fs.readdir(registryRoot)) {
  const candidate = path.join(registryRoot, registry, "tauri-" + version, "mobile/android");
  if (await fs.stat(candidate).then((value) => value.isDirectory()).catch(() => false)) { source = candidate; break; }
}
if (!source) throw new Error("Fetch the Tauri Cargo dependency before Android initialization.");
const destination = path.join(project, "lumina-tauri-android");
await fs.mkdir(destination, { recursive: true });
await fs.cp(path.join(source, "src"), path.join(destination, "src"), { recursive: true });
for (const name of ["build.gradle.kts", "proguard-rules.pro"]) await fs.copyFile(path.join(source, name), path.join(destination, name));
const managerFile = path.join(destination, "src/main/java/app/tauri/plugin/PluginManager.kt");
let manager = (await fs.readFile(managerFile, "utf8")).replaceAll("\r\n", "\n");
const before = `    // TODO: on destroy, we should change to a different activity
    if (::activity.isInitialized) {
      return
    }
    this.activity = activity`;
if (!manager.includes(before)) throw new Error("Tauri lifecycle shape changed; update and review the activity rebind patch.");
manager = manager.replace(before, `    if (::activity.isInitialized && this.activity === activity) return
    val previousPlugins = plugins.values.toList()
    plugins.clear()
    requestPermissionsCallback = null
    startActivityForResultCallback = null
    startIntentSenderForResultCallback = null
    this.activity = activity
    for (previous in previousPlugins) {
      val instance = previous.instance.javaClass.getConstructor(android.app.Activity::class.java).newInstance(activity)
      plugins[previous.name] = PluginHandle(this, previous.name, instance, previous.config, jsonMapper)
    }`);
await fs.writeFile(managerFile, manager);
const settingsFile = path.join(project, "settings.gradle");
let settings = await fs.readFile(settingsFile, "utf8");
const override = "project(':tauri-android').projectDir = new File(rootDir, 'lumina-tauri-android')";
if (!settings.includes(override)) await fs.writeFile(settingsFile, settings.trimEnd() + "\n\n" + override + "\n");

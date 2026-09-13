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

if (!source.includes("initNdkContext")) {
  const packageLine = packageMatch[0];
  if (!source.includes("import android.os.Bundle")) {
    source = source.replace(packageLine, `${packageLine}\n\nimport android.content.Context\nimport android.os.Bundle`);
  } else if (!source.includes("import android.content.Context")) {
    source = source.replace("import android.os.Bundle", "import android.content.Context\nimport android.os.Bundle");
  }

  const classPattern = /class\s+MainActivity\s*:\s*TauriActivity\(\)\s*\{?[\s\S]*?\}?\s*$/m;
  const simplePattern = /class\s+MainActivity\s*:\s*TauriActivity\(\)\s*$/m;
  const replacement = `class MainActivity : TauriActivity() {
  private external fun initNdkContext(context: Context)

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    initNdkContext(this.applicationContext)
  }
}`;

  if (simplePattern.test(source)) {
    source = source.replace(simplePattern, replacement);
  } else if (classPattern.test(source)) {
    source = source.replace(classPattern, replacement);
  } else {
    throw new Error("MainActivity shape is not recognized; refusing to patch it automatically.");
  }

  await fs.writeFile(activityPath, source);
}

console.log(`Android secure-storage activity patch ready: ${path.relative(process.cwd(), activityPath)}`);

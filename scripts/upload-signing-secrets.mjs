import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";
const repository = "Junyxor/LuminaShelf";
const credentials = JSON.parse(readFileSync(resolve(".signing/release.json"), "utf8"));
const entries = [
  ["LUMINASHELF_ANDROID_KEYSTORE", readFileSync(resolve(".signing/luminashelf-release.p12")).toString("base64")],
  ["LUMINASHELF_ANDROID_STORE_PASSWORD", credentials.storePassword],
  ["LUMINASHELF_ANDROID_KEY_ALIAS", credentials.alias],
];
for (const [name, value] of entries) {
  const result = spawnSync("gh", ["secret", "set", name, "--repo", repository, "--app", "actions"], {
    input: value, encoding: "utf8", windowsHide: true,
    env: { ...process.env, GH_PROMPT_DISABLED: "1", GIT_TERMINAL_PROMPT: "0", GCM_INTERACTIVE: "Never" },
  });
  if (result.error || result.status !== 0) throw new Error(`Could not store repository secret ${name}; check repository administration access.`);
  console.log(`Stored ${name} in ${repository} Actions secrets.`);
}

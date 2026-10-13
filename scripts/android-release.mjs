import { createHash, randomBytes } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { root, run } from "./android-prepare.mjs";
import { buildAndroid } from "./android-build.mjs";

const sdk = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
const javaHome = process.env.JAVA_HOME;
if (!sdk || !javaHome) throw new Error("Set ANDROID_HOME and JAVA_HOME to the Android SDK and JDK 17 directories.");
const executable = (name) => process.platform === "win32" ? `${name}.exe` : name;
const java = join(javaHome, "bin", executable("java"));
const keytool = join(javaHome, "bin", executable("keytool"));
const builds = readdirSync(join(sdk, "build-tools")).filter((version) => /^\d+\.\d+\.\d+$/.test(version));
builds.sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
const tools = join(sdk, "build-tools", builds.at(-1) || "36.0.0");
const apksigner = join(tools, "lib/apksigner.jar");
const zipalign = join(tools, executable("zipalign"));
for (const file of [java, keytool, apksigner, zipalign]) {
  if (!existsSync(file)) throw new Error(`Required build tool is missing: ${file}`);
}

const targets = process.argv.slice(2);
if (targets.length === 0) targets.push("aarch64", "armv7", "x86_64");
const abis = { aarch64: "arm64-v8a", armv7: "armeabi-v7a", x86_64: "x86_64", i686: "x86" };
if (new Set(targets).size !== targets.length || targets.some((target) => !abis[target])) {
  throw new Error("Targets must be unique values from: aarch64 armv7 x86_64 i686");
}

const signingDir = resolve(root, ".signing");
mkdirSync(signingDir, { recursive: true, mode: 0o700 });
if (process.platform === "win32") {
  const account = `${process.env.USERDOMAIN}\\${process.env.USERNAME}`;
  run("icacls", [signingDir, "/inheritance:r", "/grant:r", `${account}:(OI)(CI)F`]);
}
const keystore = join(signingDir, "luminashelf-release.p12");
const credentialsFile = join(signingDir, "release.json");
if (existsSync(keystore) !== existsSync(credentialsFile)) {
  throw new Error("The release keystore and its credentials must be backed up/restored together.");
}
const credentials = existsSync(credentialsFile)
  ? JSON.parse(readFileSync(credentialsFile, "utf8"))
  : { alias: "luminashelf", storePassword: randomBytes(32).toString("base64url") };
const signingEnv = { ...process.env, LUMINASHELF_STORE_PASSWORD: credentials.storePassword };
if (!existsSync(keystore)) {
  writeFileSync(credentialsFile, `${JSON.stringify(credentials, null, 2)}\n`, { mode: 0o600 });
  run(keytool, [
    "-genkeypair", "-keystore", keystore, "-storetype", "PKCS12",
    "-alias", credentials.alias, "-keyalg", "RSA", "-keysize", "3072",
    "-sigalg", "SHA256withRSA", "-validity", "10000",
    "-dname", "CN=LuminaShelf Release, OU=Android, O=LuminaShelf",
    "-storepass:env", "LUMINASHELF_STORE_PASSWORD",
    "-keypass:env", "LUMINASHELF_STORE_PASSWORD", "-noprompt",
  ], { env: signingEnv });
}

const unsigned = buildAndroid(targets);
if (!existsSync(unsigned)) throw new Error(`Release APK is missing: ${unsigned}`);

const { version } = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
const output = resolve(root, "artifacts/android");
mkdirSync(output, { recursive: true });
const flavor = targets.length === 1 ? abis[targets[0]] : "universal";
const aligned = join(output, `LuminaShelf-${version}-${flavor}-aligned.apk`);
const signed = join(output, `LuminaShelf-${version}-${flavor}-release.apk`);
run(zipalign, ["-P", "16", "-f", "4", unsigned, aligned]);
run(java, [
  "-jar", apksigner, "sign", "--ks", keystore, "--ks-key-alias", credentials.alias,
  "--ks-pass", "env:LUMINASHELF_STORE_PASSWORD", "--key-pass", "env:LUMINASHELF_STORE_PASSWORD",
  "--out", signed, aligned,
], { env: signingEnv });
run(java, ["-jar", apksigner, "verify", "--verbose", "--print-certs", signed]);
run(zipalign, ["-c", "-P", "16", "4", signed]);
const sha256 = createHash("sha256").update(readFileSync(signed)).digest("hex");
writeFileSync(`${signed}.sha256`, `${sha256}  ${signed.split(/[\\/]/).at(-1)}\n`);
run(keytool, [
  "-exportcert", "-rfc", "-keystore", keystore, "-alias", credentials.alias,
  "-storepass:env", "LUMINASHELF_STORE_PASSWORD", "-file", join(output, "release-certificate.pem"),
], { env: signingEnv });
const commit = spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" });
const changes = spawnSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" });
writeFileSync(join(output, `${flavor}-build-info.json`), `${JSON.stringify({
  applicationId: "app.luminashelf.client", version, minSdk: 24, targetSdk: 36,
  abis: targets.map((target) => abis[target]), sha256, bytes: statSync(signed).size,
  sourceCommit: commit.stdout?.trim(), sourceDirty: Boolean(changes.stdout?.trim()),
  builtAt: new Date().toISOString(),
}, null, 2)}\n`);
console.log(`Signed APK: ${signed}\nSHA-256: ${sha256}\nSigning backup: ${signingDir}`);

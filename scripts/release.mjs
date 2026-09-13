/**
 * Builds, signs and publishes a release, from this machine.
 *
 * The updater refuses any package it cannot verify against the public key compiled into the
 * application, so a release has to be signed by whoever holds the private key. That leaves
 * two places to do it: a CI runner, which means handing the key to GitHub as a secret, or
 * here — where the key already lives and never has to leave.
 *
 * This is the second option. It produces the three files a release needs and uploads them:
 *
 *   Oracle_<version>_x64-setup.exe      the installer
 *   Oracle_<version>_x64-setup.exe.sig  its signature
 *   latest.json                         what the updater reads
 *
 * Usage:
 *
 *   npm run release
 *   npm run release -- --notes "What changed in this one"
 *   npm run release -- --dry-run
 *
 * The key is handed over as a path, not as its contents, so it never passes through this
 * script, an argument list, or a shell history.
 */

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(import.meta.url), "..", "..");
/** Where the signing key lives. `ORACLE_SIGNING_KEY` overrides it; this script's own name
 * for it, to avoid suggesting the Tauri CLI reads that variable. */
const KEY = process.env.ORACLE_SIGNING_KEY ?? join(homedir(), ".tauri", "oracle.key");

/** The platform key the updater looks itself up under. */
const PLATFORM = "windows-x86_64";

const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run");
const notes = valueOf("--notes") ?? "Installer below. An existing installation updates itself from this release.";

function valueOf(flag) {
  const at = args.indexOf(flag);
  return at === -1 ? undefined : args[at + 1];
}

function fail(message) {
  console.error(`\n  ${message}\n`);
  process.exit(1);
}

/**
 * Runs a command, letting its output through, and stops the release if it fails.
 *
 * `shell` is opt-in, and only `npm` needs it: it is a `.cmd` on Windows and cannot be
 * spawned without one. Everywhere else it must stay off, because a shell flattens the
 * argument array into a string with no quoting — which turned `--title Oracle 0.1.1` into
 * three arguments and left `gh` looking for a file called 0.1.1.
 */
function run(command, commandArgs, { env = {}, shell = false } = {}) {
  console.log(`\n$ ${command} ${commandArgs.join(" ")}`);
  execFileSync(command, commandArgs, {
    cwd: ROOT,
    stdio: "inherit",
    env: { ...process.env, ...env },
    shell,
  });
}

/** True when the command succeeds, with its output swallowed. For asking questions. */
function succeeds(command, commandArgs) {
  try {
    execFileSync(command, commandArgs, { cwd: ROOT, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------

const config = JSON.parse(readFileSync(join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"));
const version = config.version;
const tag = `v${version}`;

if (!config.plugins?.updater?.pubkey) {
  fail("tauri.conf.json has no updater public key, so nothing could verify this release.");
}

if (!existsSync(KEY)) {
  fail(
    `No signing key at ${KEY}.\n` +
      "  Generate one with `npm run tauri signer generate -- -w <path>`, or point\n" +
      "  ORACLE_SIGNING_KEY at an existing key.",
  );
}

// A published release must never be overwritten: that is how an installation ends up offered
// a build it already has, under a version it does not.
if (succeeds("gh", ["release", "view", tag])) {
  fail(
    `Oracle ${version} is already published. Bump "version" in src-tauri/tauri.conf.json\n` +
      "  (and package.json, and src-tauri/Cargo.toml) before releasing again.",
  );
}

// A tag with no release behind it is a release that stopped part way through, so it is
// resumed rather than refused. The alternative is asking someone to delete a tag by hand
// before they can retry, which is a poor answer to "the last step failed".
const tagged =
  execFileSync("git", ["tag", "--list", tag], { cwd: ROOT }).toString().trim() !== "";

console.log(`\nReleasing Oracle ${version}\n  key: ${KEY}`);
if (tagged) {
  console.log(`  ${tag} is already tagged: resuming an interrupted release.`);
}

run("npm", ["run", "tauri", "build"], {
  shell: true,
  env: {
    // The variable takes either the key's contents or a path to it, and a path is what is
    // wanted: nothing then reads the key but the bundler. Note that
    // TAURI_SIGNING_PRIVATE_KEY_PATH, which the key generator advertises, is not read by the
    // build at all — passing the path there produces "no private key found".
    TAURI_SIGNING_PRIVATE_KEY: KEY,
    // Set explicitly: an unset variable makes the bundler prompt, which hangs a script.
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  },
});

const bundle = join(ROOT, "src-tauri", "target", "release", "bundle", "nsis");
const installer = join(bundle, `Oracle_${version}_x64-setup.exe`);
const signature = `${installer}.sig`;

for (const path of [installer, signature]) {
  if (!existsSync(path)) fail(`The build did not produce ${path}.`);
}

// The manifest the updater fetches. `url` has to be the final download address, which for a
// GitHub release is predictable from the tag and the file name.
const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    [PLATFORM]: {
      signature: readFileSync(signature, "utf8").trim(),
      url: `https://github.com/Alicienn/oracle/releases/download/${tag}/Oracle_${version}_x64-setup.exe`,
    },
  },
};

const manifestPath = join(bundle, "latest.json");
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`\nWrote ${manifestPath}`);

if (dryRun) {
  console.log("\n--dry-run: nothing was tagged, pushed or published.\n");
  console.log(readFileSync(manifestPath, "utf8"));
  process.exit(0);
}

// Tag from the commit being released, so the release and the source agree.
if (!tagged) {
  run("git", ["tag", tag]);
}
// Pushed every time: the tag may exist locally from an attempt that stopped before this.
run("git", ["push", "origin", tag]);

run("gh", [
  "release",
  "create",
  tag,
  installer,
  manifestPath,
  "--title",
  `Oracle ${version}`,
  "--notes",
  notes,
]);

console.log(`\nOracle ${version} published. Installations on an older build will offer it.\n`);

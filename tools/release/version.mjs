#!/usr/bin/env node
// Release version tooling. Aeria has one version, declared in the workspace
// Cargo.toml and mirrored by both package.json files. See
// docs/development/releases.md.
//
//   node tools/release/version.mjs check [--tag vX.Y.Z]
//       Prints the version; fails when the declarations disagree or when the
//       tag does not name exactly that version.
//   node tools/release/version.mjs nightly <build-number>
//       Writes and prints the nightly version <next>-nightly.<build-number>.
//   node tools/release/version.mjs feed --version V --url URL --signature FILE [--notes TEXT]
//       Prints the updater feed (latest.json) for one Windows installer.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const cargoToml = `${root}Cargo.toml`;
const packageJsons = [`${root}package.json`, `${root}apps/desktop/package.json`];

const workspaceVersion = /(\[workspace\.package\][^[]*?\nversion\s*=\s*")([^"]+)(")/;
const releaseVersion = /^(\d+)\.(\d+)\.(\d+)$/;

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function readVersions() {
  const cargo = readFileSync(cargoToml, "utf8").match(workspaceVersion);
  if (!cargo) fail("Cargo.toml has no [workspace.package] version");
  const versions = [["Cargo.toml", cargo[2]]];
  for (const path of packageJsons) {
    versions.push([path.slice(root.length), JSON.parse(readFileSync(path, "utf8")).version]);
  }
  return versions;
}

function declaredVersion() {
  const versions = readVersions();
  const [, version] = versions[0];
  const mismatched = versions.filter(([, candidate]) => candidate !== version);
  if (mismatched.length > 0) {
    fail(`versions disagree: ${versions.map(([file, candidate]) => `${file}=${candidate}`).join(", ")}`);
  }
  if (!releaseVersion.test(version)) fail(`the declared version ${version} must be MAJOR.MINOR.PATCH`);
  return version;
}

function writeVersion(version) {
  const cargo = readFileSync(cargoToml, "utf8");
  writeFileSync(cargoToml, cargo.replace(workspaceVersion, `$1${version}$3`));
  for (const path of packageJsons) {
    const text = readFileSync(path, "utf8");
    writeFileSync(path, text.replace(/("version"\s*:\s*")[^"]+(")/, `$1${version}$2`));
  }
}

function tagExists(tag) {
  return execFileSync("git", ["tag", "--list", tag], { cwd: root, encoding: "utf8" }).trim() === tag;
}

/**
 * Nightly builds precede the next stable release: before `vX.Y.Z` is tagged
 * they are `X.Y.Z-nightly.N`, afterwards `X.Y.(Z+1)-nightly.N`.
 */
export function nightlyVersion(version, released, build) {
  const [, major, minor, patch] = version.match(releaseVersion);
  const base = released ? `${major}.${minor}.${Number(patch) + 1}` : version;
  return `${base}-nightly.${build}`;
}

function option(args, name) {
  const index = args.indexOf(name);
  if (index === -1) return undefined;
  const value = args[index + 1];
  if (value === undefined || value.startsWith("--")) fail(`${name} needs a value`);
  return value;
}

function main([command, ...args]) {
  if (command === "check") {
    const version = declaredVersion();
    const tag = option(args, "--tag");
    if (tag !== undefined && tag !== `v${version}`) fail(`tag ${tag} does not match the declared version ${version}`);
    console.log(version);
  } else if (command === "nightly") {
    const build = args[0];
    if (!/^[1-9]\d*$/.test(build ?? "")) fail("nightly needs a positive build number");
    const version = declaredVersion();
    const nightly = nightlyVersion(version, tagExists(`v${version}`), build);
    writeVersion(nightly);
    console.log(nightly);
  } else if (command === "feed") {
    const version = option(args, "--version");
    const url = option(args, "--url");
    const signature = option(args, "--signature");
    if (!version || !url || !signature) fail("feed needs --version, --url, and --signature");
    console.log(JSON.stringify(updaterFeed({ version, url, signature: readFileSync(signature, "utf8"), notes: option(args, "--notes") ?? "" }), null, 2));
  } else {
    fail(`unknown command ${command ?? "(none)"}; expected check, nightly, or feed`);
  }
}

/** The Tauri updater feed for one Windows installer. */
export function updaterFeed({ version, url, signature, notes, date = new Date() }) {
  return {
    version,
    notes,
    pub_date: date.toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: {
      "windows-x86_64": { signature: signature.trim(), url },
    },
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}

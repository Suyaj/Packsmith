#!/usr/bin/env node
// 版本号单一来源：根目录 .env 的 APP_VERSION。
// 同步目标：package.json、package-lock.json、src-tauri/Cargo.toml、
// src-tauri/Cargo.lock（packsmith 包条目）。
// tauri.conf.json 不存版本号——Tauri 会回退读 Cargo.toml 的 version。
//
// 用法：
//   node scripts/sync-version.mjs                 从 .env 同步到各文件
//   node scripts/sync-version.mjs --check         只校验一致性，不改文件（CI 用）
//   node scripts/sync-version.mjs --from-package  以 package.json 为准回写 .env 并同步
//                                                 （npm version 生命周期钩子用，见 package.json "version" 脚本）
import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const FILES = {
  env: path.join(root, ".env"),
  pkg: path.join(root, "package.json"),
  pkgLock: path.join(root, "package-lock.json"),
  cargoToml: path.join(root, "src-tauri", "Cargo.toml"),
  cargoLock: path.join(root, "src-tauri", "Cargo.lock"),
};
// 发版提交时需要一并暂存的文件（npm version 只自动暂存 package.json/lock）
const GIT_ADD_TARGETS = [
  ".env",
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
];

const SEMVER = /^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/;

function readVersionFromEnv() {
  const match = readFileSync(FILES.env, "utf8").match(/^APP_VERSION=(.+)$/m);
  if (!match) throw new Error(".env 中未找到 APP_VERSION=... 行");
  return match[1].trim();
}

function readVersionFromPackageJson() {
  return JSON.parse(readFileSync(FILES.pkg, "utf8")).version;
}

function patchEnv(version) {
  const text = readFileSync(FILES.env, "utf8");
  const pattern = /^APP_VERSION=.+$/m;
  if (!pattern.test(text)) throw new Error(".env 中未找到 APP_VERSION 行");
  writeFileSync(FILES.env, text.replace(pattern, `APP_VERSION=${version}`));
}

function patchPackageJson(version) {
  const pkg = JSON.parse(readFileSync(FILES.pkg, "utf8"));
  pkg.version = version;
  writeFileSync(FILES.pkg, JSON.stringify(pkg, null, 2) + "\n");
}

function patchPackageLock(version) {
  // package-lock 有两处版本号：根级与 packages[""]
  const lock = JSON.parse(readFileSync(FILES.pkgLock, "utf8"));
  lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  writeFileSync(FILES.pkgLock, JSON.stringify(lock, null, 2) + "\n");
}

function patchCargoToml(version) {
  const text = readFileSync(FILES.cargoToml, "utf8");
  // 值已一致时替换结果与原文相同，不能靠文本比较判断是否匹配到
  const pattern = /(\[package\][\s\S]*?^version\s*=\s*)".*"/m;
  if (!pattern.test(text)) throw new Error("Cargo.toml [package] 中未找到 version 字段");
  writeFileSync(FILES.cargoToml, text.replace(pattern, `$1"${version}"`));
}

function patchCargoLock(version) {
  const text = readFileSync(FILES.cargoLock, "utf8");
  const pattern = /(name = "packsmith"\nversion\s*=\s*)"[^"]*"/;
  if (!pattern.test(text)) throw new Error("Cargo.lock 中未找到 packsmith 包条目");
  writeFileSync(FILES.cargoLock, text.replace(pattern, `$1"${version}"`));
}

function check() {
  const versions = {
    ".env": readVersionFromEnv(),
    "package.json": readVersionFromPackageJson(),
    "package-lock.json": JSON.parse(readFileSync(FILES.pkgLock, "utf8")).packages?.[""]
      ?.version,
    "src-tauri/Cargo.toml": readFileSync(FILES.cargoToml, "utf8").match(
      /\[package\][\s\S]*?^version\s*=\s*"(.*?)"/m,
    )?.[1],
    "src-tauri/Cargo.lock": readFileSync(FILES.cargoLock, "utf8").match(
      /name = "packsmith"\nversion\s*=\s*"(.*?)"/,
    )?.[1],
  };
  const expected = versions[".env"];
  const bad = Object.entries(versions).filter(([, v]) => v !== expected);
  if (bad.length > 0) {
    console.error(`版本号不一致（期望 ${expected}，来自 .env）：`);
    for (const [file, v] of bad) console.error(`  ${file}: ${v ?? "未找到"}`);
    console.error("运行 npm run sync-version 修正。");
    process.exit(1);
  }
  console.log(`版本号一致：${expected}`);
}

function sync(source, version) {
  if (!SEMVER.test(version)) {
    throw new Error(`非法版本号 "${version}"（需为 semver，如 1.2.3 或 1.2.3-beta.1）`);
  }
  if (source === "env") patchPackageJson(version);
  else patchEnv(version);
  patchPackageLock(version);
  patchCargoToml(version);
  patchCargoLock(version);
  console.log(
    `版本号已同步为 ${version}（.env → package.json / package-lock.json / Cargo.toml / Cargo.lock）`,
  );
}

const mode = process.argv[2];
if (mode === "--check") {
  check();
} else if (mode === "--from-package") {
  // npm version 生命周期：package.json 已被 npm 更新，回写 .env 后同步其余文件
  const version = readVersionFromPackageJson();
  sync("package", version);
  execSync(`git add ${GIT_ADD_TARGETS.join(" ")}`, { cwd: root, stdio: "inherit" });
} else if (mode === undefined) {
  sync("env", readVersionFromEnv());
} else {
  console.error("未知参数，用法见文件头注释。");
  process.exit(1);
}

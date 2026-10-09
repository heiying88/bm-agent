#!/usr/bin/env node
/**
 * [LOCAL PATCH] dream-privacy-config delivery
 * Pure-Node re-implementation of `scripts/frontend-package.cjs stage:prebuilt`
 * for workstations without 7z/zip on PATH (Windows). Produces the exact same
 * committed package layout (lotus-frontend.zip + canonical sidecar manifest)
 * with byte-identical hashing/ordering rules, then self-verifies like the Rust
 * validator does (canonical manifest, hash round-trip, entry presence).
 *
 * Usage:
 *   node scripts/frontend-package-node.cjs [--dist <dir>] [--frontend <dir>]
 *     --dist      built dist directory (default: <frontend>/dist)
 *     --frontend  frontend package root with package.json (default: Lotus-main)
 */

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const zlib = require("node:zlib");

const ROOT = path.resolve(__dirname, "..");
const OUTPUT_DIR = path.join(ROOT, "crates", "app", "bamboo-server", "frontend_package");
const FRONTEND_MANIFEST_FILE = "frontend-manifest.json";

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) continue;
    if (index + 1 < argv.length && !argv[index + 1].startsWith("--")) {
      args[arg.slice(2)] = argv[++index];
    } else {
      args[arg.slice(2)] = true;
    }
  }
  return args;
}

function fail(message) {
  throw new Error(message);
}

function comparePortablePath(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

function listFilesRecursively(rootDir, ignoredPaths = []) {
  const ignored = new Set(ignoredPaths);
  const files = [];
  const walk = (currentDir) => {
    const entries = fs
      .readdirSync(currentDir, { withFileTypes: true })
      .sort((left, right) => comparePortablePath(left.name, right.name));
    for (const entry of entries) {
      const absolutePath = path.join(currentDir, entry.name);
      const relativePath = path.relative(rootDir, absolutePath).replace(/\\/g, "/");
      if (ignored.has(relativePath)) continue;
      if (entry.isDirectory()) walk(absolutePath);
      else if (entry.isFile()) files.push({ absolutePath, relativePath });
      else fail(`Frontend resource ${relativePath} must be a regular file`);
    }
  };
  walk(rootDir);
  return files.sort((left, right) => comparePortablePath(left.relativePath, right.relativePath));
}

function computeDirectoryHash(rootDir, ignoredPaths = []) {
  const hash = crypto.createHash("sha256");
  for (const file of listFilesRecursively(rootDir, ignoredPaths)) {
    hash.update(file.relativePath);
    hash.update("\0");
    hash.update(fs.readFileSync(file.absolutePath));
    hash.update("\0");
  }
  return `sha256:${hash.digest("hex")}`;
}

// ---- Minimal deterministic ZIP writer (deflate via zlib, stored fallback) ----
const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (let index = 0; index < buffer.length; index += 1) {
    crc = (crc >>> 8) ^ CRC_TABLE[(crc ^ buffer[index]) & 0xff];
  }
  return (crc ^ -1) >>> 0;
}

function dosDateTime(date) {
  const year = Math.max(1980, date.getFullYear());
  const time =
    (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2);
  const day = ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  return { time, day };
}

function createZip(entries, zipPath) {
  // entries: [{ name (posix rel), data (Buffer), isDirectory }]
  const chunks = [];
  const central = [];
  let offset = 0;
  const { time, day } = dosDateTime(new Date());
  for (const entry of entries) {
    const nameBytes = Buffer.from(entry.name, "utf8");
    const crc = crc32(entry.data);
    let method = 8;
    let payload = zlib.deflateRawSync(entry.data, { level: 9 });
    if (payload.length >= entry.data.length) {
      method = 0;
      payload = entry.data;
    }
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0x0800, 6); // UTF-8 names
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(time, 10);
    local.writeUInt16LE(day, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(payload.length, 18);
    local.writeUInt32LE(entry.data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28);
    chunks.push(local, nameBytes, payload);

    const centralEntry = Buffer.alloc(46);
    centralEntry.writeUInt32LE(0x02014b50, 0);
    centralEntry.writeUInt16LE(20, 4); // version made by
    centralEntry.writeUInt16LE(20, 6); // version needed
    centralEntry.writeUInt16LE(0x0800, 8);
    centralEntry.writeUInt16LE(method, 10);
    centralEntry.writeUInt16LE(time, 12);
    centralEntry.writeUInt16LE(day, 14);
    centralEntry.writeUInt32LE(crc, 16);
    centralEntry.writeUInt32LE(payload.length, 20);
    centralEntry.writeUInt32LE(entry.data.length, 24);
    centralEntry.writeUInt16LE(nameBytes.length, 28);
    centralEntry.writeUInt16LE(0, 30); // extra
    centralEntry.writeUInt16LE(0, 32); // comment
    centralEntry.writeUInt16LE(0, 34); // disk
    centralEntry.writeUInt16LE(0, 36); // internal attrs
    centralEntry.writeUInt32LE(entry.isDirectory ? 0x10 : 0, 38);
    centralEntry.writeUInt32LE(offset, 42);
    central.push(centralEntry, nameBytes);
    offset += local.length + nameBytes.length + payload.length;
  }
  const centralBuffer = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralBuffer.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  fs.writeFileSync(zipPath, Buffer.concat([...chunks, centralBuffer, end]));
}

function collectZipEntries(stageDir) {
  const entries = [];
  const directories = new Set();
  for (const file of listFilesRecursively(stageDir)) {
    const segments = file.relativePath.split("/");
    for (let index = 1; index < segments.length; index += 1) {
      directories.add(segments.slice(0, index).join("/") + "/");
    }
    entries.push({ name: file.relativePath, data: fs.readFileSync(file.absolutePath) });
  }
  for (const directory of [...directories].sort(comparePortablePath)) {
    entries.push({ name: directory, data: Buffer.alloc(0), isDirectory: true });
  }
  entries.sort((left, right) => comparePortablePath(left.name, right.name));
  return entries;
}

// ---- Main ----
const args = parseArgs(process.argv.slice(2));
const frontendRoot = path.resolve(
  ROOT,
  args.frontend || "Lotus-main",
);
const distDir = path.resolve(args.dist ? path.join(ROOT, args.dist) : path.join(frontendRoot, "dist"));

const packageJsonPath = path.join(frontendRoot, "package.json");
if (!fs.existsSync(packageJsonPath)) fail(`Frontend package.json missing at ${packageJsonPath}`);
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
if (packageJson.name !== "@bigduu/lotus") {
  fail(`Unexpected frontend package name ${JSON.stringify(packageJson.name)}; expected @bigduu/lotus`);
}
if (!fs.existsSync(path.join(distDir, "index.html"))) {
  fail(`Frontend dist is missing index.html: ${distDir}`);
}

const workspace = fs.mkdtempSync(path.join(path.dirname(OUTPUT_DIR), ".frontend-package-node-"));
const stageDir = path.join(workspace, "stage");
const packageRoot = path.join(workspace, "package");
fs.mkdirSync(stageDir, { recursive: true });
fs.mkdirSync(packageRoot, { recursive: true });
try {
  fs.cpSync(distDir, stageDir, { recursive: true });
  const manifest = {
    schema_version: 1,
    frontend_name: "lotus",
    frontend_version: packageJson.version,
    bundle_hash: computeDirectoryHash(stageDir),
    built_at: new Date().toISOString(),
    entry: "index.html",
  };
  const content = `${JSON.stringify(manifest, null, 2)}\n`;
  fs.writeFileSync(path.join(stageDir, FRONTEND_MANIFEST_FILE), content);
  fs.writeFileSync(path.join(packageRoot, FRONTEND_MANIFEST_FILE), content);

  createZip(collectZipEntries(stageDir), path.join(packageRoot, "lotus-frontend.zip"));

  // Self-verify (mirrors validateStagedPackage for the legacy package name):
  // canonical manifest, hash round-trip over staged files, entry presence.
  const sidecar = fs.readFileSync(path.join(packageRoot, FRONTEND_MANIFEST_FILE), "utf8");
  const parsed = JSON.parse(sidecar);
  if (sidecar !== `${JSON.stringify(parsed, null, 2)}\n`) fail("Manifest is not canonical JSON");
  if (parsed.frontend_name !== "lotus" || parsed.entry !== "index.html") {
    fail("Manifest identity fields are invalid");
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(parsed.bundle_hash)) fail("Manifest bundle hash is invalid");
  const recomputed = computeDirectoryHash(stageDir, [FRONTEND_MANIFEST_FILE]);
  if (recomputed !== parsed.bundle_hash) {
    fail(`Bundle hash mismatch: ${recomputed} != ${parsed.bundle_hash}`);
  }

  // Swap the committed package atomically (backup + rename, like the cjs path).
  fs.mkdirSync(path.dirname(OUTPUT_DIR), { recursive: true });
  const backupDir = `${OUTPUT_DIR}.backup-${process.pid}-${Date.now()}`;
  const hadPrevious = fs.existsSync(OUTPUT_DIR);
  if (hadPrevious) fs.renameSync(OUTPUT_DIR, backupDir);
  try {
    fs.renameSync(packageRoot, OUTPUT_DIR);
  } catch (error) {
    if (hadPrevious && !fs.existsSync(OUTPUT_DIR)) fs.renameSync(backupDir, OUTPUT_DIR);
    throw error;
  }
  if (hadPrevious) fs.rmSync(backupDir, { recursive: true, force: true });

  console.log(`✅ Created embedded frontend package: ${path.join(OUTPUT_DIR, "lotus-frontend.zip")}`);
  console.log(`✅ Wrote embedded frontend manifest: ${path.join(OUTPUT_DIR, FRONTEND_MANIFEST_FILE)}`);
  console.log(`ℹ️ Embedded frontend: lotus@${manifest.frontend_version}`);
  console.log(`ℹ️ Embedded frontend hash: ${manifest.bundle_hash}`);
} finally {
  fs.rmSync(workspace, { recursive: true, force: true });
}

import fs from "node:fs";
import path from "node:path";

const platformName = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
const BIN_DIR = path.resolve("src-tauri/resources/bin", platformName);
const baseNames = ["ffmpeg", "ffprobe", "aria2c"];
const targetNames = process.platform === "win32" ? baseNames.map((name) => `${name}.exe`) : baseNames;
const defaultSourceDir = path.resolve(process.cwd(), "bin", platformName);
const sourceDir = process.env.BIN_SOURCE_DIR || defaultSourceDir;

function ensureDir(pathname) {
  if (!fs.existsSync(pathname)) {
    fs.mkdirSync(pathname, { recursive: true });
  }
}

function copyFile(sourcePath, targetPath) {
  fs.copyFileSync(sourcePath, targetPath);
  if (process.platform !== "win32") {
    fs.chmodSync(targetPath, 0o755);
  }
}

function main() {
  if (!fs.existsSync(sourceDir)) {
    throw new Error(`未找到源目录: ${sourceDir}`);
  }
  ensureDir(BIN_DIR);

  const missing = [];
  for (const name of targetNames) {
    const sourcePath = path.join(sourceDir, name);
    if (!fs.existsSync(sourcePath)) {
      missing.push(name);
      continue;
    }
    const targetPath = path.join(BIN_DIR, name);
    copyFile(sourcePath, targetPath);
  }

  if (missing.length > 0) {
    throw new Error(`缺少二进制文件: ${missing.join(", ")}`);
  }

  console.log(`二进制已写入: ${BIN_DIR}`);
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exit(1);
}

import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const targetVersion = process.argv[2] || "1.0.0";

if (!process.argv[2]) {
  console.warn("未提供版本号，默认使用 1.0.0");
}

const packageJsonPath = path.join(root, "package.json");
const tauriConfPath = path.join(root, "src-tauri", "tauri.conf.json");
const cargoTomlPath = path.join(root, "src-tauri", "Cargo.toml");

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJson(filePath, data) {
  fs.writeFileSync(filePath, `${JSON.stringify(data, null, 2)}\n`);
}

function updateCargoTomlVersion(tomlText, version) {
  const sectionStart = tomlText.indexOf("[package]");
  if (sectionStart === -1) {
    throw new Error("Cargo.toml 缺少 [package] 段落");
  }
  const nextSectionMatch = tomlText.slice(sectionStart + 1).match(/\n\[/);
  const sectionEnd = nextSectionMatch
    ? sectionStart + 1 + nextSectionMatch.index
    : tomlText.length;
  const section = tomlText.slice(sectionStart, sectionEnd);
  const updatedSection = section.replace(
    /^version\s*=\s*\"[^\"]+\"/m,
    `version = \"${version}\"`,
  );
  if (section === updatedSection) {
    throw new Error("Cargo.toml 未找到 version 字段");
  }
  return `${tomlText.slice(0, sectionStart)}${updatedSection}${tomlText.slice(sectionEnd)}`;
}

const packageJson = readJson(packageJsonPath);
packageJson.version = targetVersion;
writeJson(packageJsonPath, packageJson);

const tauriConf = readJson(tauriConfPath);
tauriConf.version = targetVersion;
writeJson(tauriConfPath, tauriConf);

const cargoToml = fs.readFileSync(cargoTomlPath, "utf8");
const updatedCargoToml = updateCargoTomlVersion(cargoToml, targetVersion);
fs.writeFileSync(cargoTomlPath, updatedCargoToml);

console.log(`版本已同步为: ${targetVersion}`);

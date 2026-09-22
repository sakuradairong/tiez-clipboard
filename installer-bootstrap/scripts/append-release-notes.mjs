import fs from "node:fs";

const version = process.argv[2];
const existingPath = process.argv[3];
const notesPath = process.argv[4];
if (!version || !existingPath || !notesPath) {
  throw new Error("usage: append-release-notes.mjs <version> <existing-notes> <output>");
}

const marker = "<!-- tiez-bootstrapper -->";
const existing = fs.readFileSync(existingPath, "utf8");
if (existing.includes(marker)) {
  console.log("unchanged");
  process.exit(0);
}

const addition = [
  marker,
  "### Install wizard (additional)",
  "",
  `- \`TieZ-${version}-windows-x64-installer.exe\` is an optional install wizard. It embeds the NSIS setup and runs that setup with \`/S\` only.`,
  `- \`TieZ_${version}_x64-setup.exe\` remains the primary installer. The in-app updater still downloads that NSIS setup via \`latest.json\`.`,
  "",
].join("\n");

fs.writeFileSync(notesPath, `${existing.trimEnd()}\n\n${addition}`);
console.log("updated");

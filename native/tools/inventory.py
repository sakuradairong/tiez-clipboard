"""Track legacy compatibility surface; --check fails when source inventory changes."""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "native/compatibility.json"


def inventory():
    rust = sorted((ROOT / "src-tauri/src").rglob("*.rs"))
    frontend = sorted(p for p in (ROOT / "src").rglob("*")
                      if p.suffix in (".ts", ".tsx") and ".test." not in p.name)
    texts = {p: p.read_text(encoding="utf-8") for p in rust + frontend}
    main = texts[ROOT / "src-tauri/src/main.rs"]
    block = re.search(r"generate_handler!\s*\[([\s\S]*?)\]", main).group(1)
    registered = [name.strip() for name in block.split(",") if name.strip()]
    if not all(re.fullmatch(r"[A-Za-z_][\w:]*", name) for name in registered):
        raise ValueError("Command registry format changed; review the extractor.")
    declarations = {}
    for path in rust:
        for match in re.finditer(r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", texts[path]):
            declarations.setdefault(match.group(1), []).append(path.relative_to(ROOT).as_posix())
    commands = [{"name": name.rsplit("::", 1)[-1], "registry_path": name,
                 "declarations": declarations.get(name.rsplit("::", 1)[-1], [])}
                for name in registered]
    invocations = sorted({match for p in frontend for match in re.findall(
        r"""\binvoke(?:<[^;\n]*?>)?\s*\(\s*["'](\w+)["']""", texts[p])})
    seed = texts[ROOT / "src-tauri/src/database.rs"].split("pub fn seed_defaults", 1)[1].split("#[cfg(test)]", 1)[0]
    defaults = sorted(set(re.findall(r"VALUES\s*\(\s*'([^']+)'", seed)))
    names = {command["name"] for command in commands}
    # Conservative: also retain setting-like literals with no seeded default.
    candidates = sorted({value for text in texts.values() for value in re.findall(
        r"""["']((?:app\.|mqtt_|cloud_sync_|ai_|relay_|file_transfer_|file_server_)[A-Za-z0-9_.]+)["']""", text)
                         if value not in names} | set(defaults))
    themes = re.findall(r"""\bid:\s*["']([^"']+)["']""", texts[ROOT / "src/shared/config/themes.ts"])
    ui = sorted(p.relative_to(ROOT).as_posix() for p in frontend
                if p.suffix == ".tsx" and ("features" in p.parts or "components" in p.parts or p.name == "App.tsx"))
    return {"format": 1, "reference": "Legacy sources in this branch; no runtime equivalence implied",
            "registered_commands": commands,
            "declared_unregistered_commands": sorted(set(declarations) - names),
            "frontend_literal_invocations": invocations,
            "unregistered_frontend_literal_invocations": sorted(set(invocations) - names),
            "seeded_setting_keys": defaults, "setting_literal_candidates": candidates,
            "builtin_themes": themes, "ui_modules": ui}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = inventory()
    content = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != content:
            raise SystemExit("Compatibility inventory is stale. Review legacy surface changes and regenerate.")
    else:
        OUTPUT.write_text(content, encoding="utf-8", newline="\n")
    print(json.dumps({"commands": len(result["registered_commands"]),
                      "seeded_settings": len(result["seeded_setting_keys"]),
                      "setting_candidates": len(result["setting_literal_candidates"]),
                      "ui_modules": len(result["ui_modules"]), "themes": len(result["builtin_themes"]),
                      "unregistered_calls": result["unregistered_frontend_literal_invocations"]}, ensure_ascii=False))


if __name__ == "__main__":
    main()

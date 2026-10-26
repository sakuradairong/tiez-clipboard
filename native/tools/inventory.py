"""Track legacy compatibility surface; --check fails when source inventory changes."""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / "native/compatibility.json"
OPENER_TYPES = ("text", "code", "rich_text", "image", "file", "video", "url", "link")
# Preserve offsets while excluding comments and quoted examples from call matching.
LITERALS = re.compile(r'''//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`''')
ARGUMENT = re.compile(r'''"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|[\w.$]+''')


def first_arguments(text, callee):
    code = LITERALS.sub(lambda match: re.sub(r"[^\n]", " ", match.group()), text)
    for match in re.finditer(r"\b(?:" + callee + r")\b", code):
        i = match.end()
        while i < len(code) and code[i].isspace():
            i += 1
        if i < len(code) and code[i] == "<":
            depth = 1
            i += 1
            while i < len(code) and depth:
                if code[i] == "<":
                    depth += 1
                elif code[i] == ">" and code[i - 1] != "=":
                    depth -= 1
                i += 1
            while i < len(code) and code[i].isspace():
                i += 1
        if i >= len(code) or code[i] != "(":
            continue
        i += 1
        # A quoted argument is masked too, so skip only actual leading comments.
        while i < len(text):
            if text[i].isspace():
                i += 1
            elif text.startswith("/*", i):
                end = text.find("*/", i + 2)
                i = len(text) if end < 0 else end + 2
            elif text.startswith("//", i):
                end = text.find("\n", i + 2)
                i = len(text) if end < 0 else end + 1
            else:
                break
        argument = ARGUMENT.match(text, i)
        if argument:
            end = argument.end()
            while end < len(code) and code[end].isspace():
                end += 1
            exact = end < len(code) and code[end] in ",)"
            yield text.count("\n", 0, match.start()) + 1, argument.group(), exact
        else:
            yield text.count("\n", 0, match.start()) + 1, "<expression>", False


def frontend_surface(texts):
    literal, dynamic, storage, settings = set(), [], [], set()
    for path, text in texts.items():
        for line, argument, exact in first_arguments(text, "invoke"):
            if exact and re.fullmatch(r'''["']\w+["']''', argument):
                literal.add(argument[1:-1])
            else:
                candidates = set()
                if exact and re.fullmatch(r"\w+", argument):
                    union = r"\b" + re.escape(argument) + r'''\s*:\s*((?:["']\w+["']\s*\|\s*)+["']\w+["'])'''
                    for match in re.finditer(union, text):
                        candidates.update(re.findall(r'''["'](\w+)["']''', match.group(1)))
                dynamic.append({"file": path, "line": line, "argument": argument, "argument_complete": exact,
                                "declared_string_candidates": sorted(candidates)})
        for _, argument, exact in first_arguments(text, "saveAppSetting"):
            if exact and re.fullmatch(r'''["']\w+["']''', argument):
                settings.add("app." + argument[1:-1])
        for line, argument, exact in first_arguments(text, r"(?:window\.)?(?:localStorage|sessionStorage)\.(?:getItem|setItem|removeItem)"):
            kind, key = "expression", argument
            if exact and argument[0] in "\"'`":
                key = argument[1:-1]
                kind = "template" if "${" in key else "literal"
            elif exact and re.fullmatch(r"\w+", argument):
                constant = re.search(r"\bconst\s+" + re.escape(argument) + r'''\s*=\s*(["'])([^"']+)\1''', text)
                if constant:
                    kind, key = "constant", constant.group(2)
            storage.append({"file": path, "line": line, "key": key, "kind": kind})
    return sorted(literal), dynamic, sorted(settings), storage


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
        for match in re.finditer(r"#\[(?:tauri::)?command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", texts[path]):
            declarations.setdefault(match.group(1), []).append(path.relative_to(ROOT).as_posix())
    commands = [{"name": name.rsplit("::", 1)[-1], "registry_path": name,
                 "declarations": declarations.get(name.rsplit("::", 1)[-1], [])}
                for name in registered]
    invocations, dynamic_calls, dynamic_keys, browser_storage = frontend_surface(
        {p.relative_to(ROOT).as_posix(): texts[p] for p in frontend})
    # Reviewed family includes file/link, although the legacy UI shows six opener rows.
    if "`app.${type}`" not in texts[ROOT / "src/App.tsx"]:
        raise ValueError("Dynamic app setting family changed; review the inventory.")
    opener_keys = ["app." + kind for kind in OPENER_TYPES]
    seed = texts[ROOT / "src-tauri/src/database.rs"].split("pub fn seed_defaults", 1)[1].split("#[cfg(test)]", 1)[0]
    defaults = sorted(set(re.findall(r"VALUES\s*\(\s*'([^']+)'", seed)))
    names = {command["name"] for command in commands}
    # Conservative: also retain setting-like literals with no seeded default.
    candidates = sorted({value for text in texts.values() for value in re.findall(
        r'''["']((?:app\.|mqtt_|cloud_sync_|ai_|relay_|file_transfer_|file_server_)[A-Za-z0-9_.]+)["']''', text)
                         if value not in names} | set(defaults) | set(dynamic_keys) | set(opener_keys))
    themes = re.findall(r'''\bid:\s*["']([^"']+)["']''', texts[ROOT / "src/shared/config/themes.ts"])
    ui = sorted(p.relative_to(ROOT).as_posix() for p in frontend
                if p.suffix == ".tsx" and ("features" in p.parts or "components" in p.parts or p.name == "App.tsx"))
    return {"format": 2, "reference": "Legacy sources in this branch; no runtime equivalence implied",
            "registered_commands": commands,
            "declared_unregistered_commands": sorted(set(declarations) - names),
            "frontend_literal_invocations": invocations,
            "frontend_dynamic_invocations": dynamic_calls,
            "dynamic_setting_families": [{"template": "app.${type}", "file": "src/App.tsx",
                                          "reviewed_opener_keys": opener_keys}],
            "browser_storage_references": browser_storage,
            "coverage_limits": ["Dynamic string candidates are evidence, not evaluated bindings.",
                                "Template interpolation, aliases, plugins, events, native menus and background services require manual review.",
                                "Browser keys are inventoried without reading stored values; migration and runtime acceptance remain pending."],
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

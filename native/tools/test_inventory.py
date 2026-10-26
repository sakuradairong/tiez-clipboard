"""Migration extraction regressions; fixtures never read user application data."""
import importlib.util
from pathlib import Path
import unittest
import sys
sys.dont_write_bytecode = True

SPEC = importlib.util.spec_from_file_location("inventory", Path(__file__).with_name("inventory.py"))
inventory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(inventory)


class InventoryTests(unittest.TestCase):
    def surface(self, source):
        return inventory.frontend_surface({"fixture.tsx": source})

    def test_semicolons_nested_and_multiline_generics(self):
        literal, dynamic, _, _ = self.surface('''
invoke<{ name: string; path: string }[]>("scan_installed_apps");
invoke<Array<Record<string, { name: string; path: string }>>>("get_associated_apps");
invoke<{
  callback: (value: string) => boolean;
}>("multi_line", {});
invoke("ordinary");
''')
        self.assertEqual(literal, ["get_associated_apps", "multi_line", "ordinary", "scan_installed_apps"])
        self.assertEqual(dynamic, [])

    def test_comments_literals_and_quoted_type_delimiters(self):
        literal, dynamic, _, _ = self.surface('''
// invoke("comment");
/* invoke("block"); */
const example = `invoke("template")`;
const other = 'invoke("string")';
invoke<{ value: ">"; }>(/* comment */ "real");
''')
        self.assertEqual(literal, ["real"])
        self.assertEqual(dynamic, [])

    def test_dynamic_union_is_not_a_literal_call(self):
        literal, dynamic, _, _ = self.surface('''
const aliases = { command: "super" };
function update(command: "set_relay_send_hotkey" | "set_relay_fetch_hotkey") {
  invoke(command, {});
}
invoke(prefix + "suffix");
''')
        self.assertEqual(literal, [])
        self.assertEqual(dynamic[0]["line"], 4)
        self.assertEqual(dynamic[0]["declared_string_candidates"],
                         ["set_relay_fetch_hotkey", "set_relay_send_hotkey"])
        self.assertEqual(dynamic[1]["declared_string_candidates"], [])

    def test_browser_keys_not_stored_values(self):
        _, _, settings, storage = self.surface('''
const VIEW = "tiez_tag_manager_view_mode";
saveAppSetting('theme', theme);
props.saveAppSetting("compact_mode", "true");
localStorage.setItem("token", privateToken);
window.localStorage.getItem(VIEW);
localStorage.getItem(`tiez_store_css_${themeId}`);
localStorage.removeItem(prefix + suffix);
''')
        self.assertEqual(settings, ["app.compact_mode", "app.theme"])
        self.assertEqual([(item["key"], item["kind"]) for item in storage],
                         [("token", "literal"), ("tiez_tag_manager_view_mode", "constant"),
                          ("tiez_store_css_${themeId}", "template"), ("prefix", "expression")])
        self.assertNotIn("privateToken", str(storage))

    def test_current_source_contract(self):
        result = inventory.inventory()
        self.assertEqual(result["format"], 2)
        self.assertEqual(len(result["frontend_literal_invocations"]), 113)
        self.assertEqual(len(result["registered_commands"]), 132)
        dynamic = result["frontend_dynamic_invocations"]
        self.assertEqual(len(dynamic), 1)
        self.assertEqual(dynamic[0]["declared_string_candidates"],
                         ["set_relay_fetch_hotkey", "set_relay_send_hotkey"])
        self.assertTrue({"app." + kind for kind in inventory.OPENER_TYPES}
                        <= set(result["setting_literal_candidates"]))
        self.assertTrue(any(item["key"] == "tiez_theme_store_token" for item in result["browser_storage_references"]))
        self.assertTrue(any(item["key"] == "tiez_tag_manager_view_mode" for item in result["browser_storage_references"]))


if __name__ == "__main__":
    unittest.main()

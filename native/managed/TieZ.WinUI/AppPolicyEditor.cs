using System.Text.Json;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;

namespace TieZ.Native;

// Edits the existing JSON contract without materializing defaults or dropping extensions.
internal sealed class AppPolicyEditor : UserControl
{
    private static readonly string[] Types = ["text", "code", "url", "rich_text", "image", "file", "video"];
    private static readonly string[] TypeLabels = ["文本", "代码", "链接", "富文本", "图片", "文件", "视频"];
    private readonly CleanupPreviewView preview;
    internal CleanupPreviewView RulePreview => preview;
    internal void Initialize(TieZ.Core.NativeCore core) => preview.Initialize(core);
    private readonly StackPanel root = new() { Spacing = 12 };
    private readonly StackPanel fields = new() { Spacing = 10 };
    private readonly ComboBox selection = new() { Header = "应用策略（按执行顺序）", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly Button add = new() { Content = "添加策略" };
    private readonly Button remove = new() { Content = "删除此策略" };
    private readonly Button up = new() { Content = "上移" };
    private readonly Button down = new() { Content = "下移" };
    private readonly ToggleSwitch enabled = new() { Header = "启用此策略" };
    private readonly TextBox name = new() { Header = "应用名称", PlaceholderText = "例如：notepad" };
    private readonly TextBox path = new() { Header = "路径、可执行文件名或应用 ID", PlaceholderText = "例如：notepad.exe" };
    private readonly ComboBox action = new() { Header = "匹配后执行", ItemsSource = new[] { "清理文字", "忽略采集" } };
    private readonly TextBox rules = new() { Header = "此应用的清理规则（每行：正则 => 替换文本）", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80 };
    private readonly TextBlock semantics = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
    private readonly TextBlock extraTypes = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
    private readonly InfoBar invalid = new() { Severity = InfoBarSeverity.Warning, IsClosable = false };
    private readonly TextBox raw = new() { Header = "策略配置", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 100 };
    private readonly CheckBox[] kinds = Types.Select((_, i) => new CheckBox { Content = TypeLabels[i] }).ToArray();
    private JsonArray? policies, acceptedPolicies;
    private string acceptedText = "[]";
    private readonly Dictionary<JsonObject, JsonObject> originals = new(ReferenceEqualityComparer.Instance);
    private bool loading;
    internal event Action? Changed;
    internal string Text { get => raw.Text; set => raw.Text = value; }
    internal int Count => policies?.Count ?? 0;
    internal bool StructuredAvailable => policies is not null;
    internal bool HasRetainedData => policies is not null || acceptedPolicies is not null || originals.Count > 0
        || preview.HasRetainedData || acceptedText.Length > 0 || raw.Text.Length > 0 || name.Text.Length > 0 || path.Text.Length > 0 || rules.Text.Length > 0;
    private JsonObject? Selected => policies is not null && selection.SelectedIndex >= 0
        && selection.SelectedIndex < policies.Count ? policies[selection.SelectedIndex] as JsonObject : null;

    internal AppPolicyEditor()
    {
        preview = new CleanupPreviewView(() => Selected is { } policy ? String(policy, "cleanupRules", "") : "");
        root.Children.Add(new TextBlock { Text = "按应用清理策略", FontSize = 20 });
        root.Children.Add(new TextBlock { Text = "先按顺序执行匹配的应用策略，再执行全局清理；清理可累加，忽略会立即停止采集。名称和路径任一匹配即可；支持可执行文件名、应用 ID 和 /正则/。", TextWrapping = TextWrapping.Wrap });
        root.Children.Add(selection);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (var button in new[] { add, remove, up, down }) buttons.Children.Add(button);
        root.Children.Add(buttons); root.Children.Add(invalid);
        fields.Children.Add(enabled); fields.Children.Add(name); fields.Children.Add(path); fields.Children.Add(action);
        fields.Children.Add(new TextBlock { Text = "适用内容类型" });
        var typePanel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        for (int i = 0; i < kinds.Length; i++)
        {
            int index = i; typePanel.Children.Add(kinds[i]);
            kinds[i].Checked += (_, _) => ChangeType(index);
            kinds[i].Unchecked += (_, _) => ChangeType(index);
        }
        fields.Children.Add(new ScrollViewer { Content = typePanel, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });
        fields.Children.Add(extraTypes); fields.Children.Add(rules); fields.Children.Add(semantics); fields.Children.Add(preview);
        root.Children.Add(fields);
        root.Children.Add(new Expander { Header = "兼容配置 · 查看或编辑 JSON", Content = raw, HorizontalAlignment = HorizontalAlignment.Stretch });
        Content = root;
        selection.SelectionChanged += (_, _) => { if (!loading) ShowSelected(); };
        add.Click += (_, _) => Add(); remove.Click += (_, _) => Remove();
        up.Click += (_, _) => Move(-1); down.Click += (_, _) => Move(1);
        enabled.Toggled += (_, _) => SetField("enabled", JsonValue.Create(enabled.IsOn));
        name.TextChanging += (_, _) => SetField("appName", JsonValue.Create(name.Text));
        path.TextChanging += (_, _) => SetField("appPath", JsonValue.Create(path.Text));
        action.SelectionChanged += (_, _) => SetField("action", JsonValue.Create(action.SelectedIndex == 1 ? "ignore" : "clean"));
        rules.TextChanging += (_, _) =>
        {
            if (loading || Selected is not { } policy) return;
            string stored = String(policy, "cleanupRules", "");
            // TextBox normalizes CRLF. Do not rewrite an unchanged rule when editing another field.
            string original = originals.TryGetValue(policy, out var initial) ? String(initial, "cleanupRules", "") : stored;
            string desired = Lines(original) == Lines(rules.Text) ? original : rules.Text;
            if (stored != desired) SetField("cleanupRules", JsonValue.Create(desired));
        };
        raw.TextChanging += (_, _) => { if (!loading) { Parse(); Changed?.Invoke(); } };
        Text = "[]";
    }

    internal void Clear()
    {
        preview.Dispose(); acceptedText = ""; acceptedPolicies = null;
        Text = "";
    }
    internal void Load(string value)
    {
        acceptedText = value;
        if (raw.Text == value) Parse(); else Text = value;
        acceptedPolicies = policies?.DeepClone() as JsonArray;
    }
    private static string Lines(string value) => value.Replace("\r\n", "\n").Replace("\r", "\n");
    private static T Read<T>(JsonNode? value)
    {
        if (value is JsonValue scalar && scalar.TryGetValue<T>(out var result) && result is not null) return result;
        throw new JsonException("Unexpected policy field type.");
    }
    private static string String(JsonObject policy, string key, string fallback) =>
        policy.TryGetPropertyValue(key, out var value) ? Read<string>(value) : fallback;
    private static bool Enabled(JsonObject policy) => !policy.TryGetPropertyValue("enabled", out var value) || Read<bool>(value);
    private static string[] ContentTypes(JsonObject policy)
    {
        if (!policy.TryGetPropertyValue("contentTypes", out var value)) return Types;
        if (value is not JsonArray array) throw new JsonException("Expected content types array.");
        return array.Select(Read<string>).ToArray();
    }
    private static void Validate(JsonObject policy)
    {
        foreach (string key in new[] { "id", "appName", "appPath", "action", "cleanupRules" })
            _ = String(policy, key, "");
        _ = Enabled(policy); _ = ContentTypes(policy);
        if (String(policy, "action", "clean").ToLowerInvariant() is not ("clean" or "ignore"))
            throw new JsonException("Unknown policy action.");
    }
    private void Parse()
    {
        int previous = selection.SelectedIndex;
        originals.Clear();
        try
        {
            policies = JsonNode.Parse(raw.Text) as JsonArray ?? throw new JsonException("Expected array.");
            foreach (var node in policies)
            {
                var policy = node as JsonObject ?? throw new JsonException("Expected policy object.");
                Validate(policy); originals.Add(policy, (JsonObject)policy.DeepClone());
            }
            invalid.IsOpen = false;
        }
        catch (Exception error) when (error is JsonException or InvalidOperationException or ArgumentException)
        {
            policies = null; originals.Clear();
            invalid.Message = "配置无法用表单读取，原始内容已保留。请展开兼容配置修正；无效配置不会保存。";
            invalid.IsOpen = true;
        }
        RebuildSelection(previous);
    }
    private string Label(int index)
    {
        var policy = (JsonObject)policies![index]!;
        string title = String(policy, "appName", "");
        if (string.IsNullOrWhiteSpace(title)) title = String(policy, "appPath", "");
        if (string.IsNullOrWhiteSpace(title)) title = "未指定应用";
        return $"{index + 1}. {title}" + (Enabled(policy) ? "" : "（已停用）");
    }
    private void RebuildSelection(int selected)
    {
        loading = true;
        try
        {
            selection.Items.Clear();
            for (int i = 0; i < Count; i++) selection.Items.Add(new ComboBoxItem { Content = Label(i) });
            selection.SelectedIndex = Count == 0 ? -1 : Math.Clamp(selected, 0, Count - 1);
        }
        finally { loading = false; }
        ShowSelected();
    }
    private void ShowSelected()
    {
        preview.Invalidate();
        if (Selected is null) preview.Reset();
        loading = true;
        try
        {
            var policy = Selected;
            fields.Visibility = policy is null ? Visibility.Collapsed : Visibility.Visible;
            add.IsEnabled = policies is not null; selection.IsEnabled = Count > 0;
            remove.IsEnabled = policy is not null;
            up.IsEnabled = policy is not null && selection.SelectedIndex > 0;
            down.IsEnabled = policy is not null && selection.SelectedIndex < Count - 1;
            enabled.IsOn = policy is not null && Enabled(policy);
            name.Text = policy is null ? "" : String(policy, "appName", "");
            path.Text = policy is null ? "" : String(policy, "appPath", "");
            action.SelectedIndex = policy is not null && String(policy, "action", "clean").Equals("ignore", StringComparison.OrdinalIgnoreCase) ? 1 : 0;
            rules.Text = policy is null ? "" : String(policy, "cleanupRules", "");
            var types = policy is null ? [] : ContentTypes(policy);
            for (int i = 0; i < kinds.Length; i++) kinds[i].IsChecked = types.Contains(Types[i], StringComparer.OrdinalIgnoreCase);
            extraTypes.Text = string.Join("、", types.Where(kind => !Types.Contains(kind, StringComparer.OrdinalIgnoreCase)));
            if (extraTypes.Text.Length > 0) extraTypes.Text = "保留的其他类型：" + extraTypes.Text;
            Describe();
        }
        finally { loading = false; }
    }
    private void Describe()
    {
        bool ignore = action.SelectedIndex == 1;
        rules.IsEnabled = !ignore;
        semantics.Text = ignore ? "未选择类型时忽略全部类型。已保存的清理规则会保留，切回清理后可继续使用。"
            : "未选择类型时此策略不生效。清理规则仅转换文字类型；替换为空白或 __IGNORE_CAPTURE__ 会忽略本条采集。";
        if (string.IsNullOrWhiteSpace(name.Text) && string.IsNullOrWhiteSpace(path.Text))
            semantics.Text += " 请填写应用名称或路径，否则此策略不会匹配。";
    }
    private void Publish()
    {
        preview.Invalidate();
        loading = true;
        try { raw.Text = JsonNode.DeepEquals(policies, acceptedPolicies) ? acceptedText : policies!.ToJsonString(); }
        finally { loading = false; }
        if (selection.SelectedIndex >= 0) ((ComboBoxItem)selection.Items[selection.SelectedIndex]).Content = Label(selection.SelectedIndex);
        Describe(); Changed?.Invoke();
    }
    private static JsonNode? InitialValue(JsonObject policy, string key) => key switch
    {
        "enabled" => JsonValue.Create(Enabled(policy)),
        "action" => JsonValue.Create(String(policy, key, "clean")),
        "contentTypes" => Array(ContentTypes(policy)),
        _ => JsonValue.Create(String(policy, key, "")),
    };
    private static JsonArray Array(IEnumerable<string> values) => new(values.Select(kind => (JsonNode?)JsonValue.Create(kind)).ToArray());
    private void SetField(string key, JsonNode? value)
    {
        if (loading || Selected is not { } policy) return;
        bool removeDefault = false;
        if (originals.TryGetValue(policy, out var initial))
        {
            var expected = InitialValue(initial, key);
            bool same = key == "action"
                ? Read<string>(expected).Equals(Read<string>(value), StringComparison.OrdinalIgnoreCase)
                : JsonNode.DeepEquals(expected, value);
            if (same)
            {
                removeDefault = !initial.ContainsKey(key);
                value = initial[key]?.DeepClone();
            }
        }
        if (removeDefault) { if (!policy.Remove(key)) return; }
        else
        {
            if (JsonNode.DeepEquals(policy[key], value)) return;
            policy[key] = value;
        }
        Publish();
    }
    private void ChangeType(int index)
    {
        if (loading || Selected is not { } policy) return;
        var types = ContentTypes(policy).ToList();
        types.RemoveAll(kind => kind.Equals(Types[index], StringComparison.OrdinalIgnoreCase));
        if (kinds[index].IsChecked == true) types.Add(Types[index]);
        if (originals.TryGetValue(policy, out var initial)
            && new HashSet<string>(types, StringComparer.OrdinalIgnoreCase).SetEquals(ContentTypes(initial)))
            SetField("contentTypes", Array(ContentTypes(initial)));
        else SetField("contentTypes", Array(types));
    }
    private void Add()
    {
        if (policies is null) return;
        policies.Add(new JsonObject { ["id"] = Guid.NewGuid().ToString("N"), ["appName"] = "", ["appPath"] = "",
            ["enabled"] = true, ["action"] = "clean", ["contentTypes"] = new JsonArray(Types.Select(kind => (JsonNode?)JsonValue.Create(kind)).ToArray()), ["cleanupRules"] = "" });
        RebuildSelection(Count - 1); Publish(); name.Focus(FocusState.Programmatic);
    }
    private void Remove()
    {
        if (Selected is null) return;
        int index = selection.SelectedIndex; originals.Remove(Selected!); policies!.RemoveAt(index);
        RebuildSelection(index); Publish();
    }
    private void Move(int delta)
    {
        if (Selected is not { } policy) return;
        int index = selection.SelectedIndex, target = index + delta;
        if (target < 0 || target >= Count) return;
        policies!.RemoveAt(index); policies.Insert(target, policy);
        RebuildSelection(target); Publish();
    }

    // Exercise actual input controls and the same click handlers as users.
    internal void Select(int index) => selection.SelectedIndex = index;
    internal void Edit(string appName, string appPath, bool isEnabled, bool ignore, string cleanupRules)
    { name.Text = appName; path.Text = appPath; enabled.IsOn = isEnabled; action.SelectedIndex = 0; rules.Text = cleanupRules; action.SelectedIndex = ignore ? 1 : 0; }
    internal void SetType(int index, bool value) => kinds[index].IsChecked = value;
    internal void Invoke(string command)
    {
        var button = command switch { "add" => add, "remove" => remove, "up" => up, "down" => down, _ => throw new ArgumentException(nameof(command)) };
        if (!button.IsEnabled) throw new InvalidOperationException("Policy action is disabled.");
        ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}

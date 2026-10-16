using System.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;

namespace TieZ.Native;

// The raw contract remains authoritative; untouched lines are never regenerated.
internal sealed class CleanupRuleEditor : UserControl
{
    private const string Ignore = "__IGNORE_CAPTURE__";
    private sealed record Line(string Body, string End);
    private sealed class Rule(List<Line> prefix, Line source, string pattern, string replacement)
    {
        internal readonly List<Line> Prefix = prefix;
        internal readonly Line Source = source;
        internal readonly string OriginalPattern = pattern, OriginalReplacement = replacement;
        internal string Pattern = pattern, Replacement = replacement == Ignore ? "" : replacement;
        internal bool Ignored = replacement == Ignore;
        internal readonly Line? LabelLine = prefix.LastOrDefault(IsLabel);
        internal int LabelIndex => Prefix.FindIndex(line => ReferenceEquals(line, LabelLine));
        internal string OriginalLabel => LabelLine is null ? "" : LabelLine.Body.Trim()[1..].TrimStart()[6..].Trim();
        internal string Label = "";
    }
    private readonly StackPanel root = new() { Spacing = 10 };
    private readonly StackPanel fields = new() { Spacing = 8 };
    private readonly ComboBox selection = new() { Header = "清理规则（按执行顺序）", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox pattern = new() { Header = "匹配表达式（Rust 正则）", AcceptsReturn = true };
    private readonly TextBox replacement = new() { Header = "替换内容", AcceptsReturn = true, PlaceholderText = @"支持 $1、${name} 和 \n、\r、\t" };
    private readonly TextBox label = new() { Header = "规则说明（可选）", AcceptsReturn = true };
    private readonly ComboBox action = new() { Header = "匹配后执行", ItemsSource = new[] { "替换文字", "忽略采集" } };
    private readonly Button add = new() { Content = "添加规则" }, remove = new() { Content = "删除规则" }, up = new() { Content = "上移" }, down = new() { Content = "下移" };
    private readonly InfoBar feedback = new() { Severity = InfoBarSeverity.Warning, IsClosable = false };
    private readonly TextBox raw = new() { Header = "原始规则文本", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 90 };
    private readonly List<Rule> rules = [];
    private readonly List<Line> footer = [];
    private bool loading, available = true;
    private string text = "", ending = "\n";
    internal event Action? Changed;
    internal string Text { get => text; set => LoadText(value); }
    internal string RawDraft { get => raw.Text; set => raw.Text = value; }
    internal int Count => rules.Count;
    internal bool StructuredAvailable => available;
    internal bool HasInputWarning => feedback.IsOpen;
    internal bool HasRetainedData => text.Length > 0 || raw.Text.Length > 0 || rules.Count > 0 || footer.Count > 0
        || pattern.Text.Length > 0 || replacement.Text.Length > 0 || label.Text.Length > 0;
    private Rule? Selected => selection.SelectedIndex >= 0 && selection.SelectedIndex < rules.Count ? rules[selection.SelectedIndex] : null;

    internal CleanupRuleEditor()
    {
        root.Children.Add(selection);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (var button in new[] { add, remove, up, down }) buttons.Children.Add(button);
        root.Children.Add(buttons); root.Children.Add(feedback);
        fields.Children.Add(label); fields.Children.Add(pattern); fields.Children.Add(action); fields.Children.Add(replacement);
        fields.Children.Add(new TextBlock { Text = @"清空结果或替换为 __IGNORE_CAPTURE__ 会忽略采集。正则由核心在测试和保存时校验；实际换行请写成 \n、\r，制表符写成 \t。", TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
        root.Children.Add(fields);
        root.Children.Add(new Expander { Header = "兼容配置 · 查看或编辑规则文本", Content = raw, HorizontalAlignment = HorizontalAlignment.Stretch });
        Content = root;
        selection.SelectionChanged += (_, _) => { if (!loading) ShowSelected(); };
        add.Click += (_, _) => AddRule(); remove.Click += (_, _) => RemoveRule();
        up.Click += (_, _) => MoveRule(-1); down.Click += (_, _) => MoveRule(1);
        pattern.BeforeTextChanging += (_, args) => CheckInput(args, true);
        replacement.BeforeTextChanging += (_, args) => CheckInput(args, false);
        label.BeforeTextChanging += (_, args) => CheckInput(args, false);
        pattern.TextChanging += (_, _) => EditSelected();
        replacement.TextChanging += (_, _) => EditSelected();
        label.TextChanging += (_, _) => EditSelected();
        action.SelectionChanged += (_, _) => EditSelected();
        raw.TextChanging += (_, _) =>
        {
            if (!loading && Lines(raw.Text) != Lines(text)) LoadText(raw.Text);
        };
        LoadText("");
    }
    private static bool IsLabel(Line line)
    {
        string body = line.Body.Trim();
        return body.StartsWith('#') && body[1..].TrimStart().StartsWith("label:", StringComparison.OrdinalIgnoreCase);
    }
    private static string Lines(string value) => value.Replace("\r\n", "\n").Replace("\r", "\n");
    private void CheckInput(TextBoxBeforeTextChangingEventArgs args, bool isPattern)
    {
        if (loading) return;
        bool invalid = args.NewText.Contains('\n') || args.NewText.Contains('\r')
            || (isPattern && (args.NewText.Contains("=>", StringComparison.Ordinal) || args.NewText.TrimStart().StartsWith('#')));
        if (invalid)
        {
            args.Cancel = true; feedback.IsOpen = true;
            feedback.Message = @"此文本无法安全写成一行规则。换行请使用 \n、\r；匹配 => 请将 > 写作 \x3E；开头的 # 请写作 \x23 或 [#]。";
        }
        else if (available) feedback.IsOpen = false;
    }
    private static List<Line> Split(string value)
    {
        var result = new List<Line>();
        int start = 0;
        for (int i = 0; i < value.Length; i++)
        {
            if (value[i] != '\n') continue;
            int end = i > start && value[i - 1] == '\r' ? i - 1 : i;
            result.Add(new Line(value[start..end], value[end..(i + 1)])); start = i + 1;
        }
        if (start < value.Length) result.Add(new Line(value[start..], ""));
        return result;
    }
    private void LoadText(string value)
    {
        bool changed = value != text;
        text = value; rules.Clear(); footer.Clear(); available = true;
        var lines = Split(value);
        ending = lines.FirstOrDefault(line => line.End.Length > 0)?.End ?? "\n";
        var prefix = new List<Line>();
        foreach (var line in lines)
        {
            string body = line.Body.Trim();
            if (body.Length == 0 || body.StartsWith('#')) { prefix.Add(line); continue; }
            int split = body.IndexOf("=>", StringComparison.Ordinal);
            if (split < 0 || body[..split].Trim().Length == 0 || body.Contains('\r')) { available = false; break; }
            var rule = new Rule(prefix, line, body[..split].Trim(), body[(split + 2)..].Trim());
            rule.Label = rule.OriginalLabel;
            rules.Add(rule); prefix = [];
        }
        if (available) footer.AddRange(prefix); else rules.Clear();
        loading = true;
        try { raw.Text = value; }
        finally { loading = false; }
        Rebuild(0);
        feedback.IsOpen = !available;
        feedback.Message = "原始文本包含无法逐条表示的规则，请在兼容配置中修正。原始草稿已完整保留。";
        if (changed) Changed?.Invoke();
    }
    private void Rebuild(int selected)
    {
        loading = true;
        try
        {
            selection.Items.Clear();
            for (int i = 0; i < rules.Count; i++) selection.Items.Add(new ComboBoxItem { Content = Summary(i) });
            selection.SelectedIndex = rules.Count == 0 ? -1 : Math.Clamp(selected, 0, rules.Count - 1);
        }
        finally { loading = false; }
        ShowSelected();
    }
    private string Summary(int index) => $"{index + 1}. {(rules[index].Label.Length > 0 ? rules[index].Label : rules[index].Pattern)}";
    private void ShowSelected()
    {
        loading = true;
        try
        {
            var rule = Selected;
            fields.Visibility = rule is null ? Visibility.Collapsed : Visibility.Visible;
            selection.IsEnabled = rules.Count > 0; add.IsEnabled = available; remove.IsEnabled = rule is not null;
            up.IsEnabled = rule is not null && selection.SelectedIndex > 0;
            down.IsEnabled = rule is not null && selection.SelectedIndex < rules.Count - 1;
            pattern.Text = rule?.Pattern ?? ""; replacement.Text = rule?.Replacement ?? ""; label.Text = rule?.Label ?? "";
            action.SelectedIndex = rule?.Ignored == true ? 1 : 0;
            replacement.IsEnabled = rule is not null && !rule.Ignored;
        }
        finally { loading = false; }
    }
    private void EditSelected()
    {
        if (loading || Selected is not { } rule) return;
        rule.Pattern = pattern.Text; rule.Replacement = replacement.Text; rule.Label = label.Text;
        rule.Ignored = action.SelectedIndex == 1; replacement.IsEnabled = !rule.Ignored;
        ((ComboBoxItem)selection.Items[selection.SelectedIndex]).Content = Summary(selection.SelectedIndex);
        Publish();
    }
    private IEnumerable<Line> Render(Rule rule)
    {
        for (int i = 0; i < rule.Prefix.Count; i++)
        {
            // Clearing a label must not reveal an older, shadowed label after reload.
            if (rule.Label.Trim().Length == 0 && rule.Label.Trim() != rule.OriginalLabel && IsLabel(rule.Prefix[i])) continue;
            if (i != rule.LabelIndex || rule.Label.Trim() == rule.OriginalLabel) yield return rule.Prefix[i];
            else if (rule.Label.Trim().Length > 0) yield return new Line("# label: " + rule.Label.Trim(), rule.Prefix[i].End);
        }
        if (rule.LabelIndex < 0 && rule.Label.Trim().Length > 0) yield return new Line("# label: " + rule.Label.Trim(), ending);
        string desired = rule.Ignored ? Ignore : rule.Replacement.Trim();
        yield return rule.Pattern.Trim() == rule.OriginalPattern && desired == rule.OriginalReplacement ? rule.Source
            : new Line(rule.Pattern.Trim() + " => " + desired, rule.Source.End);
    }
    private void Publish()
    {
        var lines = rules.SelectMany(Render).Concat(footer).ToArray();
        var output = new StringBuilder();
        for (int i = 0; i < lines.Length; i++)
            output.Append(lines[i].Body).Append(lines[i].End.Length == 0 && i < lines.Length - 1 ? ending : lines[i].End);
        string value = output.ToString();
        if (value == text) return;
        text = value; loading = true;
        try { raw.Text = value; }
        finally { loading = false; }
        Changed?.Invoke();
    }
    private void AddRule()
    {
        if (!available) return;
        var rule = new Rule([], new Line(" => ", ""), "", "");
        rules.Add(rule); Rebuild(rules.Count - 1); Publish(); pattern.Focus(FocusState.Programmatic);
    }
    private void RemoveRule()
    {
        if (Selected is not { } rule) return;
        int index = selection.SelectedIndex;
        // Delete this rule and its label declarations; retain unrelated comments.
        var comments = rule.Prefix.Where(line => !IsLabel(line)).ToList();
        rules.RemoveAt(index);
        if (index < rules.Count) rules[index].Prefix.InsertRange(0, comments); else footer.InsertRange(0, comments);
        Rebuild(index); Publish();
    }
    private void MoveRule(int delta)
    {
        if (Selected is not { } rule) return;
        int index = selection.SelectedIndex, target = index + delta;
        if (target < 0 || target >= rules.Count) return;
        rules.RemoveAt(index); rules.Insert(target, rule); Rebuild(target); Publish();
    }
    internal void Clear() { LoadText(""); feedback.IsOpen = false; }
    internal void Select(int index) => selection.SelectedIndex = index;
    internal void Edit(string expression, string value, string description)
    { pattern.Text = expression; replacement.Text = value; label.Text = description; }
    internal void SetIgnore(bool value) => action.SelectedIndex = value ? 1 : 0;
    internal string ReplacementDraft => replacement.Text;
    internal void Invoke(string command)
    {
        var button = command switch { "add" => add, "remove" => remove, "up" => up, "down" => down, _ => throw new ArgumentException(nameof(command)) };
        if (button.IsEnabled) ((IInvokeProvider)new ButtonAutomationPeer(button).GetPattern(PatternInterface.Invoke)).Invoke();
    }
}

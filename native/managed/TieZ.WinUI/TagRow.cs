using System.Globalization;
using System.Reflection;
using Microsoft.UI;
using Microsoft.UI.Xaml.Media;
using TieZ.Core;
using Color = Windows.UI.Color;
using Colors = Microsoft.UI.Colors;

namespace TieZ.Native;

public sealed class TagRow(TagInfo info)
{
    internal TagInfo Info => info;
    public string Label => info.Name.Length == 0 ? "（未命名标签）" : info.Name;
    public string Details => $"{info.Count} 条 · 已保存 {info.StoredCount} · 本次会话 {info.SessionCount}" + (info.IsSensitive ? " · 敏感内容" : "");
    public SolidColorBrush Swatch => new(NativeTagColor.TryParse(info.Color, out var color) ? color : Colors.Transparent);
}
internal static class NativeTagColor
{
    private static readonly Dictionary<string, Color> Names = typeof(Colors).GetProperties(BindingFlags.Public | BindingFlags.Static)
        .Where(property => property.PropertyType == typeof(Color))
        .ToDictionary(property => property.Name, property => (Color)property.GetValue(null)!, StringComparer.OrdinalIgnoreCase);
    internal static string Css(Color color) => color.A == 255 ? $"#{color.R:x2}{color.G:x2}{color.B:x2}" : $"#{color.R:x2}{color.G:x2}{color.B:x2}{color.A:x2}";
    internal static bool TryParse(string? text, out Color color)
    {
        color = Colors.Transparent;
        if (string.IsNullOrWhiteSpace(text)) return false;
        text = text.Trim();
        if (Names.TryGetValue(text, out color)) return true;
        if (text.StartsWith('#'))
        {
            string hex = text[1..];
            if (hex.Length is 3 or 4) hex = string.Concat(hex.Select(value => new string(value, 2)));
            if (hex.Length is not (6 or 8) || !uint.TryParse(hex, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out uint value)) return false;
            color = hex.Length == 6 ? ColorHelper.FromArgb(255, (byte)(value >> 16), (byte)(value >> 8), (byte)value)
                : ColorHelper.FromArgb((byte)value, (byte)(value >> 24), (byte)(value >> 16), (byte)(value >> 8));
            return true;
        }
        int open = text.IndexOf('(');
        if (open < 0 || !text.EndsWith(')')) return false;
        string function = text[..open].Trim();
        bool alpha = function.Equals("rgba", StringComparison.OrdinalIgnoreCase);
        if (!alpha && !function.Equals("rgb", StringComparison.OrdinalIgnoreCase)) return false;
        string[] values = text[(open + 1)..^1].Split(',');
        if (values.Length != (alpha ? 4 : 3)) return false;
        var bytes = new byte[] { 0, 0, 0, 255 };
        for (int i = 0; i < values.Length; i++)
        {
            string component = values[i].Trim(); bool percent = component.EndsWith('%');
            if (percent) component = component[..^1];
            if (!double.TryParse(component, NumberStyles.Float, CultureInfo.InvariantCulture, out double number) || !double.IsFinite(number)) return false;
            double maximum = percent ? 100 : i == 3 ? 1 : 255;
            bytes[i] = (byte)Math.Round(Math.Clamp(number / maximum, 0, 1) * 255, MidpointRounding.AwayFromZero);
        }
        color = ColorHelper.FromArgb(bytes[3], bytes[0], bytes[1], bytes[2]); return true;
    }
}

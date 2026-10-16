using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Windowing;
using TieZ.Core;
using Windows.Graphics;

namespace TieZ.Native;

internal sealed class CompactPreviewWindow : Window
{
    private readonly Grid root = new() { RowSpacing = 8 };
    private readonly TextBlock header = new() { FontSize = 16, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
    private readonly TextBlock metadata = new() { FontSize = 12, TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
    private readonly TextBox text = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap };
    private readonly ImagePreviewView image = new();
    private readonly DocumentPreviewView document = new();
    private readonly nint hwnd;
    private readonly SubclassProc subclass;
    private bool closed;
    internal long? DisplayedId { get; private set; }
    internal bool HasImage => image.HasImage;
    internal bool HasDocument => document.HasDocument;
    internal string Text => text.Text;
    internal ElementTheme CurrentTheme { get => root.RequestedTheme; set => root.RequestedTheme = value; }
    internal nint FocusBeforeShow { get; private set; }
    internal nint FocusAfterShow { get; private set; }
    internal bool ShowPreservedForeground => FocusBeforeShow != 0 && FocusAfterShow == FocusBeforeShow;
    internal bool IsNonActivating => (GetWindowLongPtrW(hwnd, -20).ToInt64() & 0x08000080) == 0x08000080;

    internal CompactPreviewWindow(MainWindow owner)
    {
        Title = "TieZ · 紧凑预览";
        hwnd = WinRT.Interop.WindowNative.GetWindowHandle(this);
        subclass = WindowProc;
        try
        {
            root.RowDefinitions.Add(new() { Height = GridLength.Auto });
            root.RowDefinitions.Add(new() { Height = GridLength.Auto });
            root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
            root.Children.Add(header); root.Children.Add(metadata); Grid.SetRow(metadata, 1);
            var body = new Grid(); body.Children.Add(text); body.Children.Add(image); body.Children.Add(document);
            root.Children.Add(body); Grid.SetRow(body, 2);
            Content = new Border { Child = root, IsHitTestVisible = false,
                Style = (Style)Application.Current.Resources["NativeCompactPreviewStyle"] };
            AppWindow.IsShownInSwitchers = false;
            var presenter = OverlappedPresenter.Create();
            presenter.IsResizable = presenter.IsMaximizable = presenter.IsMinimizable = false;
            presenter.IsAlwaysOnTop = true; presenter.SetBorderAndTitleBar(false, false);
            AppWindow.SetPresenter(presenter);
            SetLong(-8, WinRT.Interop.WindowNative.GetWindowHandle(owner));
            SetLong(-20, (nint)(GetWindowLongPtrW(hwnd, -20).ToInt64() | 0x08000080));
            if (!SetWindowSubclass(hwnd, subclass, 1, 0)) throw new InvalidOperationException("Could not install native preview hit-test guard.");
            Closed += (_, _) => { closed = true; Clear(); RemoveWindowSubclass(hwnd, subclass, 1); };
        }
        catch { Close(); throw; }
    }
    private void SetLong(int index, nint value)
    {
        Marshal.SetLastPInvokeError(0);
        if (SetWindowLongPtrW(hwnd, index, value) == 0 && Marshal.GetLastPInvokeError() != 0)
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastPInvokeError());
    }
    internal async Task PrepareAsync(ClipboardEntry entry, NativeCore core, ulong revision, bool allowSensitive, CancellationToken token)
    {
        Clear(); token.ThrowIfCancellationRequested();
        header.Text = entry.ContentType;
        metadata.Text = string.Join(" · ", new[] { entry.SourceApp, entry.Id < 0 ? "仅本次会话" : "已保存" }.Where(s => !string.IsNullOrEmpty(s)));
        if (entry.ContentType == "image")
        {
            var preview = await core.GetImagePreviewAsync(entry.Id, revision, allowSensitive, cancellationToken: token);
            token.ThrowIfCancellationRequested();
            await image.ShowAsync(preview, token);
        }
        else if (entry.ContentType == "rich_text" && entry.HtmlContent is not null)
        {
            var preview = await core.GetDocumentPreviewAsync(entry.Id, revision, allowSensitive, token);
            token.ThrowIfCancellationRequested();
            document.Show(preview, entry.Content);
        }
        else text.Text = entry.Content;
        token.ThrowIfCancellationRequested();
        if (closed) throw new OperationCanceledException(token);
        text.Visibility = image.HasImage || document.HasDocument ? Visibility.Collapsed : Visibility.Visible;
        DisplayedId = entry.Id;
    }
    internal void ShowAt(RectInt32 bounds)
    {
        if (closed) return;
        AppWindow.MoveAndResize(bounds);
        FocusBeforeShow = GetForegroundWindow();
        AppWindow.Show(activateWindow: false);
        FocusAfterShow = GetForegroundWindow();
    }
    internal void HideAndClear()
    {
        if (closed) return;
        AppWindow.Hide(); Clear();
    }
    private void Clear()
    {
        DisplayedId = null; header.Text = metadata.Text = text.Text = "";
        image.Clear(); document.Clear(); text.Visibility = Visibility.Visible;
    }
    private nint WindowProc(nint window, uint message, nuint wparam, nint lparam, nuint id, nuint data) =>
        message switch { 0x0084 => -1, 0x0021 => 3, _ => DefSubclassProc(window, message, wparam, lparam) };
    private delegate nint SubclassProc(nint window, uint message, nuint wparam, nint lparam, nuint id, nuint data);
    [DllImport("user32.dll")] private static extern nint GetForegroundWindow();
    [DllImport("user32.dll", SetLastError = true)] private static extern nint SetWindowLongPtrW(nint window, int index, nint value);
    [DllImport("user32.dll")] private static extern nint GetWindowLongPtrW(nint window, int index);
    [DllImport("comctl32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetWindowSubclass(nint window, SubclassProc callback, nuint id, nuint data);
    [DllImport("comctl32.dll")] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool RemoveWindowSubclass(nint window, SubclassProc callback, nuint id);
    [DllImport("comctl32.dll")] private static extern nint DefSubclassProc(nint window, uint message, nuint wparam, nint lparam);
}

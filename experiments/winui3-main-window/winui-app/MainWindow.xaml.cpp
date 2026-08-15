#include "pch.h"
#include "MainWindow.xaml.h"

#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif

#include <microsoft.ui.xaml.window.h>
#include <commctrl.h>

#pragma comment(lib, "comctl32.lib")

namespace
{
    constexpr UINT kToggleHotkeyId = 1;

    winrt::hstring StatusMessage(
        std::wstring_view prefix,
        winrt::hstring const& detail)
    {
        std::wstring message{ prefix };
        message.append(detail.c_str(), detail.size());
        return winrt::hstring{ message };
    }

    winrt::hstring AdapterLabel(winrt::hstring const& adapter)
    {
        if (adapter == L"memory")
        {
            return L"内存";
        }
        if (adapter == L"sqlite-read-only")
        {
            return L"SQLite 只读";
        }
        if (adapter == L"sqlite")
        {
            return L"SQLite";
        }
        return adapter;
    }

    winrt::hstring ContentTypeLabel(winrt::hstring const& contentType)
    {
        if (contentType == L"text") return L"文本";
        if (contentType == L"image") return L"图片";
        if (contentType == L"url") return L"链接";
        if (contentType == L"code") return L"代码";
        if (contentType == L"file" || contentType == L"files") return L"文件";
        if (contentType == L"html") return L"富文本";
        return contentType;
    }

    winrt::hstring CapturedAtLabel(winrt::hstring const& capturedAt)
    {
        if (capturedAt == L"Just now")
        {
            return L"刚刚";
        }

        std::wistringstream stream{ std::wstring{ capturedAt.c_str() } };
        std::uint64_t amount{};
        std::wstring unit;
        std::wstring ago;
        if (stream >> amount >> unit >> ago && ago == L"ago")
        {
            std::wstringstream label;
            label << amount;
            if (unit == L"minute" || unit == L"minutes") label << L" 分钟前";
            else if (unit == L"hour" || unit == L"hours") label << L" 小时前";
            else if (unit == L"day" || unit == L"days") label << L" 天前";
            else return capturedAt;
            return winrt::hstring{ label.str() };
        }
        return capturedAt;
    }

    winrt::hstring ActionStatus(std::string_view action)
    {
        if (action == "pin") return L"置顶状态已更新";
        if (action == "delete") return L"记录已删除";
        if (action == "paste-plain") return L"已执行纯文本粘贴";
        if (action == "paste-rich") return L"已执行富文本粘贴";
        if (action == "copy-plain") return L"已复制到剪贴板";
        return L"操作已完成";
    }

    winrt::Microsoft::UI::Xaml::Controls::Button ActionButton(
        winrt::hstring const& label,
        std::function<void()> action)
    {
        winrt::Microsoft::UI::Xaml::Controls::Button button;
        button.Content(winrt::box_value(label));
        button.Padding(winrt::Microsoft::UI::Xaml::ThicknessHelper::FromLengths(6, 5, 6, 5));
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(button, label);
        button.Click([action = std::move(action)](auto const&, auto const&)
        {
            action();
        });
        return button;
    }

    winrt::Microsoft::UI::Xaml::Controls::MenuFlyoutItem CommandItem(
        winrt::hstring const& label,
        bool enabled,
        std::function<void()> action)
    {
        winrt::Microsoft::UI::Xaml::Controls::MenuFlyoutItem item;
        item.Text(label);
        item.IsEnabled(enabled);
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(item, label);
        item.Click([action = std::move(action)](auto const&, auto const&)
        {
            action();
        });
        return item;
    }

    LRESULT CALLBACK HotkeySubclassProc(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR,
        DWORD_PTR refData)
    {
        if (message == WM_HOTKEY && wParam == kToggleHotkeyId)
        {
            auto* window = reinterpret_cast<winrt::Tiez::WinUIProbe::implementation::MainWindow*>(refData);
            window->OnToggleHotkey();
            return 0;
        }

        return DefSubclassProc(hwnd, message, wParam, lParam);
    }
}

namespace winrt::Tiez::WinUIProbe::implementation
{
    using namespace Microsoft::UI::Xaml;
    using namespace Microsoft::UI::Xaml::Automation;
    using namespace Microsoft::UI::Xaml::Controls;
    using namespace Microsoft::UI::Xaml::Controls::Primitives;
    using namespace Microsoft::UI::Xaml::Input;
    using namespace Microsoft::UI::Xaml::Media;
    using namespace Windows::Data::Json;
    using winrt::Windows::Foundation::IInspectable;
    using Windows::System::VirtualKey;

    MainWindow::MainWindow()
    {
        InitializeComponent();
        Title(L"TieZ · WinUI 3 原生主窗口实验");
        SetInitialWindowSize();
        SetupLifecycle();
        SetupImeGuards();
        SearchBox().Focus(FocusState::Programmatic);

        try
        {
            m_refreshSink = std::make_shared<HistoryRefreshSink>();
            m_refreshSink->window = this;
            m_refreshSink->dispatcher = DispatcherQueue();
            m_core = std::make_unique<tiez::probe::RustCoreBridge>();
            m_core->SetChangedCallback(&MainWindow::OnHistoryChanged, m_refreshSink.get());
            m_core->StartCapture();
            RefreshItems();
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"启动失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    MainWindow::~MainWindow()
    {
        if (m_refreshSink)
        {
            std::lock_guard<std::mutex> guard(m_refreshSink->mutex);
            m_refreshSink->window = nullptr;
        }
        if (m_core)
        {
            m_core->SetChangedCallback(nullptr, nullptr);
            m_core.reset();
        }
        TeardownLifecycle();
    }

    void MainWindow::OnHistoryChanged(void* userData, std::uint64_t)
    {
        auto* raw = static_cast<HistoryRefreshSink*>(userData);
        if (raw == nullptr)
        {
            return;
        }

        std::shared_ptr<HistoryRefreshSink> sink;
        try
        {
            sink = raw->shared_from_this();
        }
        catch (std::bad_weak_ptr const&)
        {
            return;
        }

        Microsoft::UI::Dispatching::DispatcherQueue dispatcher{ nullptr };
        {
            std::lock_guard<std::mutex> guard(sink->mutex);
            if (sink->window == nullptr)
            {
                return;
            }
            dispatcher = sink->dispatcher;
        }
        if (!dispatcher)
        {
            return;
        }

        dispatcher.TryEnqueue([sink]()
        {
            std::lock_guard<std::mutex> guard(sink->mutex);
            if (sink->window != nullptr)
            {
                sink->window->RefreshItems();
            }
        });
    }

    void MainWindow::SearchBox_TextChanged(IInspectable const&, TextChangedEventArgs const&)
    {
        if (m_core)
        {
            RefreshItems();
        }
    }

    void MainWindow::SearchBox_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (HandleNavigationKey(args.Key()))
        {
            args.Handled(true);
        }
    }

    void MainWindow::RootGrid_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (HandleNavigationKey(args.Key()))
        {
            args.Handled(true);
        }
    }

    void MainWindow::RefreshButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        RefreshItems();
    }

    void MainWindow::HideButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (!m_showTimer)
        {
            m_showTimer = DispatcherQueue().CreateTimer();
            m_showTimer.Interval(std::chrono::seconds(5));
            m_showTimer.IsRepeating(false);
            m_showTimer.Tick([this](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
                ShowWindow(GetWindowHandle(), SW_SHOW);
                Activate();
                SetStatus(L"窗口已在 5 秒后恢复，Rust 核心始终保持在当前进程中。");
            });
        }

        m_suspendLifecycle = true;
        SetStatus(L"窗口将隐藏 5 秒，可在此期间采样进程并比较空闲内存。");
        m_showTimer.Start();
        ShowWindow(GetWindowHandle(), SW_HIDE);
    }

    void MainWindow::PinWindowCheck_Changed(IInspectable const&, RoutedEventArgs const&)
    {
        m_pinned = PinWindowCheck().IsChecked().Value();
        SetStatus(m_pinned
            ? L"窗口已固定，失去焦点时不会隐藏。"
            : L"窗口已取消固定，失去焦点时将自动隐藏。");
    }

    void MainWindow::TypeAllButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        SetTypeFilter({});
    }

    void MainWindow::TypeChip_Click(IInspectable const& sender, RoutedEventArgs const&)
    {
        auto const button = sender.as<ToggleButton>();
        auto const tag = winrt::unbox_value_or<winrt::hstring>(button.Tag(), L"");
        auto const filter = winrt::to_string(tag);
        if (!button.IsChecked().Value())
        {
            SetTypeFilter({});
            return;
        }
        SetTypeFilter(filter);
    }

    void MainWindow::OnToggleHotkey()
    {
        if (IsWindowVisible(GetWindowHandle()))
        {
            HideMainWindow();
            return;
        }
        ShowMainWindow(true);
    }

    void MainWindow::RefreshItems()
    {
        if (!m_core)
        {
            return;
        }

        try
        {
            auto const snapshot = m_core->Snapshot(CurrentQuery());
            auto const root = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(snapshot));
            auto const items = root.GetNamedArray(L"items");
            auto const adapter = root.GetNamedString(L"adapter");
            auto const readOnly = root.GetNamedBoolean(L"read_only");

            AdapterText().Text(AdapterLabel(adapter));
            if (readOnly)
            {
                ReadOnlyText().Text(L"真实 TieZ 历史 · 操作已禁用");
            }
            else if (adapter == L"sqlite")
            {
                ReadOnlyText().Text(L"真实 TieZ 历史 · 可写");
            }
            else
            {
                ReadOnlyText().Text(L"示例数据 · 操作已启用");
            }

            auto const previousId = (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
                ? std::optional<std::int64_t>{ m_entryIds[static_cast<std::size_t>(m_selectedIndex)] }
                : std::nullopt;

            ItemsPanel().Children().Clear();
            m_entryIds.clear();
            m_cards.clear();
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                auto const item = items.GetObjectAt(index);
                m_entryIds.push_back(static_cast<std::int64_t>(item.GetNamedNumber(L"id")));
                auto const card = CreateItemCard(item, readOnly, index);
                ItemsPanel().Children().Append(card);
            }

            m_selectedIndex = -1;
            if (previousId)
            {
                for (std::size_t index = 0; index < m_entryIds.size(); ++index)
                {
                    if (m_entryIds[index] == *previousId)
                    {
                        m_selectedIndex = static_cast<int>(index);
                        break;
                    }
                }
            }
            if (m_selectedIndex < 0 && !m_entryIds.empty())
            {
                m_selectedIndex = 0;
            }
            UpdateSelectionVisuals();

            EmptyState().Visibility(items.Size() == 0 ? Visibility::Visible : Visibility::Collapsed);

            std::wstringstream status;
            status << AdapterLabel(adapter).c_str()
                   << (readOnly ? L" · 只读 · " : L" · 可写 · ")
                   << L"Rust ABI " << static_cast<std::uint32_t>(root.GetNamedNumber(L"abi_version"))
                   << L" · 第 " << static_cast<std::uint64_t>(root.GetNamedNumber(L"generation"))
                   << L" 代 · " << items.Size() << L" 条可见记录 · 适配器已就绪";
            SetStatus(winrt::hstring{ status.str() });
            WriteReadyMarker();
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"刷新失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"刷新失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    UIElement MainWindow::CreateItemCard(JsonObject const& item, bool readOnly, std::uint32_t index)
    {
        auto const entryId = static_cast<std::int64_t>(item.GetNamedNumber(L"id"));
        auto const isPinned = item.GetNamedBoolean(L"is_pinned");
        auto const isSensitive = item.GetNamedBoolean(L"is_sensitive");

        Border card;
        card.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"ClipboardCardStyle")).as<Style>());
        card.PointerPressed([this, entryId, index](auto const&, auto const&)
        {
            m_selectedIndex = static_cast<int>(index);
            UpdateSelectionVisuals();
            ShowContent(entryId);
        });
        card.DoubleTapped([this, entryId, readOnly](auto const&, auto const&)
        {
            if (!readOnly)
            {
                ApplyAction(entryId, "paste-plain");
            }
        });
        AutomationProperties::SetName(card, item.GetNamedString(L"preview"));
        AttachCardCommands(card, entryId, readOnly);
        m_cards.push_back(card);

        StackPanel content;
        content.Spacing(10);

        Grid metadata;
        metadata.ColumnSpacing(12);
        ColumnDefinition typeColumn;
        typeColumn.Width(GridLengthHelper::Auto());
        ColumnDefinition sourceColumn;
        sourceColumn.Width(GridLength{ 1, GridUnitType::Star });
        ColumnDefinition timeColumn;
        timeColumn.Width(GridLengthHelper::Auto());
        metadata.ColumnDefinitions().Append(typeColumn);
        metadata.ColumnDefinitions().Append(sourceColumn);
        metadata.ColumnDefinitions().Append(timeColumn);

        TextBlock type;
        type.Text(ContentTypeLabel(item.GetNamedString(L"content_type")));
        type.FontWeight(Windows::UI::Text::FontWeights::SemiBold());

        TextBlock source;
        source.Text(item.GetNamedString(L"source_app"));
        source.Opacity(0.72);
        Grid::SetColumn(source, 1);

        TextBlock capturedAt;
        capturedAt.Text(CapturedAtLabel(item.GetNamedString(L"captured_at")));
        capturedAt.Opacity(0.72);
        Grid::SetColumn(capturedAt, 2);

        metadata.Children().Append(type);
        metadata.Children().Append(source);
        metadata.Children().Append(capturedAt);

        if (isSensitive)
        {
            TextBlock sensitive;
            sensitive.Text(L"敏感内容 · 预览已隐藏");
            sensitive.Foreground(SolidColorBrush{ Windows::UI::Color{ 255, 196, 43, 28 } });
            content.Children().Append(sensitive);
        }

        TextBlock preview;
        preview.Text(item.GetNamedString(L"preview"));
        preview.TextWrapping(TextWrapping::WrapWholeWords);
        preview.IsTextSelectionEnabled(true);
        preview.MaxHeight(112);

        StackPanel actions;
        actions.Orientation(Orientation::Horizontal);
        actions.Spacing(8);
        auto openButton = ActionButton(
            L"查看详情",
            [this, entryId, index]
            {
                m_selectedIndex = static_cast<int>(index);
                UpdateSelectionVisuals();
                ShowContent(entryId);
            });
        auto pinButton = ActionButton(
            isPinned ? L"取消置顶" : L"置顶",
            [this, entryId] { ApplyAction(entryId, "pin"); });
        auto pastePlainButton = ActionButton(
            L"纯文本粘贴",
            [this, entryId] { ApplyAction(entryId, "paste-plain"); });
        auto pasteRichButton = ActionButton(
            L"富文本粘贴",
            [this, entryId] { ApplyAction(entryId, "paste-rich"); });
        auto copyButton = ActionButton(
            L"复制",
            [this, entryId] { ApplyAction(entryId, "copy-plain"); });
        auto deleteButton = ActionButton(
            L"删除",
            [this, entryId] { ApplyAction(entryId, "delete"); });

        pinButton.IsEnabled(!readOnly);
        pastePlainButton.IsEnabled(!readOnly);
        pasteRichButton.IsEnabled(!readOnly);
        copyButton.IsEnabled(!readOnly);
        deleteButton.IsEnabled(!readOnly);

        actions.Children().Append(openButton);
        actions.Children().Append(pinButton);
        actions.Children().Append(pastePlainButton);
        actions.Children().Append(pasteRichButton);
        actions.Children().Append(copyButton);
        actions.Children().Append(deleteButton);

        content.Children().Append(metadata);
        content.Children().Append(preview);
        content.Children().Append(actions);
        card.Child(content);
        return card;
    }

    void MainWindow::ShowContent(std::int64_t entryId)
    {
        if (!m_core)
        {
            return;
        }

        try
        {
            auto const value = m_core->Content(entryId);
            auto const content = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const contentType = content.GetNamedString(L"content_type");
            auto const available = content.GetNamedBoolean(L"available");
            auto const isSensitive = content.GetNamedBoolean(L"is_sensitive");

            std::wstringstream title;
            title << L"记录 " << entryId;
            DetailsTitleText().Text(winrt::hstring{ title.str() });

            std::wstring metadata{ ContentTypeLabel(contentType).c_str() };
            metadata.append(isSensitive ? L" · 敏感内容" : L" · 内容可用");
            DetailsMetadataText().Text(metadata);

            if (available && !isSensitive)
            {
                auto displayContent = content.GetNamedString(L"content");
                if (displayContent.empty())
                {
                    auto const htmlContent = content.GetNamedValue(L"html_content");
                    if (htmlContent.ValueType() == JsonValueType::String)
                    {
                        displayContent = htmlContent.GetString();
                    }
                }

                DetailsContentText().Text(displayContent);
                ShowDetailsImage(contentType, displayContent);
                SetStatus(L"已从独立于 Tauri 的 Rust 核心加载完整内容。");
            }
            else
            {
                DetailsContentText().Text(isSensitive
                    ? L"此内容受隐私保护，无法显示。"
                    : L"此内容当前不可用。");
                ShowDetailsImage(contentType, {});
                SetStatus(L"已加载内容元数据，原始内容仍受保护。");
            }
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"查询内容失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"查询内容失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::ApplyAction(std::int64_t entryId, std::string_view action)
    {
        auto const isPaste = action == "paste-plain" || action == "paste-rich";
        if (isPaste)
        {
            PreparePasteTarget();
        }

        try
        {
            auto const value = m_core->ApplyAction(entryId, action);
            auto const mutation = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            RefreshItems();

            std::wstringstream status;
            status << ActionStatus(action).c_str()
                   << L" · 第 "
                   << static_cast<std::uint64_t>(mutation.GetNamedNumber(L"generation"));
            auto const replacement = mutation.GetNamedValue(L"replacement_id");
            if (replacement.ValueType() == JsonValueType::Number)
            {
                status << L" 代 · 替换记录 ID "
                       << static_cast<std::int64_t>(replacement.GetNumber());
            }
            else
            {
                status << L" 代";
            }
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"操作失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }

        m_suspendLifecycle = false;
    }

    void MainWindow::SetStatus(winrt::hstring const& message)
    {
        StatusText().Text(message);
    }

    void MainWindow::WriteReadyMarker()
    {
        if (m_readyMarkerWritten)
        {
            return;
        }

        wchar_t path[32768]{};
        auto const length = GetEnvironmentVariableW(
            L"TIEZ_WINUI_READY_FILE",
            path,
            static_cast<DWORD>(std::size(path)));
        if (length > 0 && length < std::size(path))
        {
            std::ofstream marker{ std::filesystem::path{ path }, std::ios::trunc };
            marker << "pid=" << GetCurrentProcessId() << '\n';
            marker << "ready_tick_ms=" << GetTickCount64() << '\n';
            marker << "abi_version=" << m_core->AbiVersion() << '\n';
        }

        m_readyMarkerWritten = true;
    }

    HWND MainWindow::GetWindowHandle()
    {
        if (m_hwnd == nullptr)
        {
            Window window = *this;
            winrt::check_hresult(window.as<::IWindowNative>()->get_WindowHandle(&m_hwnd));
        }
        return m_hwnd;
    }

    void MainWindow::SetInitialWindowSize()
    {
        auto const hwnd = GetWindowHandle();
        auto const dpiScale = static_cast<float>(GetDpiForWindow(hwnd)) / 96.0F;
        auto const width = static_cast<int>(900 * dpiScale);
        auto const height = static_cast<int>(760 * dpiScale);
        SetWindowPos(hwnd, nullptr, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER);
    }

    void MainWindow::SetupLifecycle()
    {
        Activated([this](auto const&, WindowActivatedEventArgs const& args)
        {
            if (m_suspendLifecycle)
            {
                return;
            }
            if (args.WindowActivationState() == WindowActivationState::Deactivated
                && !m_pinned
                && IsWindowVisible(GetWindowHandle()))
            {
                HideMainWindow();
            }
        });
        Closed([this](auto const&, auto const&)
        {
            TeardownLifecycle();
        });

        m_hotkeyHwnd = CreateWindowExW(
            0,
            L"STATIC",
            L"TiezWinUIProbeHotkey",
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            nullptr,
            GetModuleHandleW(nullptr),
            nullptr);
        if (m_hotkeyHwnd == nullptr)
        {
            SetStatus(L"无法创建全局快捷键消息窗口。");
            return;
        }

        SetWindowSubclass(
            m_hotkeyHwnd,
            HotkeySubclassProc,
            1,
            reinterpret_cast<DWORD_PTR>(this));
        if (!RegisterHotKey(m_hotkeyHwnd, kToggleHotkeyId, MOD_ALT | MOD_NOREPEAT, 0x43))
        {
            SetStatus(L"Alt+C 已被其他程序占用，可按 Esc 隐藏并通过任务栏重新显示。");
        }
    }

    void MainWindow::TeardownLifecycle()
    {
        if (m_hotkeyHwnd != nullptr)
        {
            UnregisterHotKey(m_hotkeyHwnd, kToggleHotkeyId);
            RemoveWindowSubclass(m_hotkeyHwnd, HotkeySubclassProc, 1);
            DestroyWindow(m_hotkeyHwnd);
            m_hotkeyHwnd = nullptr;
        }
    }

    void MainWindow::HideMainWindow()
    {
        ShowWindow(GetWindowHandle(), SW_HIDE);
        SetStatus(L"窗口已隐藏，按 Alt+C 可重新显示。");
    }

    void MainWindow::ShowMainWindow(bool captureForeground)
    {
        if (captureForeground)
        {
            auto const foreground = GetForegroundWindow();
            if (foreground != nullptr && foreground != GetWindowHandle())
            {
                m_lastHwnd = foreground;
            }
        }
        ShowWindow(GetWindowHandle(), SW_SHOW);
        Activate();
        SearchBox().Focus(FocusState::Programmatic);
        SetStatus(L"已通过 Alt+C 显示窗口，并记录粘贴目标窗口。");
    }

    void MainWindow::PreparePasteTarget()
    {
        m_suspendLifecycle = true;
        if (!m_pinned)
        {
            ShowWindow(GetWindowHandle(), SW_HIDE);
        }
        if (m_lastHwnd != nullptr && m_lastHwnd != GetWindowHandle())
        {
            SetForegroundWindow(m_lastHwnd);
            Sleep(50);
        }
    }

    bool MainWindow::HandleNavigationKey(VirtualKey key)
    {
        if (key == VirtualKey::Escape)
        {
            HideMainWindow();
            return true;
        }
        if (key == VirtualKey::Down)
        {
            MoveSelection(1);
            return true;
        }
        if (key == VirtualKey::Up)
        {
            MoveSelection(-1);
            return true;
        }
        if (key == VirtualKey::Enter)
        {
            if (m_imeComposing || m_ignoreNextEnter || (GetKeyState(VK_PROCESSKEY) & 0x8000))
            {
                m_ignoreNextEnter = false;
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                auto const rich = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
                ApplyAction(
                    m_entryIds[static_cast<std::size_t>(m_selectedIndex)],
                    rich ? "paste-rich" : "paste-plain");
            }
            return true;
        }
        if (key == VirtualKey::Delete)
        {
            if (SearchBoxHasFocus())
            {
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                ApplyAction(m_entryIds[static_cast<std::size_t>(m_selectedIndex)], "delete");
            }
            return true;
        }
        if (key == VirtualKey::C && (GetKeyState(VK_CONTROL) & 0x8000))
        {
            if (SearchBoxHasFocus() && !SearchBox().SelectedText().empty())
            {
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                ApplyAction(m_entryIds[static_cast<std::size_t>(m_selectedIndex)], "copy-plain");
            }
            return true;
        }
        return false;
    }

    bool MainWindow::SearchBoxHasFocus()
    {
        return SearchBox().FocusState() != FocusState::Unfocused;
    }

    void MainWindow::AttachCardCommands(
        Border const& card,
        std::int64_t entryId,
        bool readOnly)
    {
        MenuFlyout flyout;
        flyout.Items().Append(CommandItem(
            L"纯文本粘贴",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "paste-plain"); }));
        flyout.Items().Append(CommandItem(
            L"富文本粘贴",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "paste-rich"); }));
        flyout.Items().Append(CommandItem(
            L"复制",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "copy-plain"); }));
        flyout.Items().Append(CommandItem(
            L"置顶 / 取消置顶",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "pin"); }));
        flyout.Items().Append(CommandItem(
            L"删除",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "delete"); }));
        flyout.Items().Append(CommandItem(
            L"查看详情",
            true,
            [this, entryId] { ShowContent(entryId); }));
        card.ContextFlyout(flyout);
    }

    void MainWindow::MoveSelection(int delta)
    {
        if (m_entryIds.empty())
        {
            return;
        }
        auto const count = static_cast<int>(m_entryIds.size());
        if (m_selectedIndex < 0)
        {
            m_selectedIndex = delta > 0 ? 0 : count - 1;
        }
        else
        {
            m_selectedIndex = (m_selectedIndex + delta + count) % count;
        }
        UpdateSelectionVisuals();
        ShowContent(m_entryIds[static_cast<std::size_t>(m_selectedIndex)]);
    }

    void MainWindow::UpdateSelectionVisuals()
    {
        for (std::size_t index = 0; index < m_cards.size(); ++index)
        {
            auto const selected = static_cast<int>(index) == m_selectedIndex;
            m_cards[index].BorderThickness(ThicknessHelper::FromUniformLength(selected ? 2.0 : 1.0));
            if (selected)
            {
                m_cards[index].BorderBrush(SolidColorBrush{ Windows::UI::Color{ 255, 0, 120, 212 } });
                m_cards[index].StartBringIntoView();
            }
            else
            {
                m_cards[index].BorderBrush(SolidColorBrush{ Windows::UI::Color{ 255, 200, 200, 200 } });
            }
        }
    }

    std::string MainWindow::CurrentQuery()
    {
        auto text = winrt::to_string(SearchBox().Text());
        if (m_typeFilter.empty())
        {
            return text;
        }
        if (text.empty())
        {
            return "type:" + m_typeFilter;
        }
        return "type:" + m_typeFilter + " " + text;
    }

    void MainWindow::SetTypeFilter(std::string filter)
    {
        m_typeFilter = std::move(filter);
        TypeAllButton().IsChecked(m_typeFilter.empty());
        TypeTextButton().IsChecked(m_typeFilter == "text");
        TypeImageButton().IsChecked(m_typeFilter == "image");
        TypeUrlButton().IsChecked(m_typeFilter == "url");
        TypeCodeButton().IsChecked(m_typeFilter == "code");
        TypeFilesButton().IsChecked(m_typeFilter == "file");
        RefreshItems();
    }

    void MainWindow::SetupImeGuards()
    {
        SearchBox().TextCompositionStarted([this](auto const&, auto const&)
        {
            m_imeComposing = true;
        });
        SearchBox().TextCompositionEnded([this](auto const&, auto const&)
        {
            m_imeComposing = false;
            m_ignoreNextEnter = true;
        });
    }

    void MainWindow::ShowDetailsImage(winrt::hstring const& contentType, winrt::hstring const& content)
    {
        DetailsImage().Source(nullptr);
        DetailsImage().Visibility(Visibility::Collapsed);
        if (contentType != L"image")
        {
            return;
        }

        std::wstring path{ content };
        if (path.empty() || path.rfind(L"data:image/", 0) == 0)
        {
            return;
        }
        std::replace(path.begin(), path.end(), L'\\', L'/');
        if (!std::filesystem::exists(std::filesystem::path{ content.c_str() }))
        {
            return;
        }

        try
        {
            Microsoft::UI::Xaml::Media::Imaging::BitmapImage bitmap;
            bitmap.UriSource(Windows::Foundation::Uri{ L"file:///" + path });
            DetailsImage().Source(bitmap);
            DetailsImage().Visibility(Visibility::Visible);
        }
        catch (winrt::hresult_error const&)
        {
        }
    }
}

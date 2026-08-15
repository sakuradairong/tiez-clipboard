#include "pch.h"
#include "MainWindow.xaml.h"

#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif

#include <microsoft.ui.xaml.window.h>
#include <commctrl.h>
#include <shellapi.h>

#pragma comment(lib, "comctl32.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shell32.lib")

namespace
{
    constexpr UINT kToggleHotkeyId = 1;
    constexpr UINT_PTR kMessageWindowSubclassId = 1;
    constexpr UINT_PTR kMainWindowSubclassId = 2;
    constexpr UINT_PTR kHoverPreviewSubclassId = 3;
    constexpr UINT kTrayCallbackMessage = WM_APP + 7;
    constexpr UINT kTrayIconId = 1;
    constexpr UINT kTrayShowCommand = 1001;
    constexpr UINT kTrayExitCommand = 1002;
    constexpr int kAppIconResourceId = 101;

    std::optional<std::filesystem::path> SelectBackupPath(HWND owner, bool save)
    {
        winrt::com_ptr<IFileDialog> dialog;
        winrt::check_hresult(CoCreateInstance(
            save ? CLSID_FileSaveDialog : CLSID_FileOpenDialog,
            nullptr,
            CLSCTX_INPROC_SERVER,
            __uuidof(IFileDialog),
            dialog.put_void()));

        COMDLG_FILTERSPEC filters[] = {
            { L"TieZ 备份 (*.tiez-backup)", L"*.tiez-backup" },
            { L"所有文件 (*.*)", L"*.*" },
        };
        winrt::check_hresult(dialog->SetFileTypes(
            static_cast<UINT>(std::size(filters)),
            filters));
        winrt::check_hresult(dialog->SetDefaultExtension(L"tiez-backup"));
        winrt::check_hresult(dialog->SetTitle(save ? L"导出 TieZ 备份" : L"选择 TieZ 备份"));
        if (save)
        {
            winrt::check_hresult(dialog->SetFileName(L"TieZ-backup.tiez-backup"));
        }

        FILEOPENDIALOGOPTIONS options{};
        winrt::check_hresult(dialog->GetOptions(&options));
        options |= FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        options |= save ? FOS_OVERWRITEPROMPT : FOS_FILEMUSTEXIST;
        winrt::check_hresult(dialog->SetOptions(options));

        auto const shown = dialog->Show(owner);
        if (shown == HRESULT_FROM_WIN32(ERROR_CANCELLED))
        {
            return std::nullopt;
        }
        winrt::check_hresult(shown);

        winrt::com_ptr<IShellItem> result;
        winrt::check_hresult(dialog->GetResult(result.put()));
        PWSTR rawPath{};
        winrt::check_hresult(result->GetDisplayName(SIGDN_FILESYSPATH, &rawPath));
        if (rawPath == nullptr)
        {
            throw winrt::hresult_error(E_UNEXPECTED, L"系统文件选择器未返回路径");
        }
        std::filesystem::path selected{ rawPath };
        CoTaskMemFree(rawPath);
        return selected;
    }

    winrt::hstring BackupSummary(
        winrt::Windows::Data::Json::JsonObject const& information,
        std::wstring_view prefix)
    {
        auto const entryCount = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"entryCount")));
        auto const fileCount = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"fileCount")));
        auto const totalBytes = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"totalBytes")));
        auto const appVersion = information.GetNamedString(L"appVersion");
        std::wstringstream message;
        message << prefix
            << L"版本 " << appVersion.c_str()
            << L"，" << entryCount << L" 条记录，"
            << fileCount << L" 个文件，共 " << totalBytes << L" 字节。";
        return winrt::hstring{ message.str() };
    }

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
        if (contentType == L"html" || contentType == L"rich_text") return L"富文本";
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

    std::vector<winrt::hstring> ItemTags(
        winrt::Windows::Data::Json::JsonObject const& item)
    {
        std::vector<winrt::hstring> tags;
        if (!item.HasKey(L"tags"))
        {
            return tags;
        }
        auto const values = item.GetNamedArray(L"tags");
        tags.reserve(values.Size());
        for (std::uint32_t index = 0; index < values.Size(); ++index)
        {
            auto const tag = values.GetStringAt(index);
            if (!tag.empty())
            {
                tags.push_back(tag);
            }
        }
        return tags;
    }

    winrt::hstring JoinTags(std::vector<winrt::hstring> const& tags)
    {
        std::wstring joined;
        for (auto const& tag : tags)
        {
            if (!joined.empty())
            {
                joined.append(L"，");
            }
            joined.append(tag.c_str(), tag.size());
        }
        return winrt::hstring{ joined };
    }

    std::vector<winrt::hstring> SplitTags(winrt::hstring const& value)
    {
        std::wstring normalized{ value.c_str(), value.size() };
        std::replace(normalized.begin(), normalized.end(), L'，', L',');
        std::replace(normalized.begin(), normalized.end(), L'；', L',');
        std::replace(normalized.begin(), normalized.end(), L';', L',');

        constexpr std::wstring_view whitespace{ L" \t\r\n" };
        std::vector<winrt::hstring> tags;
        std::size_t start{};
        while (start <= normalized.size())
        {
            auto const end = normalized.find(L',', start);
            auto const token = normalized.substr(
                start,
                end == std::wstring::npos ? std::wstring::npos : end - start);
            auto const first = token.find_first_not_of(whitespace);
            if (first != std::wstring::npos)
            {
                auto const last = token.find_last_not_of(whitespace);
                tags.emplace_back(token.substr(first, last - first + 1));
            }
            if (end == std::wstring::npos)
            {
                break;
            }
            start = end + 1;
        }
        return tags;
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

    winrt::Microsoft::UI::Xaml::Controls::ToggleSwitch SettingToggle(
        winrt::hstring const& label,
        winrt::hstring const& description)
    {
        winrt::Microsoft::UI::Xaml::Controls::ToggleSwitch toggle;
        toggle.Header(winrt::box_value(label));
        toggle.OnContent(winrt::box_value(L"已开启"));
        toggle.OffContent(winrt::box_value(L"已关闭"));
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(toggle, label);
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetHelpText(
            toggle,
            description);
        return toggle;
    }

    LRESULT CALLBACK HotkeySubclassProc(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR,
        DWORD_PTR refData)
    {
        auto* window = reinterpret_cast<winrt::Tiez::WinUIProbe::implementation::MainWindow*>(refData);
        if (window != nullptr && window->OnNativeMessage(hwnd, message, wParam, lParam))
        {
            return 0;
        }

        return DefSubclassProc(hwnd, message, wParam, lParam);
    }

    LRESULT CALLBACK HoverPreviewSubclassProc(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR,
        DWORD_PTR)
    {
        if (message == WM_NCHITTEST)
        {
            return HTTRANSPARENT;
        }
        if (message == WM_MOUSEACTIVATE)
        {
            return MA_NOACTIVATE;
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
    using namespace Windows::ApplicationModel::DataTransfer;
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
            LoadSettings();
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
        HideHoverPreview();
        if (m_hoverPreviewWindow)
        {
            RemoveWindowSubclass(
                m_hoverPreviewHwnd,
                HoverPreviewSubclassProc,
                kHoverPreviewSubclassId);
            m_hoverPreviewWindow.Close();
            m_hoverPreviewWindow = nullptr;
            m_hoverPreviewHwnd = nullptr;
        }
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

    void MainWindow::SettingsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法加载设置。");
            return;
        }

        try
        {
            EnsureSettingsDialog();
            LoadSettings();
            m_suspendLifecycle = true;
            m_settingsDialog.XamlRoot(RootGrid().XamlRoot());
            (void)m_settingsDialog.ShowAsync();
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"打开设置失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"打开设置失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::PinWindowCheck_Changed(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_settingsLoading)
        {
            return;
        }
        auto const pinned = PinWindowCheck().IsChecked().Value();
        if (PersistSetting(
            "app.window_pinned",
            pinned ? "true" : "false",
            L"固定窗口"))
        {
            ApplyPinnedWindow(pinned);
        }
        else
        {
            LoadSettings();
        }
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

    void MainWindow::TagsTextBox_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (args.Key() != VirtualKey::Enter)
        {
            return;
        }
        if (m_imeComposing || m_ignoreNextEnter || (GetKeyState(VK_PROCESSKEY) & 0x8000))
        {
            m_ignoreNextEnter = false;
            return;
        }
        if (!m_readOnly)
        {
            SaveSelectedTags();
            args.Handled(true);
        }
    }

    void MainWindow::SaveTagsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        SaveSelectedTags();
    }

    void MainWindow::OpenSelectedButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_detailsEntryId)
        {
            OpenEntry(*m_detailsEntryId);
        }
        else
        {
            SetStatus(L"请先选择一条剪贴板记录。");
        }
    }

    void MainWindow::AnalyzeImageButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        AnalyzeSelectedImageAsync();
    }

    void MainWindow::CopyImageAnalysisButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_imageAnalysisCopyText.empty())
        {
            SetStatus(L"当前没有可复制的图片识别结果。");
            return;
        }

        try
        {
            DataPackage package;
            package.SetText(winrt::hstring{ m_imageAnalysisCopyText });
            Clipboard::SetContent(package);
            Clipboard::Flush();
            SetStatus(L"图片识别结果已复制到剪贴板。");
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"复制图片识别结果失败：", error.message()));
        }
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
        HideHoverPreview();
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
            m_readOnly = readOnly;

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

            ItemsPanel().Spacing(m_compactMode ? 6 : 12);
            ItemsPanel().Children().Clear();
            m_entryIds.clear();
            m_pinnedIds.clear();
            m_cards.clear();
            m_tagsById.clear();
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                auto const item = items.GetObjectAt(index);
                if (item.GetNamedBoolean(L"is_pinned"))
                {
                    m_pinnedIds.push_back(
                        static_cast<std::int64_t>(item.GetNamedNumber(L"id")));
                }
            }
            m_canReorderPinned = !readOnly
                && CurrentQuery().empty()
                && m_pinnedIds.size() > 1;
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                auto const item = items.GetObjectAt(index);
                auto const entryId = static_cast<std::int64_t>(item.GetNamedNumber(L"id"));
                m_entryIds.push_back(entryId);
                m_tagsById.emplace(entryId, ItemTags(item));
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

            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                auto const selectedId = m_entryIds[static_cast<std::size_t>(m_selectedIndex)];
                TagsTextBox().IsEnabled(!readOnly);
                SaveTagsButton().IsEnabled(!readOnly);
                if (TagsTextBox().FocusState() == FocusState::Unfocused)
                {
                    TagsTextBox().Text(JoinTags(m_tagsById[selectedId]));
                    ShowContent(selectedId);
                }
            }
            else
            {
                m_detailsEntryId.reset();
                DetailsTitleText().Text(L"剪贴板详情");
                DetailsMetadataText().Text(L"没有可显示的记录");
                DetailsContentText().Text(L"");
                OpenSelectedButton().IsEnabled(false);
                ShowDetailsImage({}, {});
                TagsTextBox().Text(L"");
                TagsTextBox().IsEnabled(false);
                SaveTagsButton().IsEnabled(false);
            }

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
        card.Padding(m_compactMode
            ? ThicknessHelper::FromLengths(12, 8, 12, 8)
            : ThicknessHelper::FromLengths(16, 16, 16, 16));
        card.PointerPressed([this, entryId, index](auto const&, auto const&)
        {
            m_selectedIndex = static_cast<int>(index);
            UpdateSelectionVisuals();
            ShowContent(entryId);
        });
        card.PointerEntered([this, entryId](auto const&, auto const&)
        {
            if (m_compactMode)
            {
                ShowHoverPreview(entryId);
            }
        });
        card.PointerExited([this](auto const&, auto const&)
        {
            HideHoverPreview();
        });
        card.DoubleTapped([this, entryId, readOnly](auto const&, auto const&)
        {
            if (!readOnly)
            {
                ApplyAction(entryId, "paste-plain");
            }
        });
        AutomationProperties::SetName(card, item.GetNamedString(L"preview"));
        AttachCardCommands(card, entryId, readOnly, isSensitive);
        AttachPinnedReorder(card, entryId, isPinned && m_canReorderPinned);
        m_cards.push_back(card);

        StackPanel content;
        content.Spacing(m_compactMode ? 5 : 10);

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

        auto const tags = ItemTags(item);
        if (!tags.empty())
        {
            StackPanel tagPanel;
            tagPanel.Orientation(Orientation::Horizontal);
            tagPanel.Spacing(6);
            for (auto const& tag : tags)
            {
                Border chip;
                chip.Padding(ThicknessHelper::FromLengths(8, 3, 8, 3));
                chip.CornerRadius(CornerRadiusHelper::FromUniformRadius(10));
                chip.Background(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"AccentFillColorSecondaryBrush"))
                    .as<Brush>());
                TextBlock label;
                label.Text(tag);
                label.FontSize(12);
                chip.Child(label);
                std::wstring automationName{ L"标签：" };
                automationName.append(tag.c_str(), tag.size());
                AutomationProperties::SetName(chip, winrt::hstring{ automationName });
                tagPanel.Children().Append(chip);
            }
            content.Children().Append(tagPanel);
        }

        TextBlock preview;
        preview.Text(item.GetNamedString(L"preview"));
        preview.TextWrapping(TextWrapping::WrapWholeWords);
        preview.IsTextSelectionEnabled(true);
        preview.MaxHeight(m_compactMode ? 48 : 112);

        StackPanel actions;
        actions.Orientation(Orientation::Horizontal);
        actions.Spacing(8);
        auto detailsButton = ActionButton(
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
        Button moveUpButton{ nullptr };
        Button moveDownButton{ nullptr };
        if (isPinned)
        {
            auto const position = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), entryId);
            auto const pinnedIndex = position == m_pinnedIds.end()
                ? m_pinnedIds.size()
                : static_cast<std::size_t>(std::distance(m_pinnedIds.begin(), position));
            moveUpButton = ActionButton(
                L"上移",
                [this, entryId] { MovePinnedEntry(entryId, -1); });
            moveDownButton = ActionButton(
                L"下移",
                [this, entryId] { MovePinnedEntry(entryId, 1); });
            moveUpButton.IsEnabled(m_canReorderPinned && pinnedIndex > 0);
            moveDownButton.IsEnabled(
                m_canReorderPinned && pinnedIndex + 1 < m_pinnedIds.size());
        }
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

        actions.Children().Append(detailsButton);
        actions.Children().Append(pinButton);
        if (moveUpButton)
        {
            actions.Children().Append(moveUpButton);
            actions.Children().Append(moveDownButton);
        }
        actions.Children().Append(pastePlainButton);
        actions.Children().Append(pasteRichButton);
        actions.Children().Append(copyButton);
        actions.Children().Append(deleteButton);

        content.Children().Append(metadata);
        content.Children().Append(preview);
        if (!m_compactMode)
        {
            content.Children().Append(actions);
        }
        card.Child(content);
        return card;
    }

    void MainWindow::ShowContent(std::int64_t entryId)
    {
        if (!m_core)
        {
            return;
        }

        OpenSelectedButton().IsEnabled(false);
        ImageAnalysisPanel().Visibility(Visibility::Collapsed);
        ImageAnalysisProgress().IsActive(false);
        ImageAnalysisProgress().Visibility(Visibility::Collapsed);
        ImageAnalysisResultText().Text(L"");
        CopyImageAnalysisButton().Visibility(Visibility::Collapsed);
        m_imageAnalysisLoaded = false;
        m_imageAnalysisCopyText.clear();

        try
        {
            auto const value = m_core->Content(entryId);
            auto const content = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const contentType = content.GetNamedString(L"content_type");
            auto const available = content.GetNamedBoolean(L"available");
            auto const isSensitive = content.GetNamedBoolean(L"is_sensitive");
            OpenSelectedButton().IsEnabled(available && !isSensitive);

            std::wstringstream title;
            title << L"记录 " << entryId;
            DetailsTitleText().Text(winrt::hstring{ title.str() });
            m_detailsEntryId = entryId;
            if (auto const tags = m_tagsById.find(entryId); tags != m_tagsById.end())
            {
                TagsTextBox().Text(JoinTags(tags->second));
            }
            TagsTextBox().IsEnabled(!m_readOnly);
            SaveTagsButton().IsEnabled(!m_readOnly);

            std::wstring metadata{ ContentTypeLabel(contentType).c_str() };
            metadata.append(isSensitive ? L" · 敏感内容" : L" · 内容可用");
            DetailsMetadataText().Text(metadata);

            auto const canAnalyzeImage = m_productionData
                && entryId > 0
                && contentType == L"image"
                && available
                && !isSensitive;
            if (canAnalyzeImage)
            {
                ImageAnalysisPanel().Visibility(Visibility::Visible);
                if (m_imageAnalysisBusy)
                {
                    SetImageAnalysisBusy(
                        true,
                        m_imageAnalysisEntryId == entryId
                            ? L"正在识别当前图片，请稍候……"
                            : L"正在后台识别另一张图片，请稍候……");
                }
                else
                {
                    try
                    {
                        auto const response = JsonObject::Parse(
                            tiez::probe::RustCoreBridge::Utf8ToHstring(
                                m_core->ImageAnalysis(entryId)));
                        ShowImageAnalysis(response);
                    }
                    catch (winrt::hresult_error const& error)
                    {
                        SetImageAnalysisBusy(
                            false,
                            StatusMessage(L"读取图片识别缓存失败：", error.message()));
                    }
                    catch (std::exception const& error)
                    {
                        SetImageAnalysisBusy(
                            false,
                            StatusMessage(
                                L"读取图片识别缓存失败：",
                                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
                    }
                }
            }

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

    void MainWindow::SetImageAnalysisBusy(bool busy, winrt::hstring const& message)
    {
        m_imageAnalysisBusy = busy;
        ImageAnalysisProgress().IsActive(busy);
        ImageAnalysisProgress().Visibility(busy ? Visibility::Visible : Visibility::Collapsed);
        AnalyzeImageButton().IsEnabled(
            m_productionData
            && ImageAnalysisPanel().Visibility() == Visibility::Visible
            && !busy);
        if (!message.empty())
        {
            ImageAnalysisStatusText().Text(message);
        }
    }

    void MainWindow::ShowImageAnalysis(JsonObject const& response)
    {
        auto const value = response.GetNamedValue(L"analysis");
        if (value.ValueType() == JsonValueType::Null)
        {
            m_imageAnalysisLoaded = false;
            m_imageAnalysisCopyText.clear();
            AnalyzeImageButton().Content(winrt::box_value(L"开始识别"));
            ImageAnalysisResultText().Text(L"");
            CopyImageAnalysisButton().Visibility(Visibility::Collapsed);
            SetImageAnalysisBusy(
                false,
                L"尚未识别。非敏感可写记录会把结果加入本地搜索索引。");
            return;
        }

        auto const analysis = value.GetObject();
        auto const text = analysis.GetNamedString(L"text", L"");
        auto const qrCodes = analysis.GetNamedArray(L"qrCodes", JsonArray{});
        auto const optionalString = [&analysis](winrt::hstring const& name)
        {
            if (!analysis.HasKey(name))
            {
                return winrt::hstring{};
            }
            auto const field = analysis.GetNamedValue(name);
            return field.ValueType() == JsonValueType::String
                ? field.GetString()
                : winrt::hstring{};
        };
        auto const language = optionalString(L"language");
        auto const cached = analysis.GetNamedBoolean(L"cached", false);
        auto const persisted = analysis.GetNamedBoolean(L"persisted", false);
        auto const ocrError = optionalString(L"ocrError");

        std::wstringstream display;
        std::wstringstream copy;
        if (!text.empty())
        {
            display << L"识别文字";
            if (!language.empty())
            {
                display << L"（" << language.c_str() << L"）";
            }
            display << L"\n" << text.c_str();
            copy << text.c_str();
        }
        for (std::uint32_t index = 0; index < qrCodes.Size(); ++index)
        {
            auto const code = qrCodes.GetStringAt(index);
            if (!display.str().empty())
            {
                display << L"\n\n";
            }
            display << L"二维码 " << (index + 1) << L"\n" << code.c_str();
            if (!copy.str().empty())
            {
                copy << L"\n\n";
            }
            copy << code.c_str();
        }

        m_imageAnalysisLoaded = true;
        m_imageAnalysisCopyText = copy.str();
        AnalyzeImageButton().Content(winrt::box_value(L"重新识别"));
        ImageAnalysisResultText().Text(winrt::hstring{ display.str() });
        CopyImageAnalysisButton().Visibility(
            m_imageAnalysisCopyText.empty() ? Visibility::Collapsed : Visibility::Visible);

        std::wstring status = cached ? L"已加载本地识别缓存。" : L"图片识别完成。";
        if (m_imageAnalysisCopyText.empty())
        {
            status = ocrError.empty()
                ? L"没有识别到文字或二维码。"
                : std::wstring{ L"系统 OCR 不可用：" } + ocrError.c_str();
        }
        if (!persisted)
        {
            status.append(L" 本次结果仅在内存中显示，未写入搜索索引。");
        }
        SetImageAnalysisBusy(false, winrt::hstring{ status });
    }

    winrt::fire_and_forget MainWindow::AnalyzeSelectedImageAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || !m_detailsEntryId || m_imageAnalysisBusy || !m_productionData)
        {
            co_return;
        }

        auto const entryId = *m_detailsEntryId;
        auto const force = m_imageAnalysisLoaded;
        auto const uiThread = winrt::apartment_context{};
        m_imageAnalysisEntryId = entryId;
        SetImageAnalysisBusy(true, L"正在使用 Windows OCR 和本地二维码解码器识别图片……");
        std::string response;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            response = m_core->AnalyzeImage(entryId, force);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        m_imageAnalysisBusy = false;
        m_imageAnalysisEntryId.reset();
        if (!m_detailsEntryId || *m_detailsEntryId != entryId)
        {
            if (m_detailsEntryId)
            {
                ShowContent(*m_detailsEntryId);
            }
            co_return;
        }
        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"图片识别失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetImageAnalysisBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            auto const analysis = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            ShowImageAnalysis(analysis);
            SetStatus(L"图片识别完成；可复制结果，已持久化的结果也可直接搜索。");
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取图片识别结果：", error.message());
            SetImageAnalysisBusy(false, message);
            SetStatus(message);
        }
    }

    void MainWindow::OpenEntry(std::int64_t entryId)
    {
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法打开内容。");
            return;
        }

        HideHoverPreview();
        try
        {
            auto const value = m_core->PrepareOpenContent(entryId);
            auto const plan = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            if (!plan.GetNamedBoolean(L"requires_confirmation"))
            {
                LaunchOpenPlan(plan);
                return;
            }

            ContentDialog confirmation;
            confirmation.Title(winrt::box_value(L"确认打开外部内容"));
            confirmation.PrimaryButtonText(L"继续打开");
            confirmation.CloseButtonText(L"取消");
            confirmation.DefaultButton(ContentDialogButton::Close);
            confirmation.XamlRoot(RootGrid().XamlRoot());

            StackPanel message;
            message.Spacing(8);
            TextBlock warning;
            warning.Text(plan.GetNamedString(L"kind") == L"url"
                ? L"该记录使用自定义链接协议。继续后，Windows 会把它交给已注册的外部应用。"
                : L"该富文本记录会作为本地 HTML 临时文件交给默认浏览器。请仅打开你信任的内容。");
            warning.TextWrapping(TextWrapping::Wrap);
            TextBlock target;
            target.Text(plan.GetNamedString(L"target"));
            target.FontFamily(FontFamily{ L"Consolas" });
            target.TextWrapping(TextWrapping::Wrap);
            target.IsTextSelectionEnabled(true);
            message.Children().Append(warning);
            message.Children().Append(target);
            confirmation.Content(message);

            m_suspendLifecycle = true;
            confirmation.PrimaryButtonClick([this, plan](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
                LaunchOpenPlan(plan);
            });
            confirmation.Closed([this](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
            });
            (void)confirmation.ShowAsync();
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"打开内容失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"打开内容失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::LaunchOpenPlan(JsonObject const& plan)
    {
        try
        {
            auto const target = plan.GetNamedString(L"target");
            auto const result = reinterpret_cast<std::intptr_t>(ShellExecuteW(
                GetWindowHandle(),
                L"open",
                target.c_str(),
                nullptr,
                nullptr,
                SW_SHOWNORMAL));
            if (result <= 32)
            {
                throw std::runtime_error(
                    "ShellExecuteW failed with code " + std::to_string(result));
            }

            SetStatus(plan.GetNamedBoolean(L"temporary")
                ? L"已创建受控临时文件，并交给系统默认应用打开。"
                : L"已交给 Windows 默认应用打开。");
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"启动默认应用失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"启动默认应用失败：",
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
            kMessageWindowSubclassId,
            reinterpret_cast<DWORD_PTR>(this));
        SetWindowSubclass(
            GetWindowHandle(),
            HotkeySubclassProc,
            kMainWindowSubclassId,
            reinterpret_cast<DWORD_PTR>(this));
        if (!RegisterHotKey(m_hotkeyHwnd, kToggleHotkeyId, MOD_ALT | MOD_NOREPEAT, 0x43))
        {
            SetStatus(L"Alt+C 已被其他程序占用，可通过系统托盘重新显示 TieZ。");
        }
        SetupTrayIcon();
    }

    void MainWindow::TeardownLifecycle()
    {
        RemoveTrayIcon();
        if (m_hwnd != nullptr && IsWindow(m_hwnd))
        {
            RemoveWindowSubclass(m_hwnd, HotkeySubclassProc, kMainWindowSubclassId);
        }
        if (m_hotkeyHwnd != nullptr)
        {
            UnregisterHotKey(m_hotkeyHwnd, kToggleHotkeyId);
            RemoveWindowSubclass(
                m_hotkeyHwnd,
                HotkeySubclassProc,
                kMessageWindowSubclassId);
            DestroyWindow(m_hotkeyHwnd);
            m_hotkeyHwnd = nullptr;
        }
    }

    void MainWindow::SetupTrayIcon()
    {
        m_taskbarCreatedMessage = RegisterWindowMessageW(L"TaskbarCreated");
        auto const loadedIcon = LoadImageW(
            GetModuleHandleW(nullptr),
            MAKEINTRESOURCEW(kAppIconResourceId),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR);
        m_trayIcon = static_cast<HICON>(loadedIcon);
        if (m_trayIcon == nullptr)
        {
            m_trayIcon = CopyIcon(LoadIconW(nullptr, IDI_APPLICATION));
        }
        if (m_trayVisible)
        {
            AddTrayIcon();
        }
    }

    void MainWindow::AddTrayIcon()
    {
        if (m_trayAdded)
        {
            return;
        }
        if (m_hotkeyHwnd == nullptr || m_trayIcon == nullptr)
        {
            SetStatus(L"无法创建系统托盘图标；仍可使用 Alt+C 显示 TieZ。");
            return;
        }

        NOTIFYICONDATAW data{};
        data.cbSize = sizeof(data);
        data.hWnd = m_hotkeyHwnd;
        data.uID = kTrayIconId;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = kTrayCallbackMessage;
        data.hIcon = m_trayIcon;
        wcscpy_s(data.szTip, L"TieZ 剪贴板");
        if (!Shell_NotifyIconW(NIM_ADD, &data))
        {
            m_trayAdded = false;
            SetStatus(L"无法创建系统托盘图标；仍可使用 Alt+C 显示 TieZ。");
            return;
        }

        data.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &data);
        m_trayAdded = true;
    }

    void MainWindow::RemoveTrayIcon()
    {
        if (m_trayAdded && m_hotkeyHwnd != nullptr)
        {
            NOTIFYICONDATAW data{};
            data.cbSize = sizeof(data);
            data.hWnd = m_hotkeyHwnd;
            data.uID = kTrayIconId;
            Shell_NotifyIconW(NIM_DELETE, &data);
            m_trayAdded = false;
        }
        if (m_trayIcon != nullptr)
        {
            DestroyIcon(m_trayIcon);
            m_trayIcon = nullptr;
        }
    }

    void MainWindow::SetTrayVisible(bool visible)
    {
        m_trayVisible = visible;
        if (visible)
        {
            AddTrayIcon();
            return;
        }

        if (m_trayAdded && m_hotkeyHwnd != nullptr)
        {
            NOTIFYICONDATAW data{};
            data.cbSize = sizeof(data);
            data.hWnd = m_hotkeyHwnd;
            data.uID = kTrayIconId;
            Shell_NotifyIconW(NIM_DELETE, &data);
            m_trayAdded = false;
        }
    }

    void MainWindow::ShowTrayMenu()
    {
        auto const menu = CreatePopupMenu();
        if (menu == nullptr)
        {
            return;
        }
        AppendMenuW(menu, MF_STRING, kTrayShowCommand, L"显示主界面");
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, kTrayExitCommand, L"退出 TieZ");

        POINT position{};
        GetCursorPos(&position);
        m_suspendLifecycle = true;
        SetForegroundWindow(m_hotkeyHwnd);
        auto const command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            position.x,
            position.y,
            0,
            m_hotkeyHwnd,
            nullptr);
        DestroyMenu(menu);
        PostMessageW(m_hotkeyHwnd, WM_NULL, 0, 0);
        m_suspendLifecycle = false;

        if (command == kTrayShowCommand)
        {
            ShowMainWindow(false);
        }
        else if (command == kTrayExitCommand)
        {
            RequestExit();
        }
    }

    void MainWindow::RequestExit()
    {
        if (m_exitRequested)
        {
            return;
        }
        m_exitRequested = true;
        m_suspendLifecycle = true;
        RemoveTrayIcon();
        Close();
    }

    bool MainWindow::OnNativeMessage(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam)
    {
        if (hwnd == m_hotkeyHwnd && message == WM_HOTKEY && wParam == kToggleHotkeyId)
        {
            OnToggleHotkey();
            return true;
        }
        if (hwnd == m_hotkeyHwnd
            && m_taskbarCreatedMessage != 0
            && message == m_taskbarCreatedMessage)
        {
            m_trayAdded = false;
            if (m_trayVisible)
            {
                AddTrayIcon();
            }
            return true;
        }
        if (hwnd == m_hotkeyHwnd && message == kTrayCallbackMessage)
        {
            auto const notification = LOWORD(lParam);
            if (notification == NIN_SELECT
                || notification == NIN_KEYSELECT
                || notification == WM_LBUTTONUP
                || notification == WM_LBUTTONDBLCLK)
            {
                ShowMainWindow(false);
            }
            else if (notification == WM_CONTEXTMENU || notification == WM_RBUTTONUP)
            {
                ShowTrayMenu();
            }
            return true;
        }
        if (hwnd == GetWindowHandle() && message == WM_CLOSE && !m_exitRequested)
        {
            HideMainWindow();
            return true;
        }
        return false;
    }

    void MainWindow::HideMainWindow()
    {
        HideHoverPreview();
        ShowWindow(GetWindowHandle(), SW_HIDE);
        SetStatus(L"窗口已隐藏，按 Alt+C 或点击系统托盘图标可重新显示。");
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
        SetStatus(captureForeground
            ? L"已通过 Alt+C 显示窗口，并记录粘贴目标窗口。"
            : L"已通过系统托盘显示主界面。");
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
        if (TagsTextBox().FocusState() != FocusState::Unfocused)
        {
            return false;
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
        bool readOnly,
        bool isSensitive)
    {
        MenuFlyout flyout;
        flyout.Items().Append(CommandItem(
            L"打开",
            !isSensitive,
            [this, entryId] { OpenEntry(entryId); }));
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
            [this, entryId] { SelectEntry(entryId); }));
        card.ContextFlyout(flyout);
    }

    void MainWindow::AttachPinnedReorder(
        Border const& card,
        std::int64_t entryId,
        bool enabled)
    {
        card.CanDrag(enabled);
        card.AllowDrop(enabled);
        if (!enabled)
        {
            return;
        }

        AutomationProperties::SetHelpText(
            card,
            L"可拖动调整置顶顺序，也可使用上移和下移按钮。");
        card.DragStarting([this, entryId](auto const&, DragStartingEventArgs const& args)
        {
            m_draggedPinnedId = entryId;
            args.Data().SetText(winrt::to_hstring(entryId));
            args.Data().RequestedOperation(DataPackageOperation::Move);
        });
        card.DragOver([this, entryId](auto const&, DragEventArgs const& args)
        {
            if (m_draggedPinnedId && *m_draggedPinnedId != entryId)
            {
                args.AcceptedOperation(DataPackageOperation::Move);
                args.Handled(true);
            }
        });
        card.Drop([this, entryId](IInspectable const& sender, DragEventArgs const& args)
        {
            if (m_draggedPinnedId && *m_draggedPinnedId != entryId)
            {
                auto const targetCard = sender.as<Border>();
                auto const position = args.GetPosition(targetCard);
                auto const afterTarget = position.Y > targetCard.ActualHeight() / 2.0;
                auto const sourceId = *m_draggedPinnedId;
                m_draggedPinnedId.reset();
                DropPinnedEntry(sourceId, entryId, afterTarget);
                args.Handled(true);
            }
        });
        card.DropCompleted([this](auto const&, DropCompletedEventArgs const&)
        {
            m_draggedPinnedId.reset();
        });
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

    void MainWindow::MovePinnedEntry(std::int64_t entryId, int delta)
    {
        if (!m_canReorderPinned || delta == 0)
        {
            return;
        }
        auto const position = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), entryId);
        if (position == m_pinnedIds.end())
        {
            return;
        }
        auto const index = static_cast<std::ptrdiff_t>(
            std::distance(m_pinnedIds.begin(), position));
        auto const target = index + delta;
        if (target < 0 || target >= static_cast<std::ptrdiff_t>(m_pinnedIds.size()))
        {
            return;
        }
        std::iter_swap(
            m_pinnedIds.begin() + index,
            m_pinnedIds.begin() + target);
        PersistPinnedOrder();
    }

    void MainWindow::DropPinnedEntry(
        std::int64_t sourceId,
        std::int64_t targetId,
        bool afterTarget)
    {
        if (!m_canReorderPinned || sourceId == targetId)
        {
            return;
        }
        auto source = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), sourceId);
        auto target = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), targetId);
        if (source == m_pinnedIds.end() || target == m_pinnedIds.end())
        {
            return;
        }

        m_pinnedIds.erase(source);
        target = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), targetId);
        auto insert = target + (afterTarget ? 1 : 0);
        m_pinnedIds.insert(insert, sourceId);
        PersistPinnedOrder();
    }

    void MainWindow::PersistPinnedOrder()
    {
        try
        {
            JsonArray ids;
            for (auto const entryId : m_pinnedIds)
            {
                ids.Append(JsonValue::CreateNumberValue(static_cast<double>(entryId)));
            }
            auto const value = m_core->UpdatePinnedOrder(
                winrt::to_string(ids.Stringify()));
            auto const result = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            RefreshItems();

            std::wstringstream status;
            status << L"置顶顺序已保存 · " << m_pinnedIds.size()
                   << L" 条 · 第 "
                   << static_cast<std::uint64_t>(result.GetNamedNumber(L"generation"))
                   << L" 代";
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            RefreshItems();
            SetStatus(StatusMessage(
                L"保存置顶顺序失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
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

    void MainWindow::SaveSelectedTags()
    {
        if (!m_core || !m_detailsEntryId || m_readOnly)
        {
            SetStatus(m_readOnly
                ? L"当前历史以只读方式打开，无法保存标签。"
                : L"请先选择一条记录再保存标签。");
            return;
        }

        try
        {
            auto const tags = SplitTags(TagsTextBox().Text());
            JsonArray values;
            for (auto const& tag : tags)
            {
                values.Append(JsonValue::CreateStringValue(tag));
            }
            auto const requestedId = *m_detailsEntryId;
            auto const result = m_core->UpdateTags(
                requestedId,
                winrt::to_string(values.Stringify()));
            auto const mutation = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(result));

            auto effectiveId = requestedId;
            auto const replacement = mutation.GetNamedValue(L"replacement_id");
            if (replacement.ValueType() == JsonValueType::Number)
            {
                effectiveId = static_cast<std::int64_t>(replacement.GetNumber());
            }
            m_detailsEntryId = effectiveId;
            RefreshItems();
            m_tagsById[effectiveId] = tags;
            SelectEntry(effectiveId);

            std::wstringstream status;
            status << L"标签已保存";
            if (effectiveId != requestedId)
            {
                status << L" · 会话记录已安全保存为 ID " << effectiveId;
            }
            else
            {
                status << L" · 记录 ID " << effectiveId;
            }
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"保存标签失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::SelectEntry(std::int64_t entryId)
    {
        for (std::size_t index = 0; index < m_entryIds.size(); ++index)
        {
            if (m_entryIds[index] == entryId)
            {
                m_selectedIndex = static_cast<int>(index);
                UpdateSelectionVisuals();
                ShowContent(entryId);
                return;
            }
        }
        ShowContent(entryId);
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

    void MainWindow::SetBackupBusy(bool busy, winrt::hstring const& message)
    {
        m_backupBusy = busy;
        if (m_exportBackupButton)
        {
            m_exportBackupButton.IsEnabled(m_productionData && !busy);
        }
        if (m_restoreBackupButton)
        {
            m_restoreBackupButton.IsEnabled(
                m_productionData && !m_settingsReadOnly && !busy);
        }
        if (m_backupProgress)
        {
            m_backupProgress.IsActive(busy);
            m_backupProgress.Visibility(busy ? Visibility::Visible : Visibility::Collapsed);
        }
        if (m_backupStatus)
        {
            m_backupStatus.Text(message);
        }
    }

    winrt::fire_and_forget MainWindow::ExportBackupAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || m_backupBusy || !m_productionData)
        {
            co_return;
        }

        std::optional<std::filesystem::path> destination;
        try
        {
            destination = SelectBackupPath(GetWindowHandle(), true);
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法选择备份位置：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"无法选择备份位置：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        if (!destination)
        {
            co_return;
        }

        auto const path = winrt::to_string(winrt::hstring{ destination->wstring() });
        auto const uiThread = winrt::apartment_context{};
        SetBackupBusy(true, L"正在创建一致性快照并校验备份，请勿退出 TieZ……");
        std::string response;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            response = m_core->CreateBackup(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"导出备份失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            auto const information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            auto const message = BackupSummary(information, L"备份已导出：");
            SetBackupBusy(false, message);
            SetStatus(L"备份已安全导出。原始数据未被修改。");
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取备份结果：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
        }
    }

    winrt::fire_and_forget MainWindow::RestoreBackupAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || m_backupBusy || !m_productionData || m_settingsReadOnly)
        {
            co_return;
        }

        std::optional<std::filesystem::path> source;
        try
        {
            source = SelectBackupPath(GetWindowHandle(), false);
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法选择备份文件：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"无法选择备份文件：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        if (!source)
        {
            co_return;
        }

        auto const path = winrt::to_string(winrt::hstring{ source->wstring() });
        auto const uiThread = winrt::apartment_context{};
        SetBackupBusy(true, L"正在检查备份结构、数据库和全部 SHA-256 校验值……");
        std::string inspectionResponse;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            inspectionResponse = m_core->InspectBackup(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"备份校验失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        JsonObject information;
        try
        {
            information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(inspectionResponse));
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取备份信息：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        auto summary = std::wstring{ BackupSummary(information, L"已验证备份：").c_str() };
        summary.append(
            L"\n\n继续后将把备份复制到 TieZ 的安全待恢复位置。当前数据不会立即改变；下次启动会在打开数据库前恢复，并保留七天回滚副本。是否继续？");
        if (MessageBoxW(
            GetWindowHandle(),
            summary.c_str(),
            L"确认安排恢复",
            MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) != IDYES)
        {
            SetBackupBusy(false, L"已取消恢复，当前数据未改变。");
            co_return;
        }

        SetBackupBusy(true, L"正在复制并再次校验待恢复备份……");
        std::string scheduleResponse;
        failure.clear();
        co_await winrt::resume_background();
        try
        {
            scheduleResponse = m_core->ScheduleRestore(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"安排恢复失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(scheduleResponse));
            SetBackupBusy(false, BackupSummary(information, L"恢复已安排："));
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取恢复结果：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        SetStatus(L"恢复已安排；退出后再次启动 TieZ 即会安全应用。");
        if (MessageBoxW(
            GetWindowHandle(),
            L"恢复已安排。现在退出 TieZ，可在下次启动时应用备份。\n\n是否立即退出？",
            L"恢复已安排",
            MB_ICONINFORMATION | MB_YESNO | MB_DEFBUTTON1) == IDYES)
        {
            RequestExit();
        }
    }

    void MainWindow::EnsureSettingsDialog()
    {
        if (m_settingsDialog)
        {
            return;
        }

        m_settingsDialog = ContentDialog();
        m_settingsDialog.Title(winrt::box_value(L"TieZ 设置"));
        m_settingsDialog.CloseButtonText(L"完成");
        m_settingsDialog.DefaultButton(ContentDialogButton::Close);

        m_settingsPanel = StackPanel();
        m_settingsPanel.Spacing(14);
        m_settingsPanel.MaxWidth(520);

        TextBlock introduction;
        introduction.Text(L"这些设置直接写入 TieZ 数据库，并立即作用于原生主窗口。敏感密钥不会在此界面读取或显示。");
        introduction.TextWrapping(TextWrapping::Wrap);
        introduction.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(introduction);

        TextBlock appearanceTitle;
        appearanceTitle.Text(L"外观与窗口");
        appearanceTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(appearanceTitle);

        m_colorModeCombo = ComboBox();
        m_colorModeCombo.Header(winrt::box_value(L"界面主题"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"跟随系统"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"浅色"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"深色"));
        AutomationProperties::SetName(m_colorModeCombo, L"界面主题");
        m_settingsPanel.Children().Append(m_colorModeCombo);

        m_compactModeToggle = SettingToggle(
            L"紧凑列表",
            L"减少卡片间距并隐藏卡片按钮；仍可双击、使用键盘或右键菜单操作。");
        m_windowPinnedToggle = SettingToggle(
            L"固定窗口",
            L"让主窗口保持置顶，并在失去焦点时继续显示。");
        m_trayVisibleToggle = SettingToggle(
            L"显示系统托盘图标",
            L"关闭后仍可使用 Alt+C 显示 TieZ。");
        m_settingsPanel.Children().Append(m_compactModeToggle);
        m_settingsPanel.Children().Append(m_windowPinnedToggle);
        m_settingsPanel.Children().Append(m_trayVisibleToggle);

        TextBlock historyTitle;
        historyTitle.Text(L"历史与捕获");
        historyTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(historyTitle);

        m_persistentToggle = SettingToggle(
            L"持久保存历史",
            L"关闭时新记录只保存在当前会话，置顶或添加标签后才会写入数据库。");
        m_persistentLimitEnabledToggle = SettingToggle(
            L"限制持久历史数量",
            L"仅清理未置顶、未加标签且未受保护的较旧记录。");
        m_persistentLimitNumber = NumberBox();
        m_persistentLimitNumber.Header(winrt::box_value(L"最多保留记录数"));
        m_persistentLimitNumber.Minimum(0);
        m_persistentLimitNumber.Maximum(100000);
        m_persistentLimitNumber.SmallChange(50);
        m_persistentLimitNumber.SpinButtonPlacementMode(NumberBoxSpinButtonPlacementMode::Inline);
        AutomationProperties::SetName(m_persistentLimitNumber, L"最多保留记录数");
        m_deduplicateToggle = SettingToggle(
            L"自动去重",
            L"忽略与最近记录相同的剪贴板内容。");
        m_captureFilesToggle = SettingToggle(
            L"捕获文件",
            L"记录从资源管理器等应用复制的文件路径。");
        m_captureRichTextToggle = SettingToggle(
            L"捕获富文本",
            L"保留 HTML 富文本；关闭时仍会按纯文本记录。");
        m_privacyProtectionToggle = SettingToggle(
            L"隐私保护",
            L"按现有规则识别敏感内容并加密持久数据。");
        m_settingsPanel.Children().Append(m_persistentToggle);
        m_settingsPanel.Children().Append(m_persistentLimitEnabledToggle);
        m_settingsPanel.Children().Append(m_persistentLimitNumber);
        m_settingsPanel.Children().Append(m_deduplicateToggle);
        m_settingsPanel.Children().Append(m_captureFilesToggle);
        m_settingsPanel.Children().Append(m_captureRichTextToggle);
        m_settingsPanel.Children().Append(m_privacyProtectionToggle);

        TextBlock dataSafetyTitle;
        dataSafetyTitle.Text(L"数据安全");
        dataSafetyTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(dataSafetyTitle);

        TextBlock dataSafetyDescription;
        dataSafetyDescription.Text(
            L"备份包含一致的 SQLite 快照、附件和表情收藏，文件本身不会额外加密，请仅保存到可信位置。受保护字段仍绑定当前 Windows 账户的 DPAPI，换账户或设备可能无法解密。恢复前会校验结构、大小和 SHA-256，并保留七天回滚副本。");
        dataSafetyDescription.TextWrapping(TextWrapping::Wrap);
        dataSafetyDescription.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(dataSafetyDescription);

        StackPanel backupActions;
        backupActions.Orientation(Orientation::Horizontal);
        backupActions.Spacing(8);
        m_exportBackupButton = Button();
        m_exportBackupButton.Content(winrt::box_value(L"导出备份"));
        m_exportBackupButton.IsEnabled(false);
        AutomationProperties::SetName(m_exportBackupButton, L"导出 TieZ 备份");
        m_restoreBackupButton = Button();
        m_restoreBackupButton.Content(winrt::box_value(L"恢复备份"));
        m_restoreBackupButton.IsEnabled(false);
        AutomationProperties::SetName(m_restoreBackupButton, L"恢复 TieZ 备份");
        m_backupProgress = ProgressRing();
        m_backupProgress.Width(22);
        m_backupProgress.Height(22);
        m_backupProgress.IsActive(false);
        m_backupProgress.Visibility(Visibility::Collapsed);
        backupActions.Children().Append(m_exportBackupButton);
        backupActions.Children().Append(m_restoreBackupButton);
        backupActions.Children().Append(m_backupProgress);
        m_settingsPanel.Children().Append(backupActions);

        m_backupStatus = TextBlock();
        m_backupStatus.TextWrapping(TextWrapping::Wrap);
        m_backupStatus.IsTextSelectionEnabled(true);
        m_backupStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_backupStatus, L"备份状态");
        m_settingsPanel.Children().Append(m_backupStatus);

        m_exportBackupButton.Click([this](auto const&, auto const&)
        {
            ExportBackupAsync();
        });
        m_restoreBackupButton.Click([this](auto const&, auto const&)
        {
            RestoreBackupAsync();
        });

        ScrollViewer scroller;
        scroller.MaxHeight(620);
        scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
        scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
        scroller.Content(m_settingsPanel);
        m_settingsDialog.Content(scroller);
        m_settingsDialog.Closed([this](auto const&, auto const&)
        {
            m_suspendLifecycle = false;
            SearchBox().Focus(FocusState::Programmatic);
        });

        m_colorModeCombo.SelectionChanged([this](auto const&, auto const&)
        {
            if (m_settingsLoading)
            {
                return;
            }
            auto const index = m_colorModeCombo.SelectedIndex();
            auto const mode = index == 1 ? "light" : index == 2 ? "dark" : "system";
            if (PersistSetting("app.color_mode", mode, L"界面主题"))
            {
                ApplyColorMode(mode);
            }
            else
            {
                LoadSettings();
            }
        });
        m_compactModeToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_compactModeToggle.IsOn();
            if (PersistSetting(
                "app.compact_mode",
                enabled ? "true" : "false",
                L"紧凑列表"))
            {
                m_compactMode = enabled;
                RefreshItems();
            }
            else LoadSettings();
        });
        m_windowPinnedToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_windowPinnedToggle.IsOn();
            if (PersistSetting(
                "app.window_pinned",
                enabled ? "true" : "false",
                L"固定窗口"))
            {
                ApplyPinnedWindow(enabled);
            }
            else LoadSettings();
        });
        m_trayVisibleToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const visible = m_trayVisibleToggle.IsOn();
            if (PersistSetting(
                "app.hide_tray_icon",
                visible ? "false" : "true",
                L"系统托盘"))
            {
                SetTrayVisible(visible);
            }
            else LoadSettings();
        });
        m_persistentToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_persistentToggle.IsOn();
            if (!PersistSetting(
                "app.persistent",
                enabled ? "true" : "false",
                L"持久保存历史"))
            {
                LoadSettings();
            }
        });
        m_persistentLimitEnabledToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_persistentLimitEnabledToggle.IsOn();
            if (PersistSetting(
                "app.persistent_limit_enabled",
                enabled ? "true" : "false",
                L"历史数量限制"))
            {
                m_persistentLimitNumber.IsEnabled(enabled && !m_settingsReadOnly);
            }
            else LoadSettings();
        });
        m_persistentLimitNumber.ValueChanged([this](auto const&, auto const& args)
        {
            if (m_settingsLoading || !std::isfinite(args.NewValue())) return;
            auto const limit = static_cast<std::int64_t>(std::llround(args.NewValue()));
            if (!PersistSetting(
                "app.persistent_limit",
                std::to_string(limit),
                L"历史数量上限"))
            {
                LoadSettings();
            }
        });
        m_deduplicateToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_deduplicateToggle.IsOn();
            if (!PersistSetting(
                "app.deduplicate",
                enabled ? "true" : "false",
                L"自动去重"))
            {
                LoadSettings();
            }
        });
        m_captureFilesToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_captureFilesToggle.IsOn();
            if (!PersistSetting(
                "app.capture_files",
                enabled ? "true" : "false",
                L"文件捕获"))
            {
                LoadSettings();
            }
        });
        m_captureRichTextToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_captureRichTextToggle.IsOn();
            if (!PersistSetting(
                "app.capture_rich_text",
                enabled ? "true" : "false",
                L"富文本捕获"))
            {
                LoadSettings();
            }
        });
        m_privacyProtectionToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_privacyProtectionToggle.IsOn();
            if (!PersistSetting(
                "app.privacy_protection",
                enabled ? "true" : "false",
                L"隐私保护"))
            {
                LoadSettings();
            }
        });
    }

    void MainWindow::LoadSettings()
    {
        if (!m_core)
        {
            return;
        }

        auto const value = m_core->Settings();
        auto const root = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(value));
        auto const values = root.GetNamedObject(L"values");
        auto const getValue = [&values](wchar_t const* key, wchar_t const* fallback)
        {
            return values.HasKey(key) ? values.GetNamedString(key) : winrt::hstring{ fallback };
        };
        auto const getBool = [&getValue](wchar_t const* key, bool fallback)
        {
            auto const value = getValue(key, fallback ? L"true" : L"false");
            return value == L"true" || value == L"1";
        };

        auto const colorMode = getValue(L"app.color_mode", L"system");
        auto const compactMode = getBool(L"app.compact_mode", false);
        auto const persistent = getBool(L"app.persistent", false);
        auto const limitEnabled = getBool(L"app.persistent_limit_enabled", true);
        auto const deduplicate = getBool(L"app.deduplicate", true);
        auto const captureFiles = getBool(L"app.capture_files", false);
        auto const captureRichText = getBool(L"app.capture_rich_text", false);
        auto const privacyProtection = getBool(L"app.privacy_protection", true);
        auto const trayVisible = !getBool(L"app.hide_tray_icon", false);
        auto const pinned = getBool(L"app.window_pinned", false);
        auto const adapter = root.GetNamedString(L"adapter", L"memory");
        double persistentLimit = 500;
        try
        {
            persistentLimit = std::stod(std::wstring{
                getValue(L"app.persistent_limit", L"500").c_str() });
        }
        catch (std::exception const&)
        {
            persistentLimit = 500;
        }

        auto const compactChanged = m_compactMode != compactMode;
        m_settingsReadOnly = root.GetNamedBoolean(L"read_only");
        m_productionData = adapter == L"sqlite" || adapter == L"sqlite-read-only";
        m_settingsLoading = true;
        m_compactMode = compactMode;
        if (m_settingsPanel)
        {
            auto const settingsEnabled = !m_settingsReadOnly;
            m_settingsPanel.IsHitTestVisible(true);
            m_settingsPanel.Opacity(1.0);
            m_colorModeCombo.IsEnabled(settingsEnabled);
            m_compactModeToggle.IsEnabled(settingsEnabled);
            m_persistentToggle.IsEnabled(settingsEnabled);
            m_persistentLimitEnabledToggle.IsEnabled(settingsEnabled);
            m_deduplicateToggle.IsEnabled(settingsEnabled);
            m_captureFilesToggle.IsEnabled(settingsEnabled);
            m_captureRichTextToggle.IsEnabled(settingsEnabled);
            m_privacyProtectionToggle.IsEnabled(settingsEnabled);
            m_trayVisibleToggle.IsEnabled(settingsEnabled);
            m_windowPinnedToggle.IsEnabled(settingsEnabled);
            m_colorModeCombo.SelectedIndex(colorMode == L"light" ? 1 : colorMode == L"dark" ? 2 : 0);
            m_compactModeToggle.IsOn(compactMode);
            m_persistentToggle.IsOn(persistent);
            m_persistentLimitEnabledToggle.IsOn(limitEnabled);
            m_persistentLimitNumber.Value(persistentLimit);
            m_persistentLimitNumber.IsEnabled(limitEnabled && !m_settingsReadOnly);
            m_deduplicateToggle.IsOn(deduplicate);
            m_captureFilesToggle.IsOn(captureFiles);
            m_captureRichTextToggle.IsOn(captureRichText);
            m_privacyProtectionToggle.IsOn(privacyProtection);
            m_trayVisibleToggle.IsOn(trayVisible);
            m_windowPinnedToggle.IsOn(pinned);

            auto backupMessage = m_backupStatus.Text();
            if (backupMessage.empty())
            {
                if (!m_productionData)
                {
                    backupMessage = L"当前使用演示数据。连接生产数据库后才能导出或恢复备份。";
                }
                else if (m_settingsReadOnly)
                {
                    backupMessage = L"当前为只读生产数据：可以导出备份，不能安排恢复。";
                }
                else
                {
                    backupMessage = L"建议定期导出备份，并将文件保存在 TieZ 数据目录之外。";
                }
            }
            SetBackupBusy(m_backupBusy, backupMessage);
        }
        m_settingsLoading = false;

        ApplyColorMode(winrt::to_string(colorMode));
        ApplyPinnedWindow(pinned);
        SetTrayVisible(trayVisible);
        if (compactChanged && m_core)
        {
            RefreshItems();
        }
    }

    bool MainWindow::PersistSetting(
        std::string_view key,
        std::string_view value,
        winrt::hstring const& label)
    {
        if (!m_core || m_settingsReadOnly)
        {
            SetStatus(m_settingsReadOnly
                ? L"当前数据库以只读方式打开，无法保存设置。"
                : L"Rust 核心尚未就绪，无法保存设置。");
            return false;
        }

        try
        {
            (void)m_core->UpdateSetting(key, value);
            std::wstring status{ label.c_str(), label.size() };
            status.append(L"已保存并立即生效。");
            SetStatus(winrt::hstring{ status });
            return true;
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"保存设置失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
            return false;
        }
    }

    void MainWindow::ApplyColorMode(std::string_view mode)
    {
        auto const theme =
            mode == "light" ? ElementTheme::Light
            : mode == "dark" ? ElementTheme::Dark
            : ElementTheme::Default;
        RootGrid().RequestedTheme(theme);
        if (m_hoverPreviewRoot)
        {
            m_hoverPreviewRoot.RequestedTheme(theme);
        }
    }

    void MainWindow::ApplyPinnedWindow(bool pinned)
    {
        m_pinned = pinned;
        auto const wasLoading = m_settingsLoading;
        m_settingsLoading = true;
        PinWindowCheck().IsChecked(pinned);
        if (m_windowPinnedToggle)
        {
            m_windowPinnedToggle.IsOn(pinned);
        }
        m_settingsLoading = wasLoading;
        SetWindowPos(
            GetWindowHandle(),
            pinned ? HWND_TOPMOST : HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }

    void MainWindow::EnsureHoverPreviewWindow()
    {
        if (m_hoverPreviewWindow)
        {
            return;
        }

        m_hoverPreviewWindow = Window();
        m_hoverPreviewWindow.Title(L"TieZ 剪贴板预览");

        m_hoverPreviewRoot = Border();
        m_hoverPreviewRoot.Padding(ThicknessHelper::FromLengths(16, 14, 16, 14));
        m_hoverPreviewRoot.CornerRadius(CornerRadiusHelper::FromUniformRadius(12));
        m_hoverPreviewRoot.BorderThickness(ThicknessHelper::FromUniformLength(1));
        m_hoverPreviewRoot.Background(Application::Current().Resources()
            .Lookup(winrt::box_value(L"CardBackgroundFillColorDefaultBrush")).as<Brush>());
        m_hoverPreviewRoot.BorderBrush(Application::Current().Resources()
            .Lookup(winrt::box_value(L"CardStrokeColorDefaultBrush")).as<Brush>());
        m_hoverPreviewRoot.RequestedTheme(RootGrid().RequestedTheme());

        Grid layout;
        layout.RowSpacing(10);
        RowDefinition titleRow;
        titleRow.Height(GridLengthHelper::Auto());
        RowDefinition contentRow;
        contentRow.Height(GridLength{ 1, GridUnitType::Star });
        layout.RowDefinitions().Append(titleRow);
        layout.RowDefinitions().Append(contentRow);

        m_hoverPreviewTitle = TextBlock();
        m_hoverPreviewTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        layout.Children().Append(m_hoverPreviewTitle);

        ScrollViewer scroller;
        scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
        scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
        Grid::SetRow(scroller, 1);
        StackPanel body;
        body.Spacing(10);
        m_hoverPreviewImage = Image();
        m_hoverPreviewImage.MaxHeight(190);
        m_hoverPreviewImage.Stretch(Stretch::Uniform);
        m_hoverPreviewImage.Visibility(Visibility::Collapsed);
        AutomationProperties::SetName(m_hoverPreviewImage, L"紧凑模式图片预览");
        m_hoverPreviewText = TextBlock();
        m_hoverPreviewText.FontFamily(FontFamily{ L"Consolas" });
        m_hoverPreviewText.TextWrapping(TextWrapping::Wrap);
        m_hoverPreviewText.IsTextSelectionEnabled(false);
        body.Children().Append(m_hoverPreviewImage);
        body.Children().Append(m_hoverPreviewText);
        scroller.Content(body);
        layout.Children().Append(scroller);
        m_hoverPreviewRoot.Child(layout);
        m_hoverPreviewWindow.Content(m_hoverPreviewRoot);

        Window preview = m_hoverPreviewWindow;
        winrt::check_hresult(preview.as<::IWindowNative>()->get_WindowHandle(&m_hoverPreviewHwnd));
        auto style = GetWindowLongPtrW(m_hoverPreviewHwnd, GWL_STYLE);
        style &= ~(WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU);
        style |= WS_POPUP;
        SetWindowLongPtrW(m_hoverPreviewHwnd, GWL_STYLE, style);
        auto extendedStyle = GetWindowLongPtrW(m_hoverPreviewHwnd, GWL_EXSTYLE);
        extendedStyle |= WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        SetWindowLongPtrW(m_hoverPreviewHwnd, GWL_EXSTYLE, extendedStyle);
        SetWindowSubclass(
            m_hoverPreviewHwnd,
            HoverPreviewSubclassProc,
            kHoverPreviewSubclassId,
            0);
        SetWindowPos(
            m_hoverPreviewHwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }

    void MainWindow::ShowHoverPreview(std::int64_t entryId)
    {
        if (!m_compactMode || !m_core)
        {
            return;
        }

        try
        {
            EnsureHoverPreviewWindow();
            auto const value = m_core->Content(entryId);
            auto const content = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const contentType = content.GetNamedString(L"content_type");
            auto const available = content.GetNamedBoolean(L"available");
            auto const isSensitive = content.GetNamedBoolean(L"is_sensitive");

            std::wstringstream title;
            title << ContentTypeLabel(contentType).c_str() << L" · 记录 " << entryId;
            m_hoverPreviewTitle.Text(winrt::hstring{ title.str() });

            winrt::hstring displayContent;
            if (isSensitive)
            {
                displayContent = L"此记录受隐私保护，悬停预览已隐藏。";
            }
            else if (!available)
            {
                displayContent = L"此记录的完整内容当前不可用。";
            }
            else
            {
                displayContent = content.GetNamedString(L"content");
                if (displayContent.empty())
                {
                    auto const htmlContent = content.GetNamedValue(L"html_content");
                    if (htmlContent.ValueType() == JsonValueType::String)
                    {
                        displayContent = htmlContent.GetString();
                    }
                }
            }
            m_hoverPreviewText.Text(displayContent);
            ShowHoverPreviewImage(
                available && !isSensitive ? contentType : winrt::hstring{},
                displayContent);

            POINT cursor{};
            GetCursorPos(&cursor);
            MONITORINFO monitorInfo{};
            monitorInfo.cbSize = sizeof(monitorInfo);
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &monitorInfo);
            auto const scale = static_cast<double>(GetDpiForWindow(GetWindowHandle())) / 96.0;
            auto const width = static_cast<int>(420 * scale);
            auto const height = static_cast<int>(300 * scale);
            auto x = cursor.x + static_cast<int>(18 * scale);
            auto y = cursor.y + static_cast<int>(18 * scale);
            if (x + width > monitorInfo.rcWork.right)
            {
                x = cursor.x - width - static_cast<int>(18 * scale);
            }
            if (y + height > monitorInfo.rcWork.bottom)
            {
                y = monitorInfo.rcWork.bottom - height;
            }
            x = std::max(monitorInfo.rcWork.left, x);
            y = std::max(monitorInfo.rcWork.top, y);
            SetWindowPos(
                m_hoverPreviewHwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        catch (std::exception const& error)
        {
            HideHoverPreview();
            SetStatus(StatusMessage(
                L"悬停预览失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::HideHoverPreview()
    {
        if (m_hoverPreviewHwnd != nullptr && IsWindow(m_hoverPreviewHwnd))
        {
            ShowWindow(m_hoverPreviewHwnd, SW_HIDE);
        }
    }

    void MainWindow::ShowHoverPreviewImage(
        winrt::hstring const& contentType,
        winrt::hstring const& content)
    {
        if (!m_hoverPreviewImage)
        {
            return;
        }
        m_hoverPreviewImage.Source(nullptr);
        m_hoverPreviewImage.Visibility(Visibility::Collapsed);
        if (contentType != L"image")
        {
            return;
        }

        std::wstring path{ content };
        if (path.empty()
            || path.rfind(L"data:image/", 0) == 0
            || !std::filesystem::exists(std::filesystem::path{ content.c_str() }))
        {
            return;
        }
        std::replace(path.begin(), path.end(), L'\\', L'/');
        Microsoft::UI::Xaml::Media::Imaging::BitmapImage bitmap;
        bitmap.UriSource(Windows::Foundation::Uri{ L"file:///" + path });
        m_hoverPreviewImage.Source(bitmap);
        m_hoverPreviewImage.Visibility(Visibility::Visible);
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
        TagsTextBox().TextCompositionStarted([this](auto const&, auto const&)
        {
            m_imeComposing = true;
        });
        TagsTextBox().TextCompositionEnded([this](auto const&, auto const&)
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

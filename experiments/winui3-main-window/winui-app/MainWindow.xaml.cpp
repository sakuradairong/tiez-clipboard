#include "pch.h"
#include "MainWindow.xaml.h"

#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif

#include <microsoft.ui.xaml.window.h>

namespace
{
    winrt::hstring StatusMessage(
        std::wstring_view prefix,
        winrt::hstring const& detail)
    {
        std::wstring message{ prefix };
        message.append(detail.c_str(), detail.size());
        return winrt::hstring{ message };
    }

    Microsoft::UI::Xaml::Controls::Button ActionButton(
        winrt::hstring const& label,
        std::function<void()> action)
    {
        Microsoft::UI::Xaml::Controls::Button button;
        button.Content(winrt::box_value(label));
        button.Click([action = std::move(action)](auto const&, auto const&)
        {
            action();
        });
        return button;
    }
}

namespace winrt::Tiez::WinUIProbe::implementation
{
    using namespace Microsoft::UI::Xaml;
    using namespace Microsoft::UI::Xaml::Controls;
    using namespace Windows::Data::Json;

    MainWindow::MainWindow()
    {
        InitializeComponent();
        Title(L"TieZ · WinUI 3 main-window experiment");
        SetInitialWindowSize();
        SearchBox().Focus(FocusState::Programmatic);

        try
        {
            m_core = std::make_unique<tiez::probe::RustCoreBridge>();
            RefreshItems();
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"Startup failed: ",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::SearchBox_TextChanged(TextBox const&, TextChangedEventArgs const&)
    {
        if (m_core)
        {
            RefreshItems();
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
                ShowWindow(GetWindowHandle(), SW_SHOW);
                Activate();
                SetStatus(L"Window restored after 5 seconds; the Rust core remained in-process.");
            });
        }

        SetStatus(L"Window hidden for 5 seconds. Sample the process now to compare idle memory.");
        m_showTimer.Start();
        ShowWindow(GetWindowHandle(), SW_HIDE);
    }

    void MainWindow::RefreshItems()
    {
        if (!m_core)
        {
            return;
        }

        try
        {
            auto const snapshot = m_core->Snapshot(winrt::to_string(SearchBox().Text()));
            auto const root = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(snapshot));
            auto const items = root.GetNamedArray(L"items");

            ItemsPanel().Children().Clear();
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                ItemsPanel().Children().Append(CreateItemCard(items.GetObjectAt(index)));
            }

            EmptyState().Visibility(items.Size() == 0 ? Visibility::Visible : Visibility::Collapsed);

            std::wstringstream status;
            status << L"Rust ABI " << static_cast<std::uint32_t>(root.GetNamedNumber(L"abi_version"))
                   << L" · generation " << static_cast<std::uint64_t>(root.GetNamedNumber(L"generation"))
                   << L" · " << items.Size() << L" visible entries · "
                   << root.GetNamedString(L"last_action").c_str();
            SetStatus(status.str());
            WriteReadyMarker();
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"Refresh failed: ", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"Refresh failed: ",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    UIElement MainWindow::CreateItemCard(JsonObject const& item)
    {
        auto const entryId = static_cast<std::int64_t>(item.GetNamedNumber(L"id"));
        auto const isPinned = item.GetNamedBoolean(L"is_pinned");

        Border card;
        card.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"ClipboardCardStyle")).as<Style>());

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
        type.Text(item.GetNamedString(L"content_type"));
        type.FontWeight(Windows::UI::Text::FontWeights::SemiBold());

        TextBlock source;
        source.Text(item.GetNamedString(L"source_app"));
        source.Opacity(0.72);
        Grid::SetColumn(source, 1);

        TextBlock capturedAt;
        capturedAt.Text(item.GetNamedString(L"captured_at"));
        capturedAt.Opacity(0.72);
        Grid::SetColumn(capturedAt, 2);

        metadata.Children().Append(type);
        metadata.Children().Append(source);
        metadata.Children().Append(capturedAt);

        TextBlock preview;
        preview.Text(item.GetNamedString(L"preview"));
        preview.TextWrapping(TextWrapping::WrapWholeWords);
        preview.IsTextSelectionEnabled(true);
        preview.MaxHeight(112);

        StackPanel actions;
        actions.Orientation(Orientation::Horizontal);
        actions.Spacing(8);
        actions.Children().Append(ActionButton(
            isPinned ? L"Unpin" : L"Pin",
            [this, entryId] { ApplyAction(entryId, "pin"); }));
        actions.Children().Append(ActionButton(
            L"Paste plain",
            [this, entryId] { ApplyAction(entryId, "paste-plain"); }));
        actions.Children().Append(ActionButton(
            L"Paste rich",
            [this, entryId] { ApplyAction(entryId, "paste-rich"); }));
        actions.Children().Append(ActionButton(
            L"Delete",
            [this, entryId] { ApplyAction(entryId, "delete"); }));

        content.Children().Append(metadata);
        content.Children().Append(preview);
        content.Children().Append(actions);
        card.Child(content);
        return card;
    }

    void MainWindow::ApplyAction(std::int64_t entryId, std::string_view action)
    {
        try
        {
            m_core->ApplyAction(entryId, action);
            RefreshItems();
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"Action failed: ",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
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
}

#pragma once

#include "MainWindow.g.h"
#include "RustCoreBridge.h"

namespace winrt::Tiez::WinUIProbe::implementation
{
    struct MainWindow : MainWindowT<MainWindow>
    {
        MainWindow();
        ~MainWindow();

        void SearchBox_TextChanged(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Controls::TextChangedEventArgs const&);
        void SearchBox_KeyDown(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Input::KeyRoutedEventArgs const& args);
        void RootGrid_KeyDown(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Input::KeyRoutedEventArgs const& args);
        void RefreshButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void HideButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void SettingsButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void PinWindowCheck_Changed(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TypeAllButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TypeChip_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TagsTextBox_KeyDown(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Input::KeyRoutedEventArgs const& args);
        void SaveTagsButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void OpenSelectedButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void OnToggleHotkey();
        bool OnNativeMessage(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam);

    private:
        void RefreshItems();
        void ShowContent(std::int64_t entryId);
        void OpenEntry(std::int64_t entryId);
        void LaunchOpenPlan(Windows::Data::Json::JsonObject const& plan);
        void ApplyAction(std::int64_t entryId, std::string_view action);
        Microsoft::UI::Xaml::UIElement CreateItemCard(
            Windows::Data::Json::JsonObject const& item,
            bool readOnly,
            std::uint32_t index);
        void SetStatus(winrt::hstring const& message);
        void WriteReadyMarker();
        HWND GetWindowHandle();
        void SetInitialWindowSize();
        void SetupLifecycle();
        void TeardownLifecycle();
        void SetupTrayIcon();
        void AddTrayIcon();
        void RemoveTrayIcon();
        void SetTrayVisible(bool visible);
        void ShowTrayMenu();
        void RequestExit();
        void HideMainWindow();
        void ShowMainWindow(bool captureForeground);
        void PreparePasteTarget();
        bool HandleNavigationKey(Windows::System::VirtualKey key);
        bool SearchBoxHasFocus();
        void AttachCardCommands(
            Microsoft::UI::Xaml::Controls::Border const& card,
            std::int64_t entryId,
            bool readOnly,
            bool isSensitive);
        void AttachPinnedReorder(
            Microsoft::UI::Xaml::Controls::Border const& card,
            std::int64_t entryId,
            bool enabled);
        void MoveSelection(int delta);
        void MovePinnedEntry(std::int64_t entryId, int delta);
        void DropPinnedEntry(std::int64_t sourceId, std::int64_t targetId, bool afterTarget);
        void PersistPinnedOrder();
        void UpdateSelectionVisuals();
        std::string CurrentQuery();
        void SaveSelectedTags();
        void SelectEntry(std::int64_t entryId);
        void SetTypeFilter(std::string filter);
        void SetupImeGuards();
        void EnsureSettingsDialog();
        void LoadSettings();
        bool PersistSetting(
            std::string_view key,
            std::string_view value,
            winrt::hstring const& label);
        void ApplyColorMode(std::string_view mode);
        void ApplyPinnedWindow(bool pinned);
        void EnsureHoverPreviewWindow();
        void ShowHoverPreview(std::int64_t entryId);
        void HideHoverPreview();
        void ShowHoverPreviewImage(
            winrt::hstring const& contentType,
            winrt::hstring const& content);
        void ShowDetailsImage(winrt::hstring const& contentType, winrt::hstring const& content);
        static void __cdecl OnHistoryChanged(void* userData, std::uint64_t generation);

        struct HistoryRefreshSink : std::enable_shared_from_this<HistoryRefreshSink>
        {
            std::mutex mutex;
            MainWindow* window{};
            Microsoft::UI::Dispatching::DispatcherQueue dispatcher{ nullptr };
        };

        std::unique_ptr<tiez::probe::RustCoreBridge> m_core;
        std::shared_ptr<HistoryRefreshSink> m_refreshSink;
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_showTimer{ nullptr };
        Microsoft::UI::Xaml::Controls::ContentDialog m_settingsDialog{ nullptr };
        Microsoft::UI::Xaml::Controls::StackPanel m_settingsPanel{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_colorModeCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_compactModeToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_persistentToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_persistentLimitEnabledToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::NumberBox m_persistentLimitNumber{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_deduplicateToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_captureFilesToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_captureRichTextToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_privacyProtectionToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_trayVisibleToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_windowPinnedToggle{ nullptr };
        Microsoft::UI::Xaml::Window m_hoverPreviewWindow{ nullptr };
        Microsoft::UI::Xaml::Controls::Border m_hoverPreviewRoot{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_hoverPreviewTitle{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_hoverPreviewText{ nullptr };
        Microsoft::UI::Xaml::Controls::Image m_hoverPreviewImage{ nullptr };
        HWND m_hwnd{};
        HWND m_hoverPreviewHwnd{};
        HWND m_hotkeyHwnd{};
        HWND m_lastHwnd{};
        HICON m_trayIcon{};
        UINT m_taskbarCreatedMessage{};
        bool m_readyMarkerWritten{};
        bool m_pinned{};
        bool m_suspendLifecycle{};
        bool m_trayAdded{};
        bool m_exitRequested{};
        bool m_imeComposing{};
        bool m_ignoreNextEnter{};
        bool m_readOnly{};
        bool m_settingsReadOnly{};
        bool m_settingsLoading{};
        bool m_compactMode{};
        bool m_trayVisible{ true };
        bool m_canReorderPinned{};
        std::string m_typeFilter;
        std::vector<std::int64_t> m_entryIds;
        std::vector<std::int64_t> m_pinnedIds;
        std::vector<Microsoft::UI::Xaml::Controls::Border> m_cards;
        std::unordered_map<std::int64_t, std::vector<winrt::hstring>> m_tagsById;
        std::optional<std::int64_t> m_detailsEntryId;
        std::optional<std::int64_t> m_draggedPinnedId;
        int m_selectedIndex{-1};
    };
}

namespace winrt::Tiez::WinUIProbe::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}

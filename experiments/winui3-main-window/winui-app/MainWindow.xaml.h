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
            Microsoft::UI::Xaml::Controls::TextBox const&,
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
        void PinWindowCheck_Changed(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TypeAllButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TypeChip_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void OnToggleHotkey();

    private:
        void RefreshItems();
        void ShowContent(std::int64_t entryId);
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
        void HideMainWindow();
        void ShowMainWindow(bool captureForeground);
        void PreparePasteTarget();
        bool HandleNavigationKey(Windows::System::VirtualKey key);
        void MoveSelection(int delta);
        void UpdateSelectionVisuals();
        std::string CurrentQuery() const;
        void SetTypeFilter(std::string filter);

        std::unique_ptr<tiez::probe::RustCoreBridge> m_core;
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_showTimer{ nullptr };
        HWND m_hwnd{};
        HWND m_hotkeyHwnd{};
        HWND m_lastHwnd{};
        bool m_readyMarkerWritten{};
        bool m_pinned{};
        bool m_suspendLifecycle{};
        std::string m_typeFilter;
        std::vector<std::int64_t> m_entryIds;
        std::vector<Microsoft::UI::Xaml::Controls::Border> m_cards;
        int m_selectedIndex{-1};
    };
}

namespace winrt::Tiez::WinUIProbe::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}

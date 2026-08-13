#pragma once

#include "MainWindow.g.h"
#include "RustCoreBridge.h"

namespace winrt::Tiez::WinUIProbe::implementation
{
    struct MainWindow : MainWindowT<MainWindow>
    {
        MainWindow();

        void SearchBox_TextChanged(
            Microsoft::UI::Xaml::Controls::TextBox const&,
            Microsoft::UI::Xaml::Controls::TextChangedEventArgs const&);
        void RefreshButton_Click(
            winrt::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void HideButton_Click(
            winrt::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);

    private:
        void RefreshItems();
        void ApplyAction(std::int64_t entryId, std::string_view action);
        Microsoft::UI::Xaml::UIElement CreateItemCard(
            Windows::Data::Json::JsonObject const& item);
        void SetStatus(winrt::hstring const& message);
        void WriteReadyMarker();
        HWND GetWindowHandle();
        void SetInitialWindowSize();

        std::unique_ptr<tiez::probe::RustCoreBridge> m_core;
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_showTimer{ nullptr };
        HWND m_hwnd{};
        bool m_readyMarkerWritten{};
    };
}

namespace winrt::Tiez::WinUIProbe::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}

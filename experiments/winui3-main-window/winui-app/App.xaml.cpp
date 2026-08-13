#include "pch.h"

#include "App.xaml.h"
#include "MainWindow.xaml.h"

namespace winrt::Tiez::WinUIProbe::implementation
{
    App::App()
    {
        InitializeComponent();

#if defined(_DEBUG) && !defined(DISABLE_XAML_GENERATED_BREAK_ON_UNHANDLED_EXCEPTION)
        UnhandledException([](winrt::IInspectable const&, winrt::UnhandledExceptionEventArgs const& event)
        {
            if (IsDebuggerPresent())
            {
                auto message = event.Message();
                __debugbreak();
            }
        });
#endif
    }

    void App::OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const&)
    {
        m_window = winrt::make<MainWindow>();
        m_window.Activate();
    }
}

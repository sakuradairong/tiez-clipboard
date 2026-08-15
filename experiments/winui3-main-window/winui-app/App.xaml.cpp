#include "pch.h"

#include "App.xaml.h"
#include "MainWindow.xaml.h"

#include <microsoft.ui.xaml.window.h>
#include <shellapi.h>

namespace
{
    bool ShouldStartHidden()
    {
        try
        {
            auto const activation =
                winrt::Microsoft::Windows::AppLifecycle::AppInstance::GetCurrent()
                    .GetActivatedEventArgs();
            if (activation
                && activation.Kind()
                    == winrt::Microsoft::Windows::AppLifecycle::ExtendedActivationKind::StartupTask)
            {
                return true;
            }
        }
        catch (winrt::hresult_error const&)
        {
            // Unpackaged development builds have no AppLifecycle startup activation.
        }

        int argumentCount{};
        auto* arguments = CommandLineToArgvW(GetCommandLineW(), &argumentCount);
        if (arguments == nullptr)
        {
            return false;
        }

        bool startHidden{};
        for (int index = 1; index < argumentCount; ++index)
        {
            if (_wcsicmp(arguments[index], L"--autostart") == 0
                || _wcsicmp(arguments[index], L"--minimized") == 0)
            {
                startHidden = true;
                break;
            }
        }
        LocalFree(arguments);
        return startHidden;
    }
}

namespace winrt::Tiez::WinUIProbe::implementation
{
    App::App()
    {
        InitializeComponent();

#if defined(_DEBUG) && !defined(DISABLE_XAML_GENERATED_BREAK_ON_UNHANDLED_EXCEPTION)
        UnhandledException([](
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::UnhandledExceptionEventArgs const& event)
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
        auto const startHidden = ShouldStartHidden();
        m_window = winrt::make<MainWindow>(startHidden);
        m_window.Activate();
        if (startHidden)
        {
            HWND hwnd{};
            winrt::check_hresult(m_window.as<::IWindowNative>()->get_WindowHandle(&hwnd));
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

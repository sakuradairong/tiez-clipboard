#include "pch.h"

#include "App.xaml.h"
#include "MainWindow.xaml.h"

#include <microsoft.ui.xaml.window.h>
#include <shellapi.h>
#include <winrt/Microsoft.UI.Windowing.h>

namespace
{
    constexpr wchar_t kMainInstanceKey[] = L"TieZ.MainWindow.v1";

    winrt::Microsoft::Windows::AppLifecycle::AppInstance g_primaryInstance{ nullptr };
    winrt::Microsoft::Windows::AppLifecycle::AppActivationArguments
        g_initialActivation{ nullptr };

    void TraceRoutingFailure(winrt::hstring const& detail)
    {
        std::wstring message{ L"TieZ instance routing failed: " };
        message.append(detail.c_str(), detail.size());
        message.push_back(L'\n');
        OutputDebugStringW(message.c_str());
    }

    bool ArgumentsRequestHidden(std::wstring_view arguments)
    {
        if (arguments.empty())
        {
            return false;
        }

        std::wstring commandLine{ arguments };
        int argumentCount{};
        auto* parsed = CommandLineToArgvW(commandLine.c_str(), &argumentCount);
        if (parsed == nullptr)
        {
            return false;
        }

        bool startHidden{};
        for (int index = 0; index < argumentCount; ++index)
        {
            if (_wcsicmp(parsed[index], L"--autostart") == 0
                || _wcsicmp(parsed[index], L"--minimized") == 0)
            {
                startHidden = true;
                break;
            }
        }
        LocalFree(parsed);
        return startHidden;
    }

    bool ActivationRequestsHidden(
        winrt::Microsoft::Windows::AppLifecycle::AppActivationArguments const& activation,
        bool inspectCurrentCommandLine)
    {
        using namespace winrt::Microsoft::Windows::AppLifecycle;
        using winrt::Windows::ApplicationModel::Activation::ILaunchActivatedEventArgs;

        if (activation)
        {
            if (activation.Kind() == ExtendedActivationKind::StartupTask)
            {
                return true;
            }
            if (activation.Kind() == ExtendedActivationKind::Launch)
            {
                auto const launch = activation.Data().try_as<ILaunchActivatedEventArgs>();
                if (launch && ArgumentsRequestHidden(launch.Arguments().c_str()))
                {
                    return true;
                }
            }
        }

        return inspectCurrentCommandLine
            && ArgumentsRequestHidden(GetCommandLineW());
    }

    bool RouteInitialActivation()
    {
        try
        {
            auto const current =
                winrt::Microsoft::Windows::AppLifecycle::AppInstance::GetCurrent();
            auto const activation = current.GetActivatedEventArgs();
            auto const primary =
                winrt::Microsoft::Windows::AppLifecycle::AppInstance::FindOrRegisterForKey(
                    kMainInstanceKey);
            if (primary.IsCurrent())
            {
                g_primaryInstance = primary;
                g_initialActivation = activation;
                return true;
            }

            // Give the registered process permission to take foreground focus
            // before the activation crosses the process boundary.
            (void)AllowSetForegroundWindow(static_cast<DWORD>(primary.ProcessId()));

            winrt::handle redirectComplete{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
            winrt::check_bool(redirectComplete.get());
            auto const operation = primary.RedirectActivationToAsync(activation);
            operation.Completed(
                [completion = redirectComplete.get()](auto const&, auto const&)
                {
                    SetEvent(completion);
                });

            auto completion = redirectComplete.get();
            DWORD completedIndex{};
            winrt::check_hresult(CoWaitForMultipleHandles(
                COWAIT_DISPATCH_CALLS | COWAIT_DISPATCH_WINDOW_MESSAGES,
                INFINITE,
                1,
                &completion,
                &completedIndex));
            operation.GetResults();
            return false;
        }
        catch (winrt::hresult_error const& error)
        {
            TraceRoutingFailure(error.message());
        }
        catch (std::exception const& error)
        {
            TraceRoutingFailure(winrt::to_hstring(error.what()));
        }
        catch (...)
        {
            TraceRoutingFailure(L"unknown error");
        }

        // Falling through keeps a visible failure path through the existing
        // Rust database-ownership guard instead of silently dropping a launch.
        return true;
    }
}

int __stdcall wWinMain(HINSTANCE, HINSTANCE, PWSTR, int)
{
    winrt::init_apartment(winrt::apartment_type::single_threaded);
    if (!RouteInitialActivation())
    {
        return 0;
    }

    winrt::Microsoft::UI::Xaml::Application::Start(
        [](auto&&)
        {
            winrt::make<winrt::Tiez::WinUIProbe::implementation::App>();
        });
    return 0;
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

    App::~App()
    {
        try
        {
            if (m_primaryInstance && m_hasActivationHandler)
            {
                m_primaryInstance.Activated(m_activationToken);
            }
            if (m_primaryInstance && m_primaryInstance.IsCurrent())
            {
                m_primaryInstance.UnregisterKey();
            }
        }
        catch (...)
        {
            // Process teardown must not be interrupted by AppLifecycle cleanup.
        }
    }

    void App::RegisterForRedirectedActivations()
    {
        m_primaryInstance = g_primaryInstance;
        m_dispatcher = Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread();
        if (!m_primaryInstance || !m_dispatcher)
        {
            return;
        }

        auto const weak = get_weak();
        auto const dispatcher = m_dispatcher;
        m_activationToken = m_primaryInstance.Activated(
            [weak, dispatcher](auto const&, auto const& activation)
            {
                bool showWindow{ true };
                try
                {
                    showWindow = !ActivationRequestsHidden(activation, false);
                }
                catch (...)
                {
                    // Unknown future activation kinds should still reveal TieZ.
                }
                if (!showWindow)
                {
                    return;
                }

                dispatcher.TryEnqueue([weak]()
                {
                    if (auto const app = weak.get())
                    {
                        app->ShowMainWindowFromActivation();
                    }
                });
            });
        m_hasActivationHandler = true;
    }

    void App::ShowMainWindowFromActivation()
    {
        if (!m_window)
        {
            return;
        }

        HWND hwnd{};
        winrt::check_hresult(m_window.as<::IWindowNative>()->get_WindowHandle(&hwnd));
        ShowWindow(hwnd, SW_RESTORE);
        m_window.Activate();
        SetForegroundWindow(hwnd);
    }

    void App::OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const&)
    {
        RegisterForRedirectedActivations();
        auto activation = g_initialActivation;
        if (!activation)
        {
            try
            {
                activation = Microsoft::Windows::AppLifecycle::AppInstance::GetCurrent()
                    .GetActivatedEventArgs();
            }
            catch (...)
            {
                // Command-line fallback below still preserves unpackaged startup.
            }
        }
        auto startHidden = ArgumentsRequestHidden(GetCommandLineW());
        try
        {
            startHidden = ActivationRequestsHidden(activation, true);
        }
        catch (...)
        {
            // Keep the command-line fallback for unpackaged or future activations.
        }
        m_window = winrt::make<MainWindow>(startHidden);
        if (startHidden)
        {
            m_window.AppWindow().Hide();
        }
        else
        {
            m_window.Activate();
        }
    }
}

#pragma once

#include "App.xaml.g.h"
#include "pch.h"

namespace winrt::Tiez::WinUIProbe::implementation
{
    struct App : AppT<App>
    {
        App();
        ~App();
        void OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const&);

    private:
        void RegisterForRedirectedActivations();
        void ShowMainWindowFromActivation();

        Microsoft::Windows::AppLifecycle::AppInstance m_primaryInstance{ nullptr };
        Microsoft::UI::Dispatching::DispatcherQueue m_dispatcher{ nullptr };
        Microsoft::UI::Xaml::Window m_window{ nullptr };
        winrt::event_token m_activationToken{};
        bool m_hasActivationHandler{};
    };
}

#pragma once

#include "MainWindow.g.h"
#include "RustCoreBridge.h"

namespace winrt::Tiez::WinUIProbe::implementation
{
    struct MainWindow : MainWindowT<MainWindow>
    {
        MainWindow();
        explicit MainWindow(bool startHidden);
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
        void ClearHistoryButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void EmojiButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void TagButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void FileTransferButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void AiButton_Click(
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
        void AnalyzeImageButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void CopyImageAnalysisButton_Click(
            winrt::Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void OnToggleHotkey();
        bool OnNativeMessage(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam);

    private:
        void RefreshItems();
        void ShowContent(std::int64_t entryId);
        winrt::fire_and_forget AnalyzeSelectedImageAsync();
        void ShowImageAnalysis(Windows::Data::Json::JsonObject const& response);
        void SetImageAnalysisBusy(bool busy, winrt::hstring const& message);
        void OpenEntry(std::int64_t entryId);
        void LaunchOpenPlan(Windows::Data::Json::JsonObject const& plan);
        void ApplyAction(std::int64_t entryId, std::string_view action);
        void PasteTransientText(winrt::hstring const& text);
        void PasteFavoriteImage(
            winrt::hstring const& path,
            winrt::hstring const& fileName);
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
        bool ApplyToggleHotkey(winrt::hstring const& configuredHotkey);
        void SaveToggleHotkey();
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
        winrt::fire_and_forget ConfirmClearHistoryAsync();
        winrt::fire_and_forget ShowEmojiPickerAsync();
        winrt::fire_and_forget ShowTagManagerAsync();
        winrt::fire_and_forget ShowFileTransferAsync();
        winrt::fire_and_forget LoadFileTransferQrAsync(winrt::hstring base64Png);
        void EnsureFileTransferDialog();
        void RefreshFileTransfer();
        void ApplyFileTransferSnapshot(Windows::Data::Json::JsonObject const& response);
        winrt::fire_and_forget SaveFileTransferSettings(bool enabled);
        winrt::fire_and_forget SendFileTransferText();
        winrt::fire_and_forget ShareFileTransferFiles();
        void EnsureAiDialog();
        winrt::fire_and_forget ShowAiAssistantAsync();
        bool LoadAiSettings();
        void ApplyAiSettingsSnapshot(Windows::Data::Json::JsonObject const& response);
        void LoadAiProfileEditor();
        winrt::fire_and_forget SaveAiSettings(bool deleteCurrent = false);
        winrt::fire_and_forget ProbeAiProfileAsync();
        winrt::fire_and_forget RunAiActionAsync();
        void SetAiBusy(bool busy, winrt::hstring const& message);
        winrt::Windows::Foundation::IAsyncOperation<winrt::hstring> RunRustOperationAsync(
            std::function<std::string()> operation);
        void EnsureSettingsDialog();
        void LoadSettings();
        winrt::fire_and_forget RefreshAutostartStateAsync(bool reconcilePreference);
        winrt::fire_and_forget ApplyAutostartAsync(bool enabled);
        void SetAutostartUi(
            bool enabled,
            bool canChange,
            winrt::hstring const& message);
        void LoadCloudSyncSettings();
        bool SaveCloudSyncSettings(bool clearPassword, bool reloadRunner = true);
        void RequestCloudSyncNow();
        void UpdateCloudSyncStatus();
        winrt::fire_and_forget ProbeCloudSyncAsync();
        void SetCloudSyncBusy(bool busy, winrt::hstring const& message);
        winrt::fire_and_forget ExportBackupAsync();
        winrt::fire_and_forget RestoreBackupAsync();
        void SetBackupBusy(bool busy, winrt::hstring const& message);
        winrt::fire_and_forget CheckForUpdatesAsync();
        winrt::fire_and_forget OpenAppInstallerAsync();
        void SetUpdateBusy(bool busy, winrt::hstring const& message);
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
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_cloudSyncStatusTimer{ nullptr };
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_fileTransferTimer{ nullptr };
        Microsoft::UI::Xaml::Controls::ContentDialog m_fileTransferDialog{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_fileTransferStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::Image m_fileTransferQrImage{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_fileTransferUrlText{ nullptr };
        Microsoft::UI::Xaml::Controls::NumberBox m_fileTransferPortNumber{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_fileTransferPathText{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_fileTransferAutoCopyToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_fileTransferAutoOpenToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_fileTransferAutoCloseToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferToggleButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferSaveButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferCopyUrlButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferOpenUrlButton{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_fileTransferMessageText{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferSendButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_fileTransferShareButton{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_fileTransferDevicesText{ nullptr };
        Microsoft::UI::Xaml::Controls::StackPanel m_fileTransferMessagesPanel{ nullptr };
        struct AiProfileState
        {
            winrt::hstring id;
            winrt::hstring baseUrl;
            winrt::hstring model;
            bool enableThinking{};
            bool apiKeyConfigured{};
        };
        Microsoft::UI::Xaml::Controls::ContentDialog m_aiDialog{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_aiStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_aiSelectionText{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiActionCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiRunButton{ nullptr };
        Microsoft::UI::Xaml::Controls::ProgressRing m_aiProgress{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_aiResultText{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiCopyButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiPasteButton{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_aiEnabledToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiProfileCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiNewProfileButton{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_aiBaseUrlText{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_aiModelText{ nullptr };
        Microsoft::UI::Xaml::Controls::PasswordBox m_aiApiKeyBox{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_aiKeyStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_aiThinkingToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiTaskProfileCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiMouthpieceProfileCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiTranslateProfileCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::ComboBox m_aiTargetLanguageCombo{ nullptr };
        Microsoft::UI::Xaml::Controls::NumberBox m_aiThinkingBudgetNumber{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiSaveButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiDeleteButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_aiProbeButton{ nullptr };
        std::vector<AiProfileState> m_aiProfiles;
        winrt::hstring m_aiCurrentProfileId;
        bool m_aiReadOnly{};
        bool m_aiLoading{};
        bool m_aiBusy{};
        bool m_aiAddingProfile{};
        bool m_aiEnabled{};
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
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_autostartToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_autostartStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_hotkeyEditor{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_hotkeyApplyButton{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_hotkeySettingsStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncEnabledToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncAutoToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_cloudSyncUrlText{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_cloudSyncUsernameText{ nullptr };
        Microsoft::UI::Xaml::Controls::PasswordBox m_cloudSyncPasswordBox{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBox m_cloudSyncBasePathText{ nullptr };
        Microsoft::UI::Xaml::Controls::NumberBox m_cloudSyncIntervalNumber{ nullptr };
        Microsoft::UI::Xaml::Controls::NumberBox m_cloudSyncSnapshotIntervalNumber{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncTextToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncImageToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncFileToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::ToggleSwitch m_cloudSyncEmojiToggle{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_cloudSyncSaveButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_cloudSyncNowButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_cloudSyncProbeButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_cloudSyncClearPasswordButton{ nullptr };
        Microsoft::UI::Xaml::Controls::ProgressRing m_cloudSyncProgress{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_cloudSyncStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_exportBackupButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_restoreBackupButton{ nullptr };
        Microsoft::UI::Xaml::Controls::ProgressRing m_backupProgress{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_backupStatus{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_checkUpdateButton{ nullptr };
        Microsoft::UI::Xaml::Controls::Button m_installUpdateButton{ nullptr };
        Microsoft::UI::Xaml::Controls::ProgressRing m_updateProgress{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_updateStatus{ nullptr };
        Microsoft::UI::Xaml::Window m_hoverPreviewWindow{ nullptr };
        Microsoft::UI::Xaml::Controls::Border m_hoverPreviewRoot{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_hoverPreviewTitle{ nullptr };
        Microsoft::UI::Xaml::Controls::TextBlock m_hoverPreviewText{ nullptr };
        Microsoft::UI::Xaml::Controls::Image m_hoverPreviewImage{ nullptr };
        HWND m_hwnd{};
        HWND m_hoverPreviewHwnd{};
        HWND m_hotkeyHwnd{};
        HWND m_lastHwnd{};
        HHOOK m_mouseHotkeyHook{};
        HICON m_trayIcon{};
        UINT m_hotkeyModifiers{};
        UINT m_hotkeyVirtualKey{};
        UINT m_taskbarCreatedMessage{};
        bool m_readyMarkerWritten{};
        bool m_pinned{};
        bool m_suspendLifecycle{};
        bool m_trayAdded{};
        bool m_hotkeyRegistered{};
        bool m_exitRequested{};
        bool m_imeComposing{};
        bool m_ignoreNextEnter{};
        bool m_readOnly{};
        bool m_settingsReadOnly{};
        bool m_settingsLoading{};
        bool m_autostartBusy{};
        bool m_autostartPreference{ true };
        bool m_startHidden{};
        bool m_initialHidePending{};
        bool m_cloudSyncBusy{};
        bool m_fileTransferRunning{};
        bool m_fileTransferReadOnly{};
        bool m_fileTransferLoading{};
        bool m_cloudSyncPasswordConfigured{};
        std::uint64_t m_cloudSyncSettingsRevision{};
        bool m_backupBusy{};
        bool m_updateBusy{};
        bool m_updateAvailable{};
        bool m_imageAnalysisBusy{};
        bool m_imageAnalysisLoaded{};
        bool m_productionData{};
        bool m_compactMode{};
        bool m_trayVisible{ true };
        bool m_canReorderPinned{};
        std::string m_typeFilter;
        winrt::hstring m_configuredHotkey{ L"Alt+C" };
        winrt::hstring m_registeredHotkey;
        winrt::hstring m_fileTransferQrBase64;
        std::vector<std::int64_t> m_entryIds;
        std::vector<std::int64_t> m_pinnedIds;
        std::vector<Microsoft::UI::Xaml::Controls::Border> m_cards;
        std::unordered_map<std::int64_t, std::vector<winrt::hstring>> m_tagsById;
        std::optional<std::int64_t> m_detailsEntryId;
        std::optional<std::int64_t> m_imageAnalysisEntryId;
        std::optional<std::int64_t> m_draggedPinnedId;
        std::wstring m_imageAnalysisCopyText;
        winrt::hstring m_appInstallerUri;
        int m_selectedIndex{-1};
    };
}

namespace winrt::Tiez::WinUIProbe::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}

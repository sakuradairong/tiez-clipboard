#include "pch.h"
#include "RustCoreBridge.h"

namespace
{
    std::string Win32ErrorMessage(DWORD error)
    {
        wchar_t* buffer = nullptr;
        FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            nullptr,
            error,
            0,
            reinterpret_cast<wchar_t*>(&buffer),
            0,
            nullptr);

        std::wstring message = buffer == nullptr ? L"unknown error" : buffer;
        if (buffer != nullptr)
        {
            LocalFree(buffer);
        }

        return winrt::to_string(message);
    }
}

namespace tiez::probe
{
    RustCoreBridge::RustCoreBridge()
    {
        auto const path = ResolveDllPath();
        m_library = LoadLibraryW(path.c_str());
        if (m_library == nullptr)
        {
            throw std::runtime_error(
                "Failed to load " + path.string() + ": " + Win32ErrorMessage(GetLastError()));
        }

        try
        {
            m_abiVersion = Resolve<AbiVersionFn>("tiez_core_abi_version");
            m_create = Resolve<CreateFn>("tiez_core_create");
            m_destroy = Resolve<DestroyFn>("tiez_core_destroy");
            m_snapshot = Resolve<SnapshotFn>("tiez_core_get_snapshot_json");
            m_content = Resolve<ContentFn>("tiez_core_get_content_json");
            m_imageAnalysis = Resolve<ContentFn>("tiez_core_get_image_analysis_json");
            m_analyzeImage = Resolve<AnalyzeImageFn>("tiez_core_analyze_image_json");
            m_prepareOpenContent = Resolve<PrepareOpenContentFn>(
                "tiez_core_prepare_open_content_json");
            m_createBackup = Resolve<PathJsonFn>("tiez_core_create_backup_json");
            m_inspectBackup = Resolve<PathJsonFn>("tiez_core_inspect_backup_json");
            m_scheduleRestore = Resolve<PathJsonFn>("tiez_core_schedule_restore_json");
            m_applyActionJson = Resolve<ApplyActionJsonFn>("tiez_core_apply_action_json");
            m_pasteText = Resolve<TextActionFn>("tiez_core_paste_text");
            m_emojiFavoritesJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_emoji_favorites_json");
            m_importEmojiFavoriteJson = Resolve<PathJsonFn>(
                "tiez_core_import_emoji_favorite_json");
            m_removeEmojiFavoriteJson = Resolve<PathJsonFn>(
                "tiez_core_remove_emoji_favorite_json");
            m_pasteEmojiFavorite = Resolve<TextActionFn>(
                "tiez_core_paste_emoji_favorite");
            m_tagCatalogJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_tag_catalog_json");
            m_tagEntriesJson = Resolve<PathJsonFn>(
                "tiez_core_get_tag_entries_json");
            m_createTagJson = Resolve<PathJsonFn>(
                "tiez_core_create_tag_json");
            m_renameTagJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_rename_tag_json");
            m_deleteTagJson = Resolve<PathJsonFn>(
                "tiez_core_delete_tag_json");
            m_setTagColorJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_set_tag_color_json");
            m_createTaggedTextJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_create_tagged_text_json");
            m_updateTagsJson = Resolve<UpdateTagsJsonFn>("tiez_core_update_tags_json");
            m_updatePinnedOrderJson = Resolve<UpdatePinnedOrderJsonFn>(
                "tiez_core_update_pinned_order_json");
            m_settingsJson = Resolve<SettingsJsonFn>("tiez_core_get_settings_json");
            m_updateSettingJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_update_setting_json");
            m_aiSettingsJson = Resolve<SettingsJsonFn>("tiez_core_get_ai_settings_json");
            m_updateAiSettingsJson = Resolve<JsonRequestFn>(
                "tiez_core_update_ai_settings_json");
            m_probeAiProfileJson = Resolve<JsonRequestFn>(
                "tiez_core_probe_ai_profile_json");
            m_runAiActionJson = Resolve<ApplyActionJsonFn>(
                "tiez_core_run_ai_action_json");
            m_cloudSyncSettingsJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_cloud_sync_settings_json");
            m_updateCloudSyncSettingsJson = Resolve<JsonRequestFn>(
                "tiez_core_update_cloud_sync_settings_json");
            m_probeCloudSyncJson = Resolve<SettingsJsonFn>(
                "tiez_core_probe_cloud_sync_json");
            m_cloudSyncStatusJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_cloud_sync_status_json");
            m_startCloudSync = Resolve<CloudSyncLifecycleFn>(
                "tiez_core_start_cloud_sync");
            m_requestCloudSync = Resolve<CloudSyncLifecycleFn>(
                "tiez_core_request_cloud_sync");
            m_stopCloudSync = Resolve<StopCloudSyncFn>(
                "tiez_core_stop_cloud_sync");
            m_relayStatusJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_relay_status_json");
            m_setRelaySharedKeyJson = Resolve<JsonRequestFn>(
                "tiez_core_set_relay_shared_key_json");
            m_generateRelaySharedKeyJson = Resolve<SettingsJsonFn>(
                "tiez_core_generate_relay_shared_key_json");
            m_clearRelaySharedKeyJson = Resolve<SettingsJsonFn>(
                "tiez_core_clear_relay_shared_key_json");
            m_relayHotkeysJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_relay_hotkeys_json");
            m_updateRelayHotkeyJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_update_relay_hotkey_json");
            m_sendRelayClipboardJson = Resolve<SettingsJsonFn>(
                "tiez_core_send_relay_clipboard_json");
            m_fetchRelayClipboardJson = Resolve<SettingsJsonFn>(
                "tiez_core_fetch_relay_clipboard_json");
            m_fileTransferJson = Resolve<SettingsJsonFn>(
                "tiez_core_get_file_transfer_json");
            m_updateFileTransferJson = Resolve<JsonRequestFn>(
                "tiez_core_update_file_transfer_json");
            m_startFileTransfer = Resolve<CloudSyncLifecycleFn>(
                "tiez_core_start_file_transfer");
            m_stopFileTransfer = Resolve<StopCloudSyncFn>(
                "tiez_core_stop_file_transfer");
            m_sendTransferTextJson = Resolve<JsonRequestFn>(
                "tiez_core_send_transfer_text_json");
            m_shareTransferFilesJson = Resolve<JsonRequestFn>(
                "tiez_core_share_transfer_files_json");
            m_setChangedCallback = Resolve<SetChangedCallbackFn>("tiez_core_set_changed_callback");
            m_startCapture = Resolve<StartCaptureFn>("tiez_core_start_capture");
            m_takeLastError = Resolve<TakeLastErrorFn>("tiez_core_take_last_error");
            m_stringFree = Resolve<StringFreeFn>("tiez_core_string_free");

            if (AbiVersion() != TIEZ_CORE_ABI_VERSION)
            {
                throw std::runtime_error(
                    "Unsupported Rust core ABI version: " + std::to_string(AbiVersion()));
            }

            m_handle = m_create();
            if (m_handle == nullptr)
            {
                throw std::runtime_error("Failed to create Rust core: " + TakeLastError());
            }
        }
        catch (...)
        {
            FreeLibrary(m_library);
            m_library = nullptr;
            throw;
        }
    }

    RustCoreBridge::~RustCoreBridge()
    {
        if (m_handle != nullptr && m_setChangedCallback != nullptr)
        {
            m_setChangedCallback(m_handle, nullptr, nullptr);
        }

        if (m_handle != nullptr && m_stopCloudSync != nullptr)
        {
            m_stopCloudSync(m_handle);
        }

        if (m_handle != nullptr && m_stopFileTransfer != nullptr)
        {
            m_stopFileTransfer(m_handle);
        }

        if (m_handle != nullptr && m_destroy != nullptr)
        {
            m_destroy(m_handle);
            m_handle = nullptr;
        }

        if (m_library != nullptr)
        {
            FreeLibrary(m_library);
            m_library = nullptr;
        }
    }

    std::string RustCoreBridge::Snapshot(std::string_view query) const
    {
        std::string queryValue{ query };
        auto* result = m_snapshot(m_handle, queryValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust snapshot failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::Content(std::int64_t entryId) const
    {
        auto* result = m_content(m_handle, entryId);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust content lookup failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::ImageAnalysis(std::int64_t entryId) const
    {
        auto* result = m_imageAnalysis(m_handle, entryId);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust image-analysis lookup failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::AnalyzeImage(std::int64_t entryId, bool force) const
    {
        auto* result = m_analyzeImage(m_handle, entryId, force);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust image analysis failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::PrepareOpenContent(std::int64_t entryId) const
    {
        auto* result = m_prepareOpenContent(m_handle, entryId);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust open-content planning failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::CreateBackup(std::string_view destination) const
    {
        std::string path{ destination };
        auto* result = m_createBackup(m_handle, path.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust backup creation failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::InspectBackup(std::string_view pathValue) const
    {
        std::string path{ pathValue };
        auto* result = m_inspectBackup(m_handle, path.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust backup inspection failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::ScheduleRestore(std::string_view pathValue) const
    {
        std::string path{ pathValue };
        auto* result = m_scheduleRestore(m_handle, path.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust restore scheduling failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::ApplyAction(
        std::int64_t entryId,
        std::string_view action) const
    {
        std::string actionValue{ action };
        auto* result = m_applyActionJson(m_handle, entryId, actionValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust action failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    bool RustCoreBridge::PasteText(std::string_view text) const
    {
        std::string textValue{ text };
        if (!m_pasteText(m_handle, textValue.c_str()))
        {
            throw std::runtime_error("Rust transient text paste failed: " + TakeLastError());
        }
        return true;
    }

    std::string RustCoreBridge::EmojiFavorites() const
    {
        auto* result = m_emojiFavoritesJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust Emoji favorites lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::ImportEmojiFavorite(std::string_view sourcePath) const
    {
        std::string path{ sourcePath };
        auto* result = m_importEmojiFavoriteJson(m_handle, path.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust Emoji favorite import failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::RemoveEmojiFavorite(std::string_view favoritePath) const
    {
        std::string path{ favoritePath };
        auto* result = m_removeEmojiFavoriteJson(m_handle, path.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust Emoji favorite removal failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    bool RustCoreBridge::PasteEmojiFavorite(std::string_view favoritePath) const
    {
        std::string path{ favoritePath };
        if (!m_pasteEmojiFavorite(m_handle, path.c_str()))
        {
            throw std::runtime_error("Rust Emoji favorite paste failed: " + TakeLastError());
        }
        return true;
    }

    std::string RustCoreBridge::TagCatalog() const
    {
        auto* result = m_tagCatalogJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tag catalog lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::TagEntries(std::string_view tag) const
    {
        std::string value{ tag };
        auto* result = m_tagEntriesJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tag entries lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::CreateTag(std::string_view name) const
    {
        std::string value{ name };
        auto* result = m_createTagJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tag creation failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::RenameTag(
        std::string_view oldName,
        std::string_view newName) const
    {
        std::string oldValue{ oldName };
        std::string newValue{ newName };
        auto* result = m_renameTagJson(m_handle, oldValue.c_str(), newValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust global tag rename failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::DeleteTag(std::string_view name) const
    {
        std::string value{ name };
        auto* result = m_deleteTagJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust global tag deletion failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::SetTagColor(
        std::string_view name,
        std::string_view color) const
    {
        std::string nameValue{ name };
        std::string colorValue{ color };
        auto* result = m_setTagColorJson(m_handle, nameValue.c_str(), colorValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tag color update failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::CreateTaggedText(
        std::string_view tag,
        std::string_view content) const
    {
        std::string tagValue{ tag };
        std::string contentValue{ content };
        auto* result = m_createTaggedTextJson(
            m_handle,
            tagValue.c_str(),
            contentValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tagged text creation failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateTags(
        std::int64_t entryId,
        std::string_view tagsJson) const
    {
        std::string tagsValue{ tagsJson };
        auto* result = m_updateTagsJson(m_handle, entryId, tagsValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust tag update failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdatePinnedOrder(std::string_view orderedIdsJson) const
    {
        std::string orderValue{ orderedIdsJson };
        auto* result = m_updatePinnedOrderJson(m_handle, orderValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust pinned reorder failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::Settings() const
    {
        auto* result = m_settingsJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust settings lookup failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateSetting(
        std::string_view key,
        std::string_view value) const
    {
        std::string keyValue{ key };
        std::string settingValue{ value };
        auto* result = m_updateSettingJson(
            m_handle,
            keyValue.c_str(),
            settingValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust setting update failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::AiSettings() const
    {
        auto* result = m_aiSettingsJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust AI settings lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateAiSettings(std::string_view requestJson) const
    {
        std::string request{ requestJson };
        auto* result = m_updateAiSettingsJson(m_handle, request.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust AI settings update failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::ProbeAiProfile(std::string_view profileId) const
    {
        std::string value{ profileId };
        auto* result = m_probeAiProfileJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust AI profile probe failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::RunAiAction(
        std::int64_t entryId,
        std::string_view action) const
    {
        std::string value{ action };
        auto* result = m_runAiActionJson(m_handle, entryId, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust AI action failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::CloudSyncSettings() const
    {
        auto* result = m_cloudSyncSettingsJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust cloud-sync settings lookup failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateCloudSyncSettings(
        std::string_view requestJson) const
    {
        std::string request{ requestJson };
        auto* result = m_updateCloudSyncSettingsJson(m_handle, request.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust cloud-sync settings update failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::ProbeCloudSync() const
    {
        auto* result = m_probeCloudSyncJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust cloud-sync probe failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    std::string RustCoreBridge::CloudSyncStatus() const
    {
        auto* result = m_cloudSyncStatusJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust cloud-sync status failed: " + TakeLastError());
        }

        return ConsumeString(result);
    }

    bool RustCoreBridge::StartCloudSync() const
    {
        if (!m_startCloudSync(m_handle))
        {
            throw std::runtime_error("Rust cloud-sync start failed: " + TakeLastError());
        }
        return true;
    }

    bool RustCoreBridge::RequestCloudSync() const
    {
        if (!m_requestCloudSync(m_handle))
        {
            throw std::runtime_error("Rust cloud-sync request failed: " + TakeLastError());
        }
        return true;
    }

    void RustCoreBridge::StopCloudSync() const noexcept
    {
        if (m_handle != nullptr && m_stopCloudSync != nullptr)
        {
            m_stopCloudSync(m_handle);
        }
    }

    std::string RustCoreBridge::RelayStatus() const
    {
        auto* result = m_relayStatusJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay status failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::SetRelaySharedKey(std::string_view sharedKey) const
    {
        std::string value{ sharedKey };
        auto* result = m_setRelaySharedKeyJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay key update failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::GenerateRelaySharedKey() const
    {
        auto* result = m_generateRelaySharedKeyJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay key generation failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::ClearRelaySharedKey() const
    {
        auto* result = m_clearRelaySharedKeyJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay key clear failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::RelayHotkeys() const
    {
        auto* result = m_relayHotkeysJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay hotkey lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateRelayHotkey(
        std::string_view key,
        std::string_view value) const
    {
        std::string keyValue{ key };
        std::string hotkeyValue{ value };
        auto* result = m_updateRelayHotkeyJson(
            m_handle,
            keyValue.c_str(),
            hotkeyValue.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay hotkey update failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::SendRelayClipboard() const
    {
        auto* result = m_sendRelayClipboardJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay send failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::FetchRelayClipboard() const
    {
        auto* result = m_fetchRelayClipboardJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust relay fetch failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::FileTransfer() const
    {
        auto* result = m_fileTransferJson(m_handle);
        if (result == nullptr)
        {
            throw std::runtime_error("Rust file-transfer lookup failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::UpdateFileTransfer(std::string_view requestJson) const
    {
        std::string request{ requestJson };
        auto* result = m_updateFileTransferJson(m_handle, request.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust file-transfer update failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    bool RustCoreBridge::StartFileTransfer() const
    {
        if (!m_startFileTransfer(m_handle))
        {
            throw std::runtime_error("Rust file-transfer start failed: " + TakeLastError());
        }
        return true;
    }

    void RustCoreBridge::StopFileTransfer() const noexcept
    {
        if (m_handle != nullptr && m_stopFileTransfer != nullptr)
        {
            m_stopFileTransfer(m_handle);
        }
    }

    std::string RustCoreBridge::SendTransferText(std::string_view text) const
    {
        std::string value{ text };
        auto* result = m_sendTransferTextJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust transfer-text send failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    std::string RustCoreBridge::ShareTransferFiles(std::string_view pathsJson) const
    {
        std::string value{ pathsJson };
        auto* result = m_shareTransferFilesJson(m_handle, value.c_str());
        if (result == nullptr)
        {
            throw std::runtime_error("Rust transfer-file share failed: " + TakeLastError());
        }
        return ConsumeString(result);
    }

    void RustCoreBridge::SetChangedCallback(TiezChangedCallback callback, void* userData) const
    {
        m_setChangedCallback(m_handle, callback, userData);
    }

    bool RustCoreBridge::StartCapture() const
    {
        if (!m_startCapture(m_handle))
        {
            throw std::runtime_error("Rust capture start failed: " + TakeLastError());
        }

        return true;
    }

    std::uint32_t RustCoreBridge::AbiVersion() const noexcept
    {
        return m_abiVersion == nullptr ? 0 : m_abiVersion();
    }

    winrt::hstring RustCoreBridge::Utf8ToHstring(std::string_view value)
    {
        if (value.empty())
        {
            return {};
        }

        auto const length = MultiByteToWideChar(
            CP_UTF8,
            MB_ERR_INVALID_CHARS,
            value.data(),
            static_cast<int>(value.size()),
            nullptr,
            0);
        if (length == 0)
        {
            throw std::runtime_error("Rust core returned invalid UTF-8");
        }

        std::wstring output(static_cast<std::size_t>(length), L'\0');
        MultiByteToWideChar(
            CP_UTF8,
            MB_ERR_INVALID_CHARS,
            value.data(),
            static_cast<int>(value.size()),
            output.data(),
            length);
        return winrt::hstring{ output };
    }

    template <typename Function>
    Function RustCoreBridge::Resolve(char const* name) const
    {
        auto* address = GetProcAddress(m_library, name);
        if (address == nullptr)
        {
            throw std::runtime_error(
                std::string("Missing Rust core export ") + name + ": " +
                Win32ErrorMessage(GetLastError()));
        }

        return reinterpret_cast<Function>(address);
    }

    std::string RustCoreBridge::ConsumeString(char* value) const
    {
        if (value == nullptr)
        {
            return {};
        }

        std::string result{ value };
        m_stringFree(value);
        return result;
    }

    std::string RustCoreBridge::TakeLastError() const
    {
        auto const result = ConsumeString(m_takeLastError());
        return result.empty() ? "no additional error was provided" : result;
    }

    std::filesystem::path RustCoreBridge::ResolveDllPath()
    {
        wchar_t configuredPath[32768]{};
        auto const configuredLength = GetEnvironmentVariableW(
            L"TIEZ_WINUI_CORE_DLL",
            configuredPath,
            static_cast<DWORD>(std::size(configuredPath)));
        if (configuredLength > 0 && configuredLength < std::size(configuredPath))
        {
            return configuredPath;
        }

        std::wstring executablePath(32768, L'\0');
        auto const length = GetModuleFileNameW(
            nullptr,
            executablePath.data(),
            static_cast<DWORD>(executablePath.size()));
        if (length == 0 || length == executablePath.size())
        {
            throw std::runtime_error("Unable to resolve the WinUI probe executable path");
        }

        executablePath.resize(length);
        return std::filesystem::path{ executablePath }.parent_path() / L"tiez_winui_core.dll";
    }
}

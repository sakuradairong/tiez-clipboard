#pragma once

#include "tiez_winui_core.h"

namespace tiez::probe
{
    class RustCoreBridge final
    {
    public:
        RustCoreBridge();
        ~RustCoreBridge();

        RustCoreBridge(RustCoreBridge const&) = delete;
        RustCoreBridge& operator=(RustCoreBridge const&) = delete;

        [[nodiscard]] std::string Snapshot(std::string_view query) const;
        [[nodiscard]] std::string Content(std::int64_t entryId) const;
        [[nodiscard]] std::string ImageAnalysis(std::int64_t entryId) const;
        [[nodiscard]] std::string AnalyzeImage(std::int64_t entryId, bool force) const;
        [[nodiscard]] std::string PrepareOpenContent(std::int64_t entryId) const;
        [[nodiscard]] std::string CreateBackup(std::string_view destination) const;
        [[nodiscard]] std::string InspectBackup(std::string_view path) const;
        [[nodiscard]] std::string ScheduleRestore(std::string_view path) const;
        [[nodiscard]] std::string ApplyAction(
            std::int64_t entryId,
            std::string_view action) const;
        bool PasteText(std::string_view text) const;
        [[nodiscard]] std::string EmojiFavorites() const;
        [[nodiscard]] std::string ImportEmojiFavorite(std::string_view sourcePath) const;
        [[nodiscard]] std::string RemoveEmojiFavorite(std::string_view favoritePath) const;
        bool PasteEmojiFavorite(std::string_view favoritePath) const;
        [[nodiscard]] std::string TagCatalog() const;
        [[nodiscard]] std::string TagEntries(std::string_view tag) const;
        [[nodiscard]] std::string CreateTag(std::string_view name) const;
        [[nodiscard]] std::string RenameTag(
            std::string_view oldName,
            std::string_view newName) const;
        [[nodiscard]] std::string DeleteTag(std::string_view name) const;
        [[nodiscard]] std::string SetTagColor(
            std::string_view name,
            std::string_view color) const;
        [[nodiscard]] std::string CreateTaggedText(
            std::string_view tag,
            std::string_view content) const;
        [[nodiscard]] std::string UpdateTags(
            std::int64_t entryId,
            std::string_view tagsJson) const;
        [[nodiscard]] std::string UpdatePinnedOrder(std::string_view orderedIdsJson) const;
        [[nodiscard]] std::string Settings() const;
        [[nodiscard]] std::string UpdateSetting(
            std::string_view key,
            std::string_view value) const;
        [[nodiscard]] std::string AiSettings() const;
        [[nodiscard]] std::string UpdateAiSettings(std::string_view requestJson) const;
        [[nodiscard]] std::string ProbeAiProfile(std::string_view profileId) const;
        [[nodiscard]] std::string RunAiAction(
            std::int64_t entryId,
            std::string_view action) const;
        [[nodiscard]] std::string CloudSyncSettings() const;
        [[nodiscard]] std::string UpdateCloudSyncSettings(
            std::string_view requestJson) const;
        [[nodiscard]] std::string ProbeCloudSync() const;
        [[nodiscard]] std::string CloudSyncStatus() const;
        bool StartCloudSync() const;
        bool RequestCloudSync() const;
        void StopCloudSync() const noexcept;
        [[nodiscard]] std::string RelayStatus() const;
        [[nodiscard]] std::string SetRelaySharedKey(std::string_view sharedKey) const;
        [[nodiscard]] std::string GenerateRelaySharedKey() const;
        [[nodiscard]] std::string ClearRelaySharedKey() const;
        [[nodiscard]] std::string RelayHotkeys() const;
        [[nodiscard]] std::string UpdateRelayHotkey(
            std::string_view key,
            std::string_view value) const;
        [[nodiscard]] std::string SendRelayClipboard() const;
        [[nodiscard]] std::string FetchRelayClipboard() const;
        [[nodiscard]] std::string FileTransfer() const;
        [[nodiscard]] std::string UpdateFileTransfer(std::string_view requestJson) const;
        bool StartFileTransfer() const;
        void StopFileTransfer() const noexcept;
        [[nodiscard]] std::string SendTransferText(std::string_view text) const;
        [[nodiscard]] std::string ShareTransferFiles(std::string_view pathsJson) const;
        void SetChangedCallback(TiezChangedCallback callback, void* userData) const;
        bool StartCapture() const;
        [[nodiscard]] std::uint32_t AbiVersion() const noexcept;

        [[nodiscard]] static winrt::hstring Utf8ToHstring(std::string_view value);

    private:
        using AbiVersionFn = std::uint32_t(__cdecl*)();
        using CreateFn = TiezCoreHandle*(__cdecl*)();
        using DestroyFn = void(__cdecl*)(TiezCoreHandle*);
        using SnapshotFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using ContentFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t);
        using AnalyzeImageFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t, bool);
        using PrepareOpenContentFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t);
        using PathJsonFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using ApplyActionJsonFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t, char const*);
        using TextActionFn = bool(__cdecl*)(TiezCoreHandle*, char const*);
        using UpdateTagsJsonFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t, char const*);
        using UpdatePinnedOrderJsonFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using SettingsJsonFn = char*(__cdecl*)(TiezCoreHandle*);
        using UpdateSettingJsonFn = char*(__cdecl*)(TiezCoreHandle*, char const*, char const*);
        using JsonRequestFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using SetChangedCallbackFn = void(__cdecl*)(TiezCoreHandle*, TiezChangedCallback, void*);
        using StartCaptureFn = bool(__cdecl*)(TiezCoreHandle*);
        using CloudSyncLifecycleFn = bool(__cdecl*)(TiezCoreHandle*);
        using StopCloudSyncFn = void(__cdecl*)(TiezCoreHandle*);
        using TakeLastErrorFn = char*(__cdecl*)();
        using StringFreeFn = void(__cdecl*)(char*);

        template <typename Function>
        [[nodiscard]] Function Resolve(char const* name) const;

        [[nodiscard]] std::string ConsumeString(char* value) const;
        [[nodiscard]] std::string TakeLastError() const;
        [[nodiscard]] static std::filesystem::path ResolveDllPath();

        HMODULE m_library{};
        TiezCoreHandle* m_handle{};
        AbiVersionFn m_abiVersion{};
        CreateFn m_create{};
        DestroyFn m_destroy{};
        SnapshotFn m_snapshot{};
        ContentFn m_content{};
        ContentFn m_imageAnalysis{};
        AnalyzeImageFn m_analyzeImage{};
        PrepareOpenContentFn m_prepareOpenContent{};
        PathJsonFn m_createBackup{};
        PathJsonFn m_inspectBackup{};
        PathJsonFn m_scheduleRestore{};
        ApplyActionJsonFn m_applyActionJson{};
        TextActionFn m_pasteText{};
        SettingsJsonFn m_emojiFavoritesJson{};
        PathJsonFn m_importEmojiFavoriteJson{};
        PathJsonFn m_removeEmojiFavoriteJson{};
        TextActionFn m_pasteEmojiFavorite{};
        SettingsJsonFn m_tagCatalogJson{};
        PathJsonFn m_tagEntriesJson{};
        PathJsonFn m_createTagJson{};
        UpdateSettingJsonFn m_renameTagJson{};
        PathJsonFn m_deleteTagJson{};
        UpdateSettingJsonFn m_setTagColorJson{};
        UpdateSettingJsonFn m_createTaggedTextJson{};
        UpdateTagsJsonFn m_updateTagsJson{};
        UpdatePinnedOrderJsonFn m_updatePinnedOrderJson{};
        SettingsJsonFn m_settingsJson{};
        UpdateSettingJsonFn m_updateSettingJson{};
        SettingsJsonFn m_aiSettingsJson{};
        JsonRequestFn m_updateAiSettingsJson{};
        JsonRequestFn m_probeAiProfileJson{};
        ApplyActionJsonFn m_runAiActionJson{};
        SettingsJsonFn m_cloudSyncSettingsJson{};
        JsonRequestFn m_updateCloudSyncSettingsJson{};
        SettingsJsonFn m_probeCloudSyncJson{};
        SettingsJsonFn m_cloudSyncStatusJson{};
        CloudSyncLifecycleFn m_startCloudSync{};
        CloudSyncLifecycleFn m_requestCloudSync{};
        StopCloudSyncFn m_stopCloudSync{};
        SettingsJsonFn m_relayStatusJson{};
        JsonRequestFn m_setRelaySharedKeyJson{};
        SettingsJsonFn m_generateRelaySharedKeyJson{};
        SettingsJsonFn m_clearRelaySharedKeyJson{};
        SettingsJsonFn m_relayHotkeysJson{};
        UpdateSettingJsonFn m_updateRelayHotkeyJson{};
        SettingsJsonFn m_sendRelayClipboardJson{};
        SettingsJsonFn m_fetchRelayClipboardJson{};
        SettingsJsonFn m_fileTransferJson{};
        JsonRequestFn m_updateFileTransferJson{};
        CloudSyncLifecycleFn m_startFileTransfer{};
        StopCloudSyncFn m_stopFileTransfer{};
        JsonRequestFn m_sendTransferTextJson{};
        JsonRequestFn m_shareTransferFilesJson{};
        SetChangedCallbackFn m_setChangedCallback{};
        StartCaptureFn m_startCapture{};
        TakeLastErrorFn m_takeLastError{};
        StringFreeFn m_stringFree{};
    };
}

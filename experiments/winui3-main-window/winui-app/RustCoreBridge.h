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
        [[nodiscard]] std::string UpdateTags(
            std::int64_t entryId,
            std::string_view tagsJson) const;
        [[nodiscard]] std::string UpdatePinnedOrder(std::string_view orderedIdsJson) const;
        [[nodiscard]] std::string Settings() const;
        [[nodiscard]] std::string UpdateSetting(
            std::string_view key,
            std::string_view value) const;
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
        using UpdateTagsJsonFn = char*(__cdecl*)(TiezCoreHandle*, std::int64_t, char const*);
        using UpdatePinnedOrderJsonFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using SettingsJsonFn = char*(__cdecl*)(TiezCoreHandle*);
        using UpdateSettingJsonFn = char*(__cdecl*)(TiezCoreHandle*, char const*, char const*);
        using SetChangedCallbackFn = void(__cdecl*)(TiezCoreHandle*, TiezChangedCallback, void*);
        using StartCaptureFn = bool(__cdecl*)(TiezCoreHandle*);
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
        UpdateTagsJsonFn m_updateTagsJson{};
        UpdatePinnedOrderJsonFn m_updatePinnedOrderJson{};
        SettingsJsonFn m_settingsJson{};
        UpdateSettingJsonFn m_updateSettingJson{};
        SetChangedCallbackFn m_setChangedCallback{};
        StartCaptureFn m_startCapture{};
        TakeLastErrorFn m_takeLastError{};
        StringFreeFn m_stringFree{};
    };
}

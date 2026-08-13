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
        void ApplyAction(std::int64_t entryId, std::string_view action) const;
        [[nodiscard]] std::uint32_t AbiVersion() const noexcept;

        [[nodiscard]] static winrt::hstring Utf8ToHstring(std::string_view value);

    private:
        using AbiVersionFn = std::uint32_t(__cdecl*)();
        using CreateFn = TiezCoreHandle*(__cdecl*)();
        using DestroyFn = void(__cdecl*)(TiezCoreHandle*);
        using SnapshotFn = char*(__cdecl*)(TiezCoreHandle*, char const*);
        using ApplyActionFn = bool(__cdecl*)(TiezCoreHandle*, std::int64_t, char const*);
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
        ApplyActionFn m_applyAction{};
        TakeLastErrorFn m_takeLastError{};
        StringFreeFn m_stringFree{};
    };
}

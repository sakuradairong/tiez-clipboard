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
            m_prepareOpenContent = Resolve<PrepareOpenContentFn>(
                "tiez_core_prepare_open_content_json");
            m_createBackup = Resolve<PathJsonFn>("tiez_core_create_backup_json");
            m_inspectBackup = Resolve<PathJsonFn>("tiez_core_inspect_backup_json");
            m_scheduleRestore = Resolve<PathJsonFn>("tiez_core_schedule_restore_json");
            m_applyActionJson = Resolve<ApplyActionJsonFn>("tiez_core_apply_action_json");
            m_updateTagsJson = Resolve<UpdateTagsJsonFn>("tiez_core_update_tags_json");
            m_updatePinnedOrderJson = Resolve<UpdatePinnedOrderJsonFn>(
                "tiez_core_update_pinned_order_json");
            m_settingsJson = Resolve<SettingsJsonFn>("tiez_core_get_settings_json");
            m_updateSettingJson = Resolve<UpdateSettingJsonFn>(
                "tiez_core_update_setting_json");
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

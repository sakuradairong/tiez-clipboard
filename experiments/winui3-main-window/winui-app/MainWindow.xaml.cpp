#include "pch.h"
#include "MainWindow.xaml.h"

#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif

#include <microsoft.ui.xaml.window.h>
#include <atomic>
#include <commctrl.h>
#include <shellapi.h>

#pragma comment(lib, "comctl32.lib")
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shell32.lib")

namespace
{
    constexpr UINT kToggleHotkeyId = 1;
    constexpr UINT_PTR kMessageWindowSubclassId = 1;
    constexpr UINT_PTR kMainWindowSubclassId = 2;
    constexpr UINT_PTR kHoverPreviewSubclassId = 3;
    constexpr UINT kTrayCallbackMessage = WM_APP + 7;
    constexpr UINT kMouseMiddleHotkeyMessage = WM_APP + 8;
    constexpr UINT kTrayIconId = 1;
    constexpr UINT kTrayShowCommand = 1001;
    constexpr UINT kTrayExitCommand = 1002;
    constexpr int kAppIconResourceId = 101;
    constexpr wchar_t kAutostartTaskId[] = L"TieZStartup";
    constexpr wchar_t kRunRegistryPath[] =
        L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    std::atomic<HWND> g_mouseMiddleTargetHwnd{};

    struct HotkeySpec
    {
        UINT modifiers{};
        UINT virtualKey{};
        std::wstring display;
    };

    struct EmojiGroup
    {
        std::wstring_view name;
        std::vector<std::wstring_view> values;
    };

    std::vector<EmojiGroup> const& EmojiGroups()
    {
        static std::vector<EmojiGroup> const groups = {
            { L"常用", { L"😀", L"😁", L"😂", L"🤣", L"😊", L"😍", L"😘", L"😎", L"🤔", L"😅", L"😭", L"😡", L"👍", L"👎", L"🙏", L"👏", L"🎉", L"🔥", L"💯", L"✨", L"👌", L"😴", L"🥳", L"🤩", L"😬", L"😇", L"🤝", L"🙌" } },
            { L"表情", { L"🙂", L"🙃", L"😉", L"😌", L"🤗", L"😪", L"😤", L"😱", L"🤯", L"😵", L"🤐", L"🫠", L"🫡", L"🫣", L"😐", L"😑", L"😶", L"🙄", L"😮", L"😲", L"🥺", L"😢", L"😥", L"😓", L"😕", L"🤒", L"🤢", L"🥵", L"🥶", L"🤡" } },
            { L"手势", { L"👌", L"✌️", L"🤞", L"🤟", L"🤘", L"🤙", L"👊", L"✊", L"🤚", L"🖐️", L"✋", L"👋", L"🫶", L"👉", L"👈", L"👇", L"👆", L"🫵", L"🤝", L"🙌", L"🤲", L"🤜", L"🤛", L"🫰", L"🤌", L"✍️", L"👏", L"🤳" } },
            { L"人物", { L"👨‍💻", L"👩‍💻", L"🧑‍💻", L"👨‍🎨", L"👩‍🎨", L"👨‍🚀", L"👩‍🚀", L"👨‍🍳", L"👩‍🍳", L"👨‍⚕️", L"👩‍⚕️", L"👨‍🏫", L"👩‍🏫", L"🧑‍💼", L"🧑‍🔧", L"🧑‍🎧", L"🧑‍🚒", L"👶", L"🧒", L"👦", L"👧", L"🧑", L"👴", L"👵" } },
            { L"动物", { L"🐶", L"🐱", L"🐭", L"🐹", L"🐰", L"🦊", L"🐻", L"🐼", L"🐯", L"🦁", L"🐮", L"🐷", L"🐸", L"🐵", L"🐔", L"🐧", L"🐦", L"🦆", L"🦉", L"🐺", L"🦄", L"🐝", L"🦋", L"🐢", L"🐙", L"🐬", L"🐳", L"🦈" } },
            { L"美食", { L"🍎", L"🍐", L"🍊", L"🍋", L"🍉", L"🍇", L"🍓", L"🍒", L"🍍", L"🥭", L"🍌", L"🥝", L"🍑", L"🍅", L"🥑", L"🥦", L"🥕", L"🌽", L"🍔", L"🍟", L"🍕", L"🌭", L"🍿", L"🍜", L"🍣", L"🍤", L"🍩", L"🍰" } },
            { L"活动", { L"⚽", L"🏀", L"🏈", L"⚾", L"🎾", L"🏐", L"🏓", L"🏸", L"🥊", L"⛳", L"🏹", L"🎯", L"🎮", L"🎲", L"🎹", L"🎸", L"🎤", L"🎧", L"🥁", L"🏆", L"🥇", L"🏃", L"🚴", L"🏋️" } },
            { L"旅行", { L"🚗", L"🚕", L"🚌", L"🚎", L"🏎️", L"🚓", L"🚑", L"🚒", L"🚀", L"✈️", L"🛫", L"🛬", L"🚢", L"⛵", L"🚲", L"🚁", L"🗺️", L"🧭", L"🏝️", L"⛰️", L"🌋", L"🏜️", L"🏕️", L"🏠" } },
            { L"物品", { L"📱", L"💻", L"🖥️", L"⌨️", L"🖱️", L"📷", L"🎥", L"📺", L"🔦", L"💡", L"🔋", L"🔌", L"📦", L"📌", L"✏️", L"📚", L"🧰", L"🧲", L"🧯", L"🧪", L"🔒", L"🔑", L"🎁", L"🛒" } },
            { L"符号", { L"❤️", L"🧡", L"💛", L"💚", L"💙", L"💜", L"🖤", L"🤍", L"🤎", L"💔", L"❗", L"❓", L"✅", L"❌", L"⚠️", L"⭕", L"💯", L"✨", L"⭐", L"🌟", L"➕", L"➖", L"♻️", L"©️" } },
        };
        return groups;
    }

    bool IsHotkeyWhitespace(wchar_t value)
    {
        return value == L' ' || value == L'\t' || value == L'\r' || value == L'\n';
    }

    std::wstring TrimHotkeyText(std::wstring_view value)
    {
        auto first = std::size_t{};
        while (first < value.size() && IsHotkeyWhitespace(value[first]))
        {
            ++first;
        }
        auto last = value.size();
        while (last > first && IsHotkeyWhitespace(value[last - 1]))
        {
            --last;
        }
        return std::wstring{ value.substr(first, last - first) };
    }

    std::wstring UpperAscii(std::wstring value)
    {
        for (auto& character : value)
        {
            if (character >= L'a' && character <= L'z')
            {
                character -= L'a' - L'A';
            }
        }
        return value;
    }

    std::optional<UINT> NamedVirtualKey(std::wstring const& key)
    {
        static constexpr std::pair<std::wstring_view, UINT> mappings[] = {
            { L"SPACE", VK_SPACE },
            { L"SPACEBAR", VK_SPACE },
            { L"TAB", VK_TAB },
            { L"ENTER", VK_RETURN },
            { L"RETURN", VK_RETURN },
            { L"ESC", VK_ESCAPE },
            { L"ESCAPE", VK_ESCAPE },
            { L"BACKSPACE", VK_BACK },
            { L"DELETE", VK_DELETE },
            { L"DEL", VK_DELETE },
            { L"INSERT", VK_INSERT },
            { L"HOME", VK_HOME },
            { L"END", VK_END },
            { L"PAGEUP", VK_PRIOR },
            { L"PAGEDOWN", VK_NEXT },
            { L"UP", VK_UP },
            { L"ARROWUP", VK_UP },
            { L"DOWN", VK_DOWN },
            { L"ARROWDOWN", VK_DOWN },
            { L"LEFT", VK_LEFT },
            { L"ARROWLEFT", VK_LEFT },
            { L"RIGHT", VK_RIGHT },
            { L"ARROWRIGHT", VK_RIGHT },
            { L"PRINTSCREEN", VK_SNAPSHOT },
            { L"PAUSE", VK_PAUSE },
            { L"CAPSLOCK", VK_CAPITAL },
            { L"NUMLOCK", VK_NUMLOCK },
            { L"SCROLLLOCK", VK_SCROLL },
            { L"PLUS", VK_OEM_PLUS },
            { L"COMMA", VK_OEM_COMMA },
            { L"MINUS", VK_OEM_MINUS },
            { L"PERIOD", VK_OEM_PERIOD },
            { L"SLASH", VK_OEM_2 },
            { L"TILDE", VK_OEM_3 },
            { L"GRAVE", VK_OEM_3 },
            { L"SEMICOLON", VK_OEM_1 },
            { L"LBRACKET", VK_OEM_4 },
            { L"BACKSLASH", VK_OEM_5 },
            { L"RBRACKET", VK_OEM_6 },
            { L"QUOTE", VK_OEM_7 },
        };
        for (auto const& [name, virtualKey] : mappings)
        {
            if (key == name)
            {
                return virtualKey;
            }
        }
        if (key.size() >= 2 && key[0] == L'F')
        {
            UINT functionKey{};
            for (std::size_t index = 1; index < key.size(); ++index)
            {
                if (key[index] < L'0' || key[index] > L'9')
                {
                    return std::nullopt;
                }
                functionKey = functionKey * 10 + static_cast<UINT>(key[index] - L'0');
            }
            if (functionKey >= 1 && functionKey <= 24)
            {
                return VK_F1 + functionKey - 1;
            }
        }
        return std::nullopt;
    }

    std::optional<HotkeySpec> ParseHotkey(std::wstring_view configured)
    {
        auto const display = TrimHotkeyText(configured);
        if (display.empty())
        {
            return HotkeySpec{ MOD_NOREPEAT, 0, display };
        }

        UINT modifiers = MOD_NOREPEAT;
        std::wstring keyToken;
        std::size_t start{};
        while (start <= display.size())
        {
            auto const end = display.find(L'+', start);
            auto const length = end == std::wstring::npos ? display.size() - start : end - start;
            auto token = UpperAscii(TrimHotkeyText(
                std::wstring_view{ display }.substr(start, length)));
            if (token.empty())
            {
                return std::nullopt;
            }
            if (token == L"CTRL" || token == L"CONTROL")
            {
                modifiers |= MOD_CONTROL;
            }
            else if (token == L"SHIFT")
            {
                modifiers |= MOD_SHIFT;
            }
            else if (token == L"ALT" || token == L"OPTION" || token == L"MENU")
            {
                modifiers |= MOD_ALT;
            }
            else if (token == L"WIN" || token == L"WINDOWS" || token == L"SUPER"
                || token == L"COMMAND" || token == L"CMD" || token == L"META")
            {
                modifiers |= MOD_WIN;
            }
            else
            {
                if (!keyToken.empty())
                {
                    return std::nullopt;
                }
                keyToken = std::move(token);
            }
            if (end == std::wstring::npos)
            {
                break;
            }
            start = end + 1;
        }
        if (keyToken.empty())
        {
            return std::nullopt;
        }

        UINT virtualKey{};
        if (keyToken.size() == 1
            && ((keyToken[0] >= L'A' && keyToken[0] <= L'Z')
                || (keyToken[0] >= L'0' && keyToken[0] <= L'9')))
        {
            virtualKey = static_cast<UINT>(keyToken[0]);
        }
        else if (auto const named = NamedVirtualKey(keyToken))
        {
            virtualKey = *named;
        }
        else if (keyToken.size() == 1)
        {
            auto const translated = VkKeyScanW(keyToken[0]);
            if (translated == -1)
            {
                return std::nullopt;
            }
            virtualKey = LOBYTE(translated);
            auto const impliedModifiers = HIBYTE(translated);
            if ((impliedModifiers & 1) != 0) modifiers |= MOD_SHIFT;
            if ((impliedModifiers & 2) != 0) modifiers |= MOD_CONTROL;
            if ((impliedModifiers & 4) != 0) modifiers |= MOD_ALT;
        }
        else
        {
            return std::nullopt;
        }
        return HotkeySpec{ modifiers, virtualKey, display };
    }

    std::optional<winrt::hstring> ReadEnvironmentText(wchar_t const* name)
    {
        auto const length = GetEnvironmentVariableW(name, nullptr, 0);
        if (length == 0)
        {
            return std::nullopt;
        }
        std::vector<wchar_t> value(length);
        auto const written = GetEnvironmentVariableW(
            name,
            value.data(),
            static_cast<DWORD>(value.size()));
        if (written == 0 || written >= value.size())
        {
            return std::nullopt;
        }
        return winrt::hstring{ value.data(), written };
    }

    winrt::hstring HotkeyLabel(winrt::hstring const& display)
    {
        std::wstring label{ L"呼出快捷键：" };
        label.append(display.c_str(), display.size());
        return winrt::hstring{ label };
    }

    bool HasPackageIdentity()
    {
        UINT32 length{};
        auto const result = GetCurrentPackageFullName(&length, nullptr);
        if (result == APPMODEL_ERROR_NO_PACKAGE)
        {
            return false;
        }
        if (result != ERROR_INSUFFICIENT_BUFFER)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }
        return true;
    }

    std::filesystem::path CurrentExecutablePath()
    {
        std::vector<wchar_t> buffer(32768);
        auto const length = GetModuleFileNameW(
            nullptr,
            buffer.data(),
            static_cast<DWORD>(buffer.size()));
        if (length == 0 || length >= buffer.size())
        {
            winrt::throw_last_error();
        }
        return std::filesystem::path{ std::wstring_view{ buffer.data(), length } };
    }

    void DeleteRunValue(wchar_t const* name)
    {
        auto const result = RegDeleteKeyValueW(HKEY_CURRENT_USER, kRunRegistryPath, name);
        if (result != ERROR_SUCCESS
            && result != ERROR_FILE_NOT_FOUND
            && result != ERROR_PATH_NOT_FOUND)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }
    }

    void RemoveLegacyAutostartValues()
    {
        DeleteRunValue(L"TieZ");
        DeleteRunValue(L"tie-z");
    }

    std::optional<std::wstring> ReadRunValue(wchar_t const* name)
    {
        DWORD bytes{};
        auto result = RegGetValueW(
            HKEY_CURRENT_USER,
            kRunRegistryPath,
            name,
            RRF_RT_REG_SZ,
            nullptr,
            nullptr,
            &bytes);
        if (result == ERROR_FILE_NOT_FOUND || result == ERROR_PATH_NOT_FOUND)
        {
            return std::nullopt;
        }
        if (result != ERROR_SUCCESS)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }

        std::vector<wchar_t> buffer(bytes / sizeof(wchar_t) + 1);
        result = RegGetValueW(
            HKEY_CURRENT_USER,
            kRunRegistryPath,
            name,
            RRF_RT_REG_SZ,
            nullptr,
            buffer.data(),
            &bytes);
        if (result != ERROR_SUCCESS)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }
        return std::wstring{ buffer.data() };
    }

    bool CommandTargetsCurrentExecutable(std::wstring const& command)
    {
        int argumentCount{};
        auto* arguments = CommandLineToArgvW(command.c_str(), &argumentCount);
        if (arguments == nullptr || argumentCount == 0)
        {
            if (arguments != nullptr)
            {
                LocalFree(arguments);
            }
            return false;
        }

        auto const current = CurrentExecutablePath().wstring();
        auto const matchesExecutable = _wcsicmp(arguments[0], current.c_str()) == 0;
        bool startsHidden{};
        for (int index = 1; index < argumentCount; ++index)
        {
            if (_wcsicmp(arguments[index], L"--autostart") == 0
                || _wcsicmp(arguments[index], L"--minimized") == 0)
            {
                startsHidden = true;
                break;
            }
        }
        LocalFree(arguments);
        return matchesExecutable && startsHidden;
    }

    bool IsNativeRunAutostartEnabled()
    {
        auto const command = ReadRunValue(L"TieZ");
        return command && CommandTargetsCurrentExecutable(*command);
    }

    void SetNativeRunAutostart(bool enabled)
    {
        if (!enabled)
        {
            RemoveLegacyAutostartValues();
            return;
        }

        auto const executable = CurrentExecutablePath().wstring();
        std::wstring command{ L"\"" };
        command.append(executable);
        command.append(L"\" --autostart");

        HKEY key{};
        auto result = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            kRunRegistryPath,
            0,
            nullptr,
            0,
            KEY_SET_VALUE,
            nullptr,
            &key,
            nullptr);
        if (result != ERROR_SUCCESS)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }

        result = RegSetValueExW(
            key,
            L"TieZ",
            0,
            REG_SZ,
            reinterpret_cast<BYTE const*>(command.c_str()),
            static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
        RegCloseKey(key);
        if (result != ERROR_SUCCESS)
        {
            winrt::check_hresult(HRESULT_FROM_WIN32(result));
        }
        DeleteRunValue(L"tie-z");
    }

    bool StartupTaskIsEnabled(winrt::Windows::ApplicationModel::StartupTaskState state)
    {
        return state == winrt::Windows::ApplicationModel::StartupTaskState::Enabled
            || state == winrt::Windows::ApplicationModel::StartupTaskState::EnabledByPolicy;
    }

    bool StartupTaskCanChange(winrt::Windows::ApplicationModel::StartupTaskState state)
    {
        return state != winrt::Windows::ApplicationModel::StartupTaskState::DisabledByUser
            && state != winrt::Windows::ApplicationModel::StartupTaskState::DisabledByPolicy
            && state != winrt::Windows::ApplicationModel::StartupTaskState::EnabledByPolicy;
    }

    winrt::hstring StartupTaskStatus(
        winrt::Windows::ApplicationModel::StartupTaskState state,
        bool reconciled)
    {
        switch (state)
        {
        case winrt::Windows::ApplicationModel::StartupTaskState::DisabledByUser:
            return L"Windows 已在系统设置或任务管理器中禁用 TieZ 自启动；请在那里重新启用。";
        case winrt::Windows::ApplicationModel::StartupTaskState::DisabledByPolicy:
            return L"系统策略禁止 TieZ 自启动，请联系设备管理员。";
        case winrt::Windows::ApplicationModel::StartupTaskState::EnabledByPolicy:
            return L"系统策略已强制启用 TieZ 自启动，应用内无法关闭。";
        case winrt::Windows::ApplicationModel::StartupTaskState::Enabled:
            return reconciled
                ? L"已由 Windows 注册；下次登录后 TieZ 将只在托盘后台启动。"
                : L"Windows 已注册 TieZ 登录启动；启动时不会弹出主窗口。";
        case winrt::Windows::ApplicationModel::StartupTaskState::Disabled:
        default:
            return L"TieZ 不会随 Windows 登录启动。";
        }
    }

    std::optional<std::filesystem::path> SelectBackupPath(HWND owner, bool save)
    {
        winrt::com_ptr<IFileDialog> dialog;
        winrt::check_hresult(CoCreateInstance(
            save ? CLSID_FileSaveDialog : CLSID_FileOpenDialog,
            nullptr,
            CLSCTX_INPROC_SERVER,
            __uuidof(IFileDialog),
            dialog.put_void()));

        COMDLG_FILTERSPEC filters[] = {
            { L"TieZ 备份 (*.tiez-backup)", L"*.tiez-backup" },
            { L"所有文件 (*.*)", L"*.*" },
        };
        winrt::check_hresult(dialog->SetFileTypes(
            static_cast<UINT>(std::size(filters)),
            filters));
        winrt::check_hresult(dialog->SetDefaultExtension(L"tiez-backup"));
        winrt::check_hresult(dialog->SetTitle(save ? L"导出 TieZ 备份" : L"选择 TieZ 备份"));
        if (save)
        {
            winrt::check_hresult(dialog->SetFileName(L"TieZ-backup.tiez-backup"));
        }

        FILEOPENDIALOGOPTIONS options{};
        winrt::check_hresult(dialog->GetOptions(&options));
        options |= FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        options |= save ? FOS_OVERWRITEPROMPT : FOS_FILEMUSTEXIST;
        winrt::check_hresult(dialog->SetOptions(options));

        auto const shown = dialog->Show(owner);
        if (shown == HRESULT_FROM_WIN32(ERROR_CANCELLED))
        {
            return std::nullopt;
        }
        winrt::check_hresult(shown);

        winrt::com_ptr<IShellItem> result;
        winrt::check_hresult(dialog->GetResult(result.put()));
        PWSTR rawPath{};
        winrt::check_hresult(result->GetDisplayName(SIGDN_FILESYSPATH, &rawPath));
        if (rawPath == nullptr)
        {
            throw winrt::hresult_error(E_UNEXPECTED, L"系统文件选择器未返回路径");
        }
        std::filesystem::path selected{ rawPath };
        CoTaskMemFree(rawPath);
        return selected;
    }

    std::vector<std::filesystem::path> SelectEmojiImagePaths(HWND owner)
    {
        winrt::com_ptr<IFileOpenDialog> dialog;
        winrt::check_hresult(CoCreateInstance(
            CLSID_FileOpenDialog,
            nullptr,
            CLSCTX_INPROC_SERVER,
            __uuidof(IFileOpenDialog),
            dialog.put_void()));

        COMDLG_FILTERSPEC filters[] = {
            { L"支持的图片", L"*.png;*.jpg;*.jpeg;*.gif;*.webp" },
            { L"所有文件 (*.*)", L"*.*" },
        };
        winrt::check_hresult(dialog->SetFileTypes(
            static_cast<UINT>(std::size(filters)),
            filters));
        winrt::check_hresult(dialog->SetTitle(L"添加图片表情收藏"));

        FILEOPENDIALOGOPTIONS options{};
        winrt::check_hresult(dialog->GetOptions(&options));
        options |= FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;
        options |= FOS_ALLOWMULTISELECT;
        winrt::check_hresult(dialog->SetOptions(options));

        auto const shown = dialog->Show(owner);
        if (shown == HRESULT_FROM_WIN32(ERROR_CANCELLED))
        {
            return {};
        }
        winrt::check_hresult(shown);

        winrt::com_ptr<IShellItemArray> results;
        winrt::check_hresult(dialog->GetResults(results.put()));
        DWORD count{};
        winrt::check_hresult(results->GetCount(&count));
        std::vector<std::filesystem::path> selected;
        selected.reserve(count);
        for (DWORD index = 0; index < count; ++index)
        {
            winrt::com_ptr<IShellItem> item;
            winrt::check_hresult(results->GetItemAt(index, item.put()));
            PWSTR rawPath{};
            winrt::check_hresult(item->GetDisplayName(SIGDN_FILESYSPATH, &rawPath));
            if (rawPath != nullptr)
            {
                selected.emplace_back(rawPath);
                CoTaskMemFree(rawPath);
            }
        }
        return selected;
    }

    winrt::hstring BackupSummary(
        winrt::Windows::Data::Json::JsonObject const& information,
        std::wstring_view prefix)
    {
        auto const entryCount = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"entryCount")));
        auto const fileCount = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"fileCount")));
        auto const totalBytes = static_cast<std::uint64_t>(std::llround(
            information.GetNamedNumber(L"totalBytes")));
        auto const appVersion = information.GetNamedString(L"appVersion");
        std::wstringstream message;
        message << prefix
            << L"版本 " << appVersion.c_str()
            << L"，" << entryCount << L" 条记录，"
            << fileCount << L" 个文件，共 " << totalBytes << L" 字节。";
        return winrt::hstring{ message.str() };
    }

    winrt::hstring StatusMessage(
        std::wstring_view prefix,
        winrt::hstring const& detail)
    {
        std::wstring message{ prefix };
        message.append(detail.c_str(), detail.size());
        return winrt::hstring{ message };
    }

    winrt::hstring AdapterLabel(winrt::hstring const& adapter)
    {
        if (adapter == L"memory")
        {
            return L"内存";
        }
        if (adapter == L"sqlite-read-only")
        {
            return L"SQLite 只读";
        }
        if (adapter == L"sqlite")
        {
            return L"SQLite";
        }
        return adapter;
    }

    winrt::hstring ContentTypeLabel(winrt::hstring const& contentType)
    {
        if (contentType == L"text") return L"文本";
        if (contentType == L"image") return L"图片";
        if (contentType == L"url") return L"链接";
        if (contentType == L"code") return L"代码";
        if (contentType == L"file" || contentType == L"files") return L"文件";
        if (contentType == L"html" || contentType == L"rich_text") return L"富文本";
        return contentType;
    }

    winrt::hstring CapturedAtLabel(winrt::hstring const& capturedAt)
    {
        if (capturedAt == L"Just now")
        {
            return L"刚刚";
        }

        std::wistringstream stream{ std::wstring{ capturedAt.c_str() } };
        std::uint64_t amount{};
        std::wstring unit;
        std::wstring ago;
        if (stream >> amount >> unit >> ago && ago == L"ago")
        {
            std::wstringstream label;
            label << amount;
            if (unit == L"minute" || unit == L"minutes") label << L" 分钟前";
            else if (unit == L"hour" || unit == L"hours") label << L" 小时前";
            else if (unit == L"day" || unit == L"days") label << L" 天前";
            else return capturedAt;
            return winrt::hstring{ label.str() };
        }
        return capturedAt;
    }

    std::wstring LowerText(winrt::hstring const& value)
    {
        std::wstring lowered{ value.c_str(), value.size() };
        std::transform(lowered.begin(), lowered.end(), lowered.begin(), [](wchar_t character)
        {
            return static_cast<wchar_t>(std::towlower(character));
        });
        return lowered;
    }

    bool IsProtectedTagName(winrt::hstring const& value)
    {
        auto const lowered = LowerText(value);
        return lowered == L"sensitive" || lowered == L"password" || lowered == L"密码";
    }

    int HexDigit(wchar_t value)
    {
        if (value >= L'0' && value <= L'9') return value - L'0';
        if (value >= L'a' && value <= L'f') return value - L'a' + 10;
        if (value >= L'A' && value <= L'F') return value - L'A' + 10;
        return -1;
    }

    winrt::Windows::UI::Color TagColor(winrt::hstring const& value)
    {
        winrt::Windows::UI::Color color{};
        color.A = 255;
        color.R = 0;
        color.G = 120;
        color.B = 212;
        if (value.size() != 7 || value[0] != L'#')
        {
            return color;
        }
        auto const redHigh = HexDigit(value[1]);
        auto const redLow = HexDigit(value[2]);
        auto const greenHigh = HexDigit(value[3]);
        auto const greenLow = HexDigit(value[4]);
        auto const blueHigh = HexDigit(value[5]);
        auto const blueLow = HexDigit(value[6]);
        if (redHigh < 0 || redLow < 0 || greenHigh < 0 || greenLow < 0
            || blueHigh < 0 || blueLow < 0)
        {
            return color;
        }
        color.R = static_cast<std::uint8_t>((redHigh << 4) | redLow);
        color.G = static_cast<std::uint8_t>((greenHigh << 4) | greenLow);
        color.B = static_cast<std::uint8_t>((blueHigh << 4) | blueLow);
        return color;
    }

    winrt::hstring TagColorHex(winrt::Windows::UI::Color const& color)
    {
        wchar_t value[8]{};
        swprintf_s(value, L"#%02X%02X%02X", color.R, color.G, color.B);
        return winrt::hstring{ value };
    }

    winrt::hstring ActionStatus(std::string_view action)
    {
        if (action == "pin") return L"置顶状态已更新";
        if (action == "delete") return L"记录已删除";
        if (action == "clear") return L"未保护的历史记录已清空";
        if (action == "paste-plain") return L"已执行纯文本粘贴";
        if (action == "paste-rich") return L"已执行富文本粘贴";
        if (action == "copy-plain") return L"已复制到剪贴板";
        return L"操作已完成";
    }

    std::vector<winrt::hstring> ItemTags(
        winrt::Windows::Data::Json::JsonObject const& item)
    {
        std::vector<winrt::hstring> tags;
        if (!item.HasKey(L"tags"))
        {
            return tags;
        }
        auto const values = item.GetNamedArray(L"tags");
        tags.reserve(values.Size());
        for (std::uint32_t index = 0; index < values.Size(); ++index)
        {
            auto const tag = values.GetStringAt(index);
            if (!tag.empty())
            {
                tags.push_back(tag);
            }
        }
        return tags;
    }

    winrt::hstring JoinTags(std::vector<winrt::hstring> const& tags)
    {
        std::wstring joined;
        for (auto const& tag : tags)
        {
            if (!joined.empty())
            {
                joined.append(L"，");
            }
            joined.append(tag.c_str(), tag.size());
        }
        return winrt::hstring{ joined };
    }

    std::vector<winrt::hstring> SplitTags(winrt::hstring const& value)
    {
        std::wstring normalized{ value.c_str(), value.size() };
        std::replace(normalized.begin(), normalized.end(), L'，', L',');
        std::replace(normalized.begin(), normalized.end(), L'；', L',');
        std::replace(normalized.begin(), normalized.end(), L';', L',');

        constexpr std::wstring_view whitespace{ L" \t\r\n" };
        std::vector<winrt::hstring> tags;
        std::size_t start{};
        while (start <= normalized.size())
        {
            auto const end = normalized.find(L',', start);
            auto const token = normalized.substr(
                start,
                end == std::wstring::npos ? std::wstring::npos : end - start);
            auto const first = token.find_first_not_of(whitespace);
            if (first != std::wstring::npos)
            {
                auto const last = token.find_last_not_of(whitespace);
                tags.emplace_back(token.substr(first, last - first + 1));
            }
            if (end == std::wstring::npos)
            {
                break;
            }
            start = end + 1;
        }
        return tags;
    }

    winrt::Microsoft::UI::Xaml::Controls::Button ActionButton(
        winrt::hstring const& label,
        std::function<void()> action)
    {
        winrt::Microsoft::UI::Xaml::Controls::Button button;
        button.Content(winrt::box_value(label));
        button.Padding(winrt::Microsoft::UI::Xaml::ThicknessHelper::FromLengths(6, 5, 6, 5));
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(button, label);
        button.Click([action = std::move(action)](auto const&, auto const&)
        {
            action();
        });
        return button;
    }

    winrt::Microsoft::UI::Xaml::Controls::MenuFlyoutItem CommandItem(
        winrt::hstring const& label,
        bool enabled,
        std::function<void()> action)
    {
        winrt::Microsoft::UI::Xaml::Controls::MenuFlyoutItem item;
        item.Text(label);
        item.IsEnabled(enabled);
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(item, label);
        item.Click([action = std::move(action)](auto const&, auto const&)
        {
            action();
        });
        return item;
    }

    winrt::Microsoft::UI::Xaml::Controls::ToggleSwitch SettingToggle(
        winrt::hstring const& label,
        winrt::hstring const& description)
    {
        winrt::Microsoft::UI::Xaml::Controls::ToggleSwitch toggle;
        toggle.Header(winrt::box_value(label));
        toggle.OnContent(winrt::box_value(L"已开启"));
        toggle.OffContent(winrt::box_value(L"已关闭"));
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(toggle, label);
        winrt::Microsoft::UI::Xaml::Automation::AutomationProperties::SetHelpText(
            toggle,
            description);
        return toggle;
    }

    LRESULT CALLBACK HotkeySubclassProc(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR,
        DWORD_PTR refData)
    {
        auto* window = reinterpret_cast<winrt::Tiez::WinUIProbe::implementation::MainWindow*>(refData);
        if (window != nullptr && window->OnNativeMessage(hwnd, message, wParam, lParam))
        {
            return 0;
        }

        return DefSubclassProc(hwnd, message, wParam, lParam);
    }

    LRESULT CALLBACK MouseMiddleHookProc(int code, WPARAM wParam, LPARAM lParam)
    {
        if (code >= 0 && wParam == WM_MBUTTONDOWN)
        {
            auto const target = g_mouseMiddleTargetHwnd.load(std::memory_order_acquire);
            if (target != nullptr && PostMessageW(target, kMouseMiddleHotkeyMessage, 0, 0))
            {
                return 1;
            }
        }
        return CallNextHookEx(nullptr, code, wParam, lParam);
    }

    LRESULT CALLBACK HoverPreviewSubclassProc(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR,
        DWORD_PTR)
    {
        if (message == WM_NCHITTEST)
        {
            return HTTRANSPARENT;
        }
        if (message == WM_MOUSEACTIVATE)
        {
            return MA_NOACTIVATE;
        }
        return DefSubclassProc(hwnd, message, wParam, lParam);
    }
}

namespace winrt::Tiez::WinUIProbe::implementation
{
    using namespace Microsoft::UI::Xaml;
    using namespace Microsoft::UI::Xaml::Automation;
    using namespace Microsoft::UI::Xaml::Controls;
    using namespace Microsoft::UI::Xaml::Controls::Primitives;
    using namespace Microsoft::UI::Xaml::Input;
    using namespace Microsoft::UI::Xaml::Media;
    using namespace Windows::ApplicationModel;
    using namespace Windows::ApplicationModel::DataTransfer;
    using namespace Windows::Data::Json;
    using namespace Windows::Management::Deployment;
    using winrt::Windows::Foundation::IInspectable;
    using Windows::System::VirtualKey;

    MainWindow::MainWindow() : MainWindow(false)
    {
    }

    MainWindow::MainWindow(bool startHidden) : m_startHidden(startHidden)
    {
        InitializeComponent();
        Title(L"TieZ · 原生剪贴板");
        SetInitialWindowSize();
        SetupLifecycle();
        SetupImeGuards();
        if (!m_startHidden)
        {
            SearchBox().Focus(FocusState::Programmatic);
        }

        try
        {
            m_refreshSink = std::make_shared<HistoryRefreshSink>();
            m_refreshSink->window = this;
            m_refreshSink->dispatcher = DispatcherQueue();
            m_core = std::make_unique<tiez::probe::RustCoreBridge>();
            m_core->SetChangedCallback(&MainWindow::OnHistoryChanged, m_refreshSink.get());
            LoadSettings();
            RefreshAutostartStateAsync(true);
            if (m_productionData && !m_settingsReadOnly)
            {
                m_core->StartCloudSync();
                m_cloudSyncStatusTimer = DispatcherQueue().CreateTimer();
                m_cloudSyncStatusTimer.Interval(std::chrono::seconds(1));
                m_cloudSyncStatusTimer.IsRepeating(true);
                m_cloudSyncStatusTimer.Tick([this](auto const&, auto const&)
                {
                    UpdateCloudSyncStatus();
                });
                m_cloudSyncStatusTimer.Start();
                UpdateCloudSyncStatus();
            }
            m_core->StartCapture();
            RefreshItems();
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"启动失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    MainWindow::~MainWindow()
    {
        if (m_cloudSyncStatusTimer)
        {
            m_cloudSyncStatusTimer.Stop();
            m_cloudSyncStatusTimer = nullptr;
        }
        HideHoverPreview();
        if (m_hoverPreviewWindow)
        {
            RemoveWindowSubclass(
                m_hoverPreviewHwnd,
                HoverPreviewSubclassProc,
                kHoverPreviewSubclassId);
            m_hoverPreviewWindow.Close();
            m_hoverPreviewWindow = nullptr;
            m_hoverPreviewHwnd = nullptr;
        }
        if (m_refreshSink)
        {
            std::lock_guard<std::mutex> guard(m_refreshSink->mutex);
            m_refreshSink->window = nullptr;
        }
        if (m_core)
        {
            m_core->SetChangedCallback(nullptr, nullptr);
            m_core.reset();
        }
        TeardownLifecycle();
    }

    void MainWindow::OnHistoryChanged(void* userData, std::uint64_t)
    {
        auto* raw = static_cast<HistoryRefreshSink*>(userData);
        if (raw == nullptr)
        {
            return;
        }

        std::shared_ptr<HistoryRefreshSink> sink;
        try
        {
            sink = raw->shared_from_this();
        }
        catch (std::bad_weak_ptr const&)
        {
            return;
        }

        Microsoft::UI::Dispatching::DispatcherQueue dispatcher{ nullptr };
        {
            std::lock_guard<std::mutex> guard(sink->mutex);
            if (sink->window == nullptr)
            {
                return;
            }
            dispatcher = sink->dispatcher;
        }
        if (!dispatcher)
        {
            return;
        }

        dispatcher.TryEnqueue([sink]()
        {
            std::lock_guard<std::mutex> guard(sink->mutex);
            if (sink->window != nullptr)
            {
                sink->window->RefreshItems();
            }
        });
    }

    void MainWindow::SearchBox_TextChanged(IInspectable const&, TextChangedEventArgs const&)
    {
        if (m_core)
        {
            RefreshItems();
        }
    }

    void MainWindow::SearchBox_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (HandleNavigationKey(args.Key()))
        {
            args.Handled(true);
        }
    }

    void MainWindow::RootGrid_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (HandleNavigationKey(args.Key()))
        {
            args.Handled(true);
        }
    }

    void MainWindow::RefreshButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        RefreshItems();
    }

    void MainWindow::ClearHistoryButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ConfirmClearHistoryAsync();
    }

    void MainWindow::EmojiButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ShowEmojiPickerAsync();
    }

    void MainWindow::TagButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ShowTagManagerAsync();
    }

    winrt::fire_and_forget MainWindow::ShowEmojiPickerAsync()
    {
        auto lifetime = get_strong();
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法粘贴表情。");
            co_return;
        }

        try
        {
            m_suspendLifecycle = true;
            for (;;)
            {
                auto const favoriteResponse = JsonObject::Parse(
                    tiez::probe::RustCoreBridge::Utf8ToHstring(m_core->EmojiFavorites()));
                auto const readOnly = favoriteResponse.GetNamedBoolean(L"read_only", true);
                auto const favorites = favoriteResponse.GetNamedArray(L"items");
                winrt::hstring selectedEmoji;
                winrt::hstring selectedFavoritePath;
                winrt::hstring selectedFavoriteName;
                winrt::hstring favoriteToRemove;

                ContentDialog dialog;
                dialog.XamlRoot(RootGrid().XamlRoot());
                dialog.Title(winrt::box_value(L"Emoji 与图片表情"));
                dialog.PrimaryButtonText(L"添加图片");
                dialog.IsPrimaryButtonEnabled(!readOnly);
                dialog.CloseButtonText(L"关闭");
                dialog.DefaultButton(ContentDialogButton::Close);
                dialog.MinWidth(700);

                StackPanel content;
                content.Spacing(12);

                TextBlock hint;
                hint.Text(L"选择后会立即粘贴到呼出 TieZ 前使用的窗口；按 Tab 可浏览全部按钮。");
                hint.TextWrapping(TextWrapping::Wrap);
                hint.Foreground(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                content.Children().Append(hint);

                StackPanel groups;
                groups.Spacing(16);

                TextBlock favoritesTitle;
                std::wstringstream favoritesLabel;
                favoritesLabel << L"图片收藏（" << favorites.Size() << L"）";
                if (readOnly)
                {
                    favoritesLabel << L" · 只读";
                }
                favoritesTitle.Text(winrt::hstring{ favoritesLabel.str() });
                favoritesTitle.Style(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
                groups.Children().Append(favoritesTitle);

                if (favorites.Size() == 0)
                {
                    TextBlock empty;
                    empty.Text(readOnly
                        ? L"当前只读数据副本没有图片表情收藏。"
                        : L"暂无图片收藏。选择“添加图片”可一次导入多张图片。");
                    empty.TextWrapping(TextWrapping::Wrap);
                    empty.Foreground(Application::Current().Resources()
                        .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                    groups.Children().Append(empty);
                }
                else
                {
                    Grid favoriteGrid;
                    favoriteGrid.ColumnSpacing(8);
                    favoriteGrid.RowSpacing(8);
                    for (int column = 0; column < 4; ++column)
                    {
                        ColumnDefinition definition;
                        definition.Width(GridLength{ 1, GridUnitType::Star });
                        favoriteGrid.ColumnDefinitions().Append(definition);
                    }
                    auto const rowCount = (favorites.Size() + 3) / 4;
                    for (std::uint32_t row = 0; row < rowCount; ++row)
                    {
                        RowDefinition definition;
                        definition.Height(GridLengthHelper::Auto());
                        favoriteGrid.RowDefinitions().Append(definition);
                    }

                    for (std::uint32_t index = 0; index < favorites.Size(); ++index)
                    {
                        auto const favorite = favorites.GetObjectAt(index);
                        auto const path = favorite.GetNamedString(L"path");
                        auto const fileName = favorite.GetNamedString(L"file_name", L"图片表情");

                        StackPanel card;
                        card.Spacing(4);

                        Button preview;
                        preview.MinHeight(96);
                        preview.HorizontalAlignment(HorizontalAlignment::Stretch);
                        preview.HorizontalContentAlignment(HorizontalAlignment::Center);
                        preview.VerticalContentAlignment(VerticalAlignment::Center);
                        try
                        {
                            std::wstring displayPath{ path.c_str(), path.size() };
                            if (displayPath.rfind(L"\\\\?\\", 0) == 0)
                            {
                                displayPath.erase(0, 4);
                            }
                            std::replace(displayPath.begin(), displayPath.end(), L'\\', L'/');
                            Microsoft::UI::Xaml::Controls::Image image;
                            image.Width(112);
                            image.Height(82);
                            image.Stretch(Stretch::Uniform);
                            Microsoft::UI::Xaml::Media::Imaging::BitmapImage bitmap;
                            bitmap.UriSource(Windows::Foundation::Uri{ L"file:///" + displayPath });
                            image.Source(bitmap);
                            preview.Content(image);
                        }
                        catch (winrt::hresult_error const&)
                        {
                            preview.Content(winrt::box_value(L"无法预览"));
                        }
                        std::wstring previewName{ L"粘贴收藏图片 " };
                        previewName.append(fileName.c_str(), fileName.size());
                        AutomationProperties::SetName(preview, winrt::hstring{ previewName });
                        AutomationProperties::SetHelpText(preview, L"选择后粘贴到上一个窗口");
                        ToolTipService::SetToolTip(preview, winrt::box_value(fileName));
                        preview.Click([
                            dialog,
                            &selectedFavoritePath,
                            &selectedFavoriteName,
                            path,
                            fileName](auto const&, auto const&)
                        {
                            selectedFavoritePath = path;
                            selectedFavoriteName = fileName;
                            dialog.Hide();
                        });
                        card.Children().Append(preview);

                        Button remove;
                        remove.Content(winrt::box_value(L"删除"));
                        remove.HorizontalAlignment(HorizontalAlignment::Stretch);
                        remove.Visibility(readOnly ? Visibility::Collapsed : Visibility::Visible);
                        std::wstring removeName{ L"删除收藏图片 " };
                        removeName.append(fileName.c_str(), fileName.size());
                        AutomationProperties::SetName(remove, winrt::hstring{ removeName });
                        AutomationProperties::SetHelpText(remove, L"从图片表情收藏中移除");
                        remove.Click([dialog, &favoriteToRemove, path](auto const&, auto const&)
                        {
                            favoriteToRemove = path;
                            dialog.Hide();
                        });
                        card.Children().Append(remove);

                        Grid::SetRow(card, static_cast<int>(index / 4));
                        Grid::SetColumn(card, static_cast<int>(index % 4));
                        favoriteGrid.Children().Append(card);
                    }
                    groups.Children().Append(favoriteGrid);
                }

                Border separator;
                separator.Height(1);
                separator.Background(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"DividerStrokeColorDefaultBrush")).as<Brush>());
                groups.Children().Append(separator);

                TextBlock unicodeTitle;
                unicodeTitle.Text(L"Unicode Emoji");
                unicodeTitle.Style(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
                groups.Children().Append(unicodeTitle);

                for (auto const& group : EmojiGroups())
                {
                    TextBlock title;
                    title.Text(winrt::hstring{ group.name });
                    title.Style(Application::Current().Resources()
                        .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
                    groups.Children().Append(title);

                    Grid grid;
                    grid.ColumnSpacing(4);
                    grid.RowSpacing(4);
                    for (int column = 0; column < 8; ++column)
                    {
                        ColumnDefinition definition;
                        definition.Width(GridLength{ 1, GridUnitType::Star });
                        grid.ColumnDefinitions().Append(definition);
                    }
                    auto const emojiRowCount = (group.values.size() + 7) / 8;
                    for (std::size_t row = 0; row < emojiRowCount; ++row)
                    {
                        RowDefinition definition;
                        definition.Height(GridLengthHelper::Auto());
                        grid.RowDefinitions().Append(definition);
                    }

                    for (std::size_t index = 0; index < group.values.size(); ++index)
                    {
                        auto const emoji = winrt::hstring{ group.values[index] };
                        Button button;
                        button.Content(winrt::box_value(emoji));
                        button.FontSize(24);
                        button.MinWidth(54);
                        button.Height(46);
                        button.HorizontalAlignment(HorizontalAlignment::Stretch);
                        std::wstring accessibleName{ L"表情 " };
                        accessibleName.append(emoji.c_str(), emoji.size());
                        AutomationProperties::SetName(button, winrt::hstring{ accessibleName });
                        AutomationProperties::SetHelpText(button, L"选择后粘贴到上一个窗口");
                        ToolTipService::SetToolTip(button, winrt::box_value(emoji));
                        Grid::SetRow(button, static_cast<int>(index / 8));
                        Grid::SetColumn(button, static_cast<int>(index % 8));
                        button.Click([dialog, &selectedEmoji, emoji](auto const&, auto const&)
                        {
                            selectedEmoji = emoji;
                            dialog.Hide();
                        });
                        grid.Children().Append(button);
                    }
                    groups.Children().Append(grid);
                }

                ScrollViewer scroller;
                scroller.MaxHeight(560);
                scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
                scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
                scroller.Content(groups);
                content.Children().Append(scroller);
                dialog.Content(content);

                auto const result = co_await dialog.ShowAsync();
                if (!favoriteToRemove.empty())
                {
                    auto const uiThread = winrt::apartment_context{};
                    auto const path = winrt::to_string(favoriteToRemove);
                    std::string response;
                    std::string failure;
                    co_await winrt::resume_background();
                    try
                    {
                        response = m_core->RemoveEmojiFavorite(path);
                    }
                    catch (std::exception const& error)
                    {
                        failure = error.what();
                    }
                    try
                    {
                        co_await uiThread;
                    }
                    catch (...)
                    {
                        co_return;
                    }
                    if (!failure.empty())
                    {
                        SetStatus(StatusMessage(
                            L"删除图片表情失败：",
                            tiez::probe::RustCoreBridge::Utf8ToHstring(failure)));
                    }
                    else
                    {
                        auto const mutation = JsonObject::Parse(
                            tiez::probe::RustCoreBridge::Utf8ToHstring(response));
                        SetStatus(mutation.GetNamedString(L"message", L"已删除图片表情。"));
                    }
                    continue;
                }

                if (result == ContentDialogResult::Primary)
                {
                    auto const selected = SelectEmojiImagePaths(GetWindowHandle());
                    if (selected.empty())
                    {
                        SetStatus(L"已取消添加图片表情。");
                        continue;
                    }

                    std::vector<std::string> paths;
                    paths.reserve(selected.size());
                    for (auto const& path : selected)
                    {
                        paths.push_back(winrt::to_string(winrt::hstring{ path.wstring() }));
                    }
                    auto const uiThread = winrt::apartment_context{};
                    std::vector<std::string> responses;
                    std::vector<std::string> failures;
                    co_await winrt::resume_background();
                    for (auto const& path : paths)
                    {
                        try
                        {
                            responses.push_back(m_core->ImportEmojiFavorite(path));
                        }
                        catch (std::exception const& error)
                        {
                            failures.push_back(error.what());
                        }
                    }
                    try
                    {
                        co_await uiThread;
                    }
                    catch (...)
                    {
                        co_return;
                    }

                    std::size_t imported{};
                    std::size_t existing{};
                    for (auto const& response : responses)
                    {
                        auto const mutation = JsonObject::Parse(
                            tiez::probe::RustCoreBridge::Utf8ToHstring(response));
                        if (mutation.GetNamedBoolean(L"changed", false))
                        {
                            ++imported;
                        }
                        else
                        {
                            ++existing;
                        }
                    }
                    std::wstringstream status;
                    status << L"图片表情导入完成：新增 " << imported << L" 个";
                    if (existing > 0)
                    {
                        status << L"，已存在 " << existing << L" 个";
                    }
                    if (!failures.empty())
                    {
                        status << L"，失败 " << failures.size() << L" 个。首个错误："
                            << tiez::probe::RustCoreBridge::Utf8ToHstring(failures.front()).c_str();
                    }
                    SetStatus(winrt::hstring{ status.str() });
                    continue;
                }

                if (!selectedFavoritePath.empty())
                {
                    PasteFavoriteImage(selectedFavoritePath, selectedFavoriteName);
                    co_return;
                }
                if (!selectedEmoji.empty())
                {
                    PasteTransientText(selectedEmoji);
                    co_return;
                }

                m_suspendLifecycle = false;
                SetStatus(L"已关闭表情选择器。");
                SearchBox().Focus(FocusState::Programmatic);
                co_return;
            }
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"无法打开表情选择器：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"无法打开表情选择器：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    winrt::Windows::Foundation::IAsyncOperation<winrt::hstring>
        MainWindow::RunRustOperationAsync(std::function<std::string()> operation)
    {
        co_await winrt::resume_background();
        co_return tiez::probe::RustCoreBridge::Utf8ToHstring(operation());
    }

    winrt::fire_and_forget MainWindow::ShowTagManagerAsync()
    {
        auto lifetime = get_strong();
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法管理标签。");
            co_return;
        }

        try
        {
            m_suspendLifecycle = true;
            winrt::hstring activeTag;
            for (;;)
            {
                if (activeTag.empty())
                {
                    auto const response = JsonObject::Parse(
                        tiez::probe::RustCoreBridge::Utf8ToHstring(m_core->TagCatalog()));
                    auto const readOnly = response.GetNamedBoolean(L"read_only", true);
                    auto const tags = response.GetNamedArray(L"tags");
                    winrt::hstring actionTag;
                    winrt::hstring actionColor;
                    std::wstring catalogAction;
                    std::uint64_t actionCount{};

                    ContentDialog dialog;
                    dialog.XamlRoot(RootGrid().XamlRoot());
                    dialog.Title(winrt::box_value(L"标签管理"));
                    dialog.PrimaryButtonText(L"新建标签");
                    dialog.IsPrimaryButtonEnabled(false);
                    dialog.CloseButtonText(L"关闭");
                    dialog.DefaultButton(ContentDialogButton::Close);
                    dialog.MinWidth(760);

                    StackPanel content;
                    content.Spacing(12);

                    TextBlock hint;
                    hint.Text(readOnly
                        ? L"当前是只读数据副本，可查看标签和记录，但不能修改。"
                        : L"搜索或输入新标签；删除标签会永久删除使用该标签的全部记录。内置敏感标签受保护。");
                    hint.TextWrapping(TextWrapping::Wrap);
                    hint.Foreground(Application::Current().Resources()
                        .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                    content.Children().Append(hint);

                    TextBox search;
                    search.PlaceholderText(L"搜索标签或输入新标签名称");
                    AutomationProperties::SetName(search, L"搜索或新建标签");
                    content.Children().Append(search);

                    StackPanel tagList;
                    tagList.Spacing(6);
                    auto tagRows = std::make_shared<
                        std::vector<std::pair<std::wstring, FrameworkElement>>>();

                    for (std::uint32_t index = 0; index < tags.Size(); ++index)
                    {
                        auto const tag = tags.GetObjectAt(index);
                        auto const name = tag.GetNamedString(L"name");
                        auto const count = static_cast<std::uint64_t>(
                            tag.GetNamedNumber(L"count", 0));
                        auto const protectedTag = tag.GetNamedBoolean(L"protected", false);
                        winrt::hstring color;
                        auto const colorValue = tag.GetNamedValue(L"color");
                        if (colorValue.ValueType() == JsonValueType::String)
                        {
                            color = colorValue.GetString();
                        }

                        Grid row;
                        row.ColumnSpacing(8);
                        ColumnDefinition dotColumn;
                        dotColumn.Width(GridLengthHelper::Auto());
                        ColumnDefinition nameColumn;
                        nameColumn.Width(GridLength{ 1, GridUnitType::Star });
                        ColumnDefinition countColumn;
                        countColumn.Width(GridLengthHelper::Auto());
                        ColumnDefinition renameColumn;
                        renameColumn.Width(GridLengthHelper::Auto());
                        ColumnDefinition colorColumn;
                        colorColumn.Width(GridLengthHelper::Auto());
                        ColumnDefinition deleteColumn;
                        deleteColumn.Width(GridLengthHelper::Auto());
                        row.ColumnDefinitions().Append(dotColumn);
                        row.ColumnDefinitions().Append(nameColumn);
                        row.ColumnDefinitions().Append(countColumn);
                        row.ColumnDefinitions().Append(renameColumn);
                        row.ColumnDefinitions().Append(colorColumn);
                        row.ColumnDefinitions().Append(deleteColumn);

                        Border dot;
                        dot.Width(12);
                        dot.Height(12);
                        dot.CornerRadius(CornerRadiusHelper::FromUniformRadius(6));
                        dot.VerticalAlignment(VerticalAlignment::Center);
                        SolidColorBrush dotBrush;
                        dotBrush.Color(TagColor(color));
                        dot.Background(dotBrush);
                        Grid::SetColumn(dot, 0);
                        row.Children().Append(dot);

                        Button view;
                        view.Content(winrt::box_value(name));
                        view.HorizontalAlignment(HorizontalAlignment::Stretch);
                        view.HorizontalContentAlignment(HorizontalAlignment::Left);
                        std::wstring viewName{ L"查看标签 " };
                        viewName.append(name.c_str(), name.size());
                        AutomationProperties::SetName(view, winrt::hstring{ viewName });
                        view.Click([dialog, &catalogAction, &actionTag, name](auto const&, auto const&)
                        {
                            catalogAction = L"view";
                            actionTag = name;
                            dialog.Hide();
                        });
                        Grid::SetColumn(view, 1);
                        row.Children().Append(view);

                        TextBlock countText;
                        std::wstringstream countLabel;
                        countLabel << count << L" 条";
                        countText.Text(winrt::hstring{ countLabel.str() });
                        countText.VerticalAlignment(VerticalAlignment::Center);
                        countText.Foreground(Application::Current().Resources()
                            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                        Grid::SetColumn(countText, 2);
                        row.Children().Append(countText);

                        Button rename;
                        rename.Content(winrt::box_value(L"重命名"));
                        rename.IsEnabled(!readOnly && !protectedTag);
                        AutomationProperties::SetName(rename, winrt::hstring{ L"重命名标签 " + std::wstring{ name } });
                        rename.Click([dialog, &catalogAction, &actionTag, name](auto const&, auto const&)
                        {
                            catalogAction = L"rename";
                            actionTag = name;
                            dialog.Hide();
                        });
                        Grid::SetColumn(rename, 3);
                        row.Children().Append(rename);

                        Button colorButton;
                        colorButton.Content(winrt::box_value(L"颜色"));
                        colorButton.IsEnabled(!readOnly);
                        AutomationProperties::SetName(colorButton, winrt::hstring{ L"设置标签颜色 " + std::wstring{ name } });
                        colorButton.Click([
                            dialog,
                            &catalogAction,
                            &actionTag,
                            &actionColor,
                            name,
                            color](auto const&, auto const&)
                        {
                            catalogAction = L"color";
                            actionTag = name;
                            actionColor = color;
                            dialog.Hide();
                        });
                        Grid::SetColumn(colorButton, 4);
                        row.Children().Append(colorButton);

                        Button remove;
                        remove.Content(winrt::box_value(L"删除"));
                        remove.IsEnabled(!readOnly && !protectedTag);
                        AutomationProperties::SetName(remove, winrt::hstring{ L"删除标签及全部记录 " + std::wstring{ name } });
                        remove.Click([
                            dialog,
                            &catalogAction,
                            &actionTag,
                            &actionCount,
                            name,
                            count](auto const&, auto const&)
                        {
                            catalogAction = L"delete";
                            actionTag = name;
                            actionCount = count;
                            dialog.Hide();
                        });
                        Grid::SetColumn(remove, 5);
                        row.Children().Append(remove);

                        tagRows->emplace_back(LowerText(name), row);
                        tagList.Children().Append(row);
                    }

                    if (tags.Size() == 0)
                    {
                        TextBlock empty;
                        empty.Text(L"暂无标签。输入名称后选择“新建标签”。");
                        empty.TextWrapping(TextWrapping::Wrap);
                        empty.Foreground(Application::Current().Resources()
                            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                        tagList.Children().Append(empty);
                    }

                    search.TextChanged([dialog, search, tagRows, readOnly](auto const&, auto const&)
                    {
                        auto const query = LowerText(search.Text());
                        for (auto const& [name, element] : *tagRows)
                        {
                            element.Visibility(query.empty() || name.find(query) != std::wstring::npos
                                ? Visibility::Visible
                                : Visibility::Collapsed);
                        }
                        dialog.IsPrimaryButtonEnabled(
                            !readOnly && !TrimHotkeyText(search.Text().c_str()).empty());
                    });

                    ScrollViewer scroller;
                    scroller.MaxHeight(520);
                    scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
                    scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
                    scroller.Content(tagList);
                    content.Children().Append(scroller);
                    dialog.Content(content);

                    auto const result = co_await dialog.ShowAsync();
                    if (result == ContentDialogResult::Primary)
                    {
                        auto const name = winrt::to_string(search.Text());
                        auto const mutationText = co_await RunRustOperationAsync([
                            core = m_core.get(),
                            name]
                        {
                            return core->CreateTag(name);
                        });
                        auto const mutation = JsonObject::Parse(mutationText);
                        SetStatus(mutation.GetNamedString(L"message", L"标签已创建。"));
                        continue;
                    }
                    if (catalogAction == L"view")
                    {
                        activeTag = actionTag;
                        continue;
                    }
                    if (catalogAction == L"rename")
                    {
                        ContentDialog renameDialog;
                        renameDialog.XamlRoot(RootGrid().XamlRoot());
                        renameDialog.Title(winrt::box_value(L"重命名标签"));
                        renameDialog.PrimaryButtonText(L"保存");
                        renameDialog.CloseButtonText(L"取消");
                        renameDialog.DefaultButton(ContentDialogButton::Primary);
                        TextBox newName;
                        newName.Text(actionTag);
                        newName.SelectAll();
                        newName.MaxLength(64);
                        AutomationProperties::SetName(newName, L"新标签名称");
                        renameDialog.Content(newName);
                        if (co_await renameDialog.ShowAsync() == ContentDialogResult::Primary)
                        {
                            auto const oldValue = winrt::to_string(actionTag);
                            auto const newValue = winrt::to_string(newName.Text());
                            auto const mutationText = co_await RunRustOperationAsync([
                                core = m_core.get(),
                                oldValue,
                                newValue]
                            {
                                return core->RenameTag(oldValue, newValue);
                            });
                            auto const mutation = JsonObject::Parse(mutationText);
                            SetStatus(mutation.GetNamedString(L"message", L"标签已重命名。"));
                            RefreshItems();
                        }
                        continue;
                    }
                    if (catalogAction == L"color")
                    {
                        ContentDialog colorDialog;
                        colorDialog.XamlRoot(RootGrid().XamlRoot());
                        colorDialog.Title(winrt::box_value(L"设置标签颜色"));
                        colorDialog.PrimaryButtonText(L"保存颜色");
                        colorDialog.CloseButtonText(L"取消");
                        colorDialog.DefaultButton(ContentDialogButton::Primary);
                        ColorPicker picker;
                        picker.IsAlphaEnabled(false);
                        picker.IsColorSpectrumVisible(true);
                        picker.IsColorSliderVisible(true);
                        picker.Color(TagColor(actionColor));
                        AutomationProperties::SetName(picker, L"标签颜色选择器");
                        colorDialog.Content(picker);
                        if (co_await colorDialog.ShowAsync() == ContentDialogResult::Primary)
                        {
                            auto const name = winrt::to_string(actionTag);
                            auto const color = winrt::to_string(TagColorHex(picker.Color()));
                            auto const mutationText = co_await RunRustOperationAsync([
                                core = m_core.get(),
                                name,
                                color]
                            {
                                return core->SetTagColor(name, color);
                            });
                            auto const mutation = JsonObject::Parse(mutationText);
                            SetStatus(mutation.GetNamedString(L"message", L"标签颜色已更新。"));
                        }
                        continue;
                    }
                    if (catalogAction == L"delete")
                    {
                        ContentDialog confirmation;
                        confirmation.XamlRoot(RootGrid().XamlRoot());
                        confirmation.Title(winrt::box_value(L"永久删除标签及记录？"));
                        confirmation.PrimaryButtonText(L"永久删除");
                        confirmation.CloseButtonText(L"取消");
                        confirmation.DefaultButton(ContentDialogButton::Close);
                        StackPanel warning;
                        warning.Spacing(8);
                        TextBlock message;
                        std::wstringstream text;
                        text << L"标签“" << actionTag.c_str() << L"”下的 " << actionCount
                             << L" 条记录将被永久删除；同步设备也会收到删除墓碑。此操作无法撤销。";
                        message.Text(winrt::hstring{ text.str() });
                        message.TextWrapping(TextWrapping::Wrap);
                        warning.Children().Append(message);
                        confirmation.Content(warning);
                        if (co_await confirmation.ShowAsync() == ContentDialogResult::Primary)
                        {
                            auto const name = winrt::to_string(actionTag);
                            auto const mutationText = co_await RunRustOperationAsync([
                                core = m_core.get(),
                                name]
                            {
                                return core->DeleteTag(name);
                            });
                            auto const mutation = JsonObject::Parse(mutationText);
                            SetStatus(mutation.GetNamedString(L"message", L"标签及记录已删除。"));
                            RefreshItems();
                        }
                        continue;
                    }

                    m_suspendLifecycle = false;
                    SetStatus(L"已关闭标签管理器。");
                    SearchBox().Focus(FocusState::Programmatic);
                    co_return;
                }

                auto const entriesResponse = JsonObject::Parse(
                    tiez::probe::RustCoreBridge::Utf8ToHstring(
                        m_core->TagEntries(winrt::to_string(activeTag))));
                auto const readOnly = entriesResponse.GetNamedBoolean(L"read_only", true);
                auto const total = static_cast<std::uint64_t>(
                    entriesResponse.GetNamedNumber(L"total", 0));
                auto const entries = entriesResponse.GetNamedArray(L"items");
                auto const protectedTag = IsProtectedTagName(activeTag);
                std::wstring entryAction;
                std::int64_t actionEntryId{};

                ContentDialog entriesDialog;
                entriesDialog.XamlRoot(RootGrid().XamlRoot());
                std::wstring entriesTitle{ L"标签：" };
                entriesTitle.append(activeTag.c_str(), activeTag.size());
                entriesDialog.Title(winrt::box_value(winrt::hstring{ entriesTitle }));
                entriesDialog.PrimaryButtonText(L"添加文本");
                entriesDialog.IsPrimaryButtonEnabled(!readOnly && !protectedTag);
                entriesDialog.CloseButtonText(L"返回标签");
                entriesDialog.DefaultButton(ContentDialogButton::Close);
                entriesDialog.MinWidth(760);

                StackPanel entriesContent;
                entriesContent.Spacing(10);
                TextBlock summary;
                std::wstringstream summaryText;
                summaryText << L"共 " << total << L" 条记录";
                if (total > entries.Size())
                {
                    summaryText << L"，当前显示前 " << entries.Size() << L" 条";
                }
                if (protectedTag)
                {
                    summaryText << L"。内置敏感标签不直接接受手动文本；请先保存到普通标签，再从记录详情安全添加此标签";
                }
                summary.Text(winrt::hstring{ summaryText.str() });
                summary.Foreground(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                entriesContent.Children().Append(summary);

                StackPanel entryList;
                entryList.Spacing(8);
                for (std::uint32_t index = 0; index < entries.Size(); ++index)
                {
                    auto const entry = entries.GetObjectAt(index);
                    auto const entryId = static_cast<std::int64_t>(entry.GetNamedNumber(L"id"));
                    auto const sensitive = entry.GetNamedBoolean(L"is_sensitive", false);
                    auto const pinned = entry.GetNamedBoolean(L"is_pinned", false);
                    auto const preview = entry.GetNamedString(L"preview");
                    auto const source = entry.GetNamedString(L"source_app");
                    auto const type = ContentTypeLabel(entry.GetNamedString(L"content_type"));
                    auto const captured = CapturedAtLabel(entry.GetNamedString(L"captured_at"));
                    auto const useCount = static_cast<std::uint64_t>(
                        entry.GetNamedNumber(L"use_count", 0));

                    Grid row;
                    row.ColumnSpacing(8);
                    ColumnDefinition contentColumn;
                    contentColumn.Width(GridLength{ 1, GridUnitType::Star });
                    for (int button = 0; button < 3; ++button)
                    {
                        ColumnDefinition definition;
                        definition.Width(GridLengthHelper::Auto());
                        row.ColumnDefinitions().Append(button == 0 ? contentColumn : definition);
                    }
                    ColumnDefinition deleteColumn;
                    deleteColumn.Width(GridLengthHelper::Auto());
                    row.ColumnDefinitions().Append(deleteColumn);

                    Button details;
                    details.HorizontalAlignment(HorizontalAlignment::Stretch);
                    details.HorizontalContentAlignment(HorizontalAlignment::Left);
                    StackPanel detailsContent;
                    detailsContent.Spacing(3);
                    TextBlock previewText;
                    previewText.Text(sensitive ? L"敏感内容，预览已隐藏" : preview);
                    previewText.TextWrapping(TextWrapping::Wrap);
                    previewText.MaxLines(3);
                    previewText.TextTrimming(TextTrimming::CharacterEllipsis);
                    detailsContent.Children().Append(previewText);
                    TextBlock metadata;
                    std::wstringstream metadataText;
                    metadataText << type.c_str() << L" · " << source.c_str() << L" · "
                                 << captured.c_str() << L" · 使用 " << useCount << L" 次";
                    if (pinned) metadataText << L" · 已置顶";
                    metadata.Text(winrt::hstring{ metadataText.str() });
                    metadata.Foreground(Application::Current().Resources()
                        .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                    detailsContent.Children().Append(metadata);
                    details.Content(detailsContent);
                    AutomationProperties::SetName(details, sensitive
                        ? L"打开敏感记录详情，预览已隐藏"
                        : winrt::hstring{ L"打开记录详情 " + std::wstring{ preview } });
                    details.Click([
                        entriesDialog,
                        &entryAction,
                        &actionEntryId,
                        entryId](auto const&, auto const&)
                    {
                        entryAction = L"details";
                        actionEntryId = entryId;
                        entriesDialog.Hide();
                    });
                    Grid::SetColumn(details, 0);
                    row.Children().Append(details);

                    Button paste;
                    paste.Content(winrt::box_value(L"粘贴"));
                    paste.IsEnabled(!readOnly);
                    AutomationProperties::SetName(paste, L"粘贴此标签记录");
                    paste.Click([
                        entriesDialog,
                        &entryAction,
                        &actionEntryId,
                        entryId](auto const&, auto const&)
                    {
                        entryAction = L"paste";
                        actionEntryId = entryId;
                        entriesDialog.Hide();
                    });
                    Grid::SetColumn(paste, 1);
                    row.Children().Append(paste);

                    Button open;
                    open.Content(winrt::box_value(L"打开"));
                    open.IsEnabled(!sensitive);
                    AutomationProperties::SetName(open, L"使用默认应用打开此记录");
                    open.Click([
                        entriesDialog,
                        &entryAction,
                        &actionEntryId,
                        entryId](auto const&, auto const&)
                    {
                        entryAction = L"open";
                        actionEntryId = entryId;
                        entriesDialog.Hide();
                    });
                    Grid::SetColumn(open, 2);
                    row.Children().Append(open);

                    Button remove;
                    remove.Content(winrt::box_value(L"删除"));
                    remove.IsEnabled(!readOnly);
                    AutomationProperties::SetName(remove, L"永久删除此标签记录");
                    remove.Click([
                        entriesDialog,
                        &entryAction,
                        &actionEntryId,
                        entryId](auto const&, auto const&)
                    {
                        entryAction = L"delete";
                        actionEntryId = entryId;
                        entriesDialog.Hide();
                    });
                    Grid::SetColumn(remove, 3);
                    row.Children().Append(remove);

                    entryList.Children().Append(row);
                }

                if (entries.Size() == 0)
                {
                    TextBlock empty;
                    empty.Text(L"该标签暂无记录。可选择“添加文本”创建一条手动记录。");
                    empty.TextWrapping(TextWrapping::Wrap);
                    empty.Foreground(Application::Current().Resources()
                        .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
                    entryList.Children().Append(empty);
                }

                ScrollViewer entriesScroller;
                entriesScroller.MaxHeight(540);
                entriesScroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
                entriesScroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
                entriesScroller.Content(entryList);
                entriesContent.Children().Append(entriesScroller);
                entriesDialog.Content(entriesContent);

                auto const entriesResult = co_await entriesDialog.ShowAsync();
                if (entriesResult == ContentDialogResult::Primary)
                {
                    ContentDialog addDialog;
                    addDialog.XamlRoot(RootGrid().XamlRoot());
                    addDialog.Title(winrt::box_value(L"添加带标签文本"));
                    addDialog.PrimaryButtonText(L"添加");
                    addDialog.CloseButtonText(L"取消");
                    addDialog.DefaultButton(ContentDialogButton::Primary);
                    TextBox text;
                    text.PlaceholderText(L"输入要保存的文本；首尾空白会保留");
                    text.AcceptsReturn(true);
                    text.TextWrapping(TextWrapping::Wrap);
                    text.MinHeight(180);
                    text.MaxLength(1'000'000);
                    AutomationProperties::SetName(text, L"手动文本内容");
                    addDialog.Content(text);
                    if (co_await addDialog.ShowAsync() == ContentDialogResult::Primary)
                    {
                        auto const tag = winrt::to_string(activeTag);
                        auto const contentValue = winrt::to_string(text.Text());
                        auto const mutationText = co_await RunRustOperationAsync([
                            core = m_core.get(),
                            tag,
                            contentValue]
                        {
                            return core->CreateTaggedText(tag, contentValue);
                        });
                        auto const mutation = JsonObject::Parse(mutationText);
                        SetStatus(mutation.GetNamedString(L"message", L"手动文本已添加。"));
                        RefreshItems();
                    }
                    continue;
                }
                if (entryAction == L"details")
                {
                    SelectEntry(actionEntryId);
                    ShowContent(actionEntryId);
                    m_suspendLifecycle = false;
                    co_return;
                }
                if (entryAction == L"paste")
                {
                    ApplyAction(actionEntryId, "paste-plain");
                    co_return;
                }
                if (entryAction == L"open")
                {
                    m_suspendLifecycle = false;
                    OpenEntry(actionEntryId);
                    co_return;
                }
                if (entryAction == L"delete")
                {
                    ContentDialog confirmation;
                    confirmation.XamlRoot(RootGrid().XamlRoot());
                    confirmation.Title(winrt::box_value(L"永久删除记录？"));
                    confirmation.Content(winrt::box_value(
                        L"该记录会从本机和同步历史中删除；此操作无法撤销。"));
                    confirmation.PrimaryButtonText(L"永久删除");
                    confirmation.CloseButtonText(L"取消");
                    confirmation.DefaultButton(ContentDialogButton::Close);
                    if (co_await confirmation.ShowAsync() == ContentDialogResult::Primary)
                    {
                        auto const mutationText = co_await RunRustOperationAsync([
                            core = m_core.get(),
                            actionEntryId]
                        {
                            return core->ApplyAction(actionEntryId, "delete");
                        });
                        auto const mutation = JsonObject::Parse(mutationText);
                        SetStatus(mutation.GetNamedString(L"message", L"记录已删除。"));
                        RefreshItems();
                    }
                    continue;
                }

                activeTag.clear();
            }
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"标签管理器失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"标签管理器失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    winrt::fire_and_forget MainWindow::ConfirmClearHistoryAsync()
    {
        auto lifetime = get_strong();
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法清空历史记录。");
            co_return;
        }
        if (m_readOnly)
        {
            SetStatus(L"当前数据库以只读方式打开，无法清空历史记录。");
            co_return;
        }

        try
        {
            ContentDialog dialog;
            dialog.XamlRoot(RootGrid().XamlRoot());
            dialog.Title(winrt::box_value(L"清空剪贴板历史？"));
            dialog.Content(winrt::box_value(
                L"将删除所有未固定且没有标签的历史记录。已固定、带标签和受保护的敏感记录会保留。此操作无法撤销。"));
            dialog.PrimaryButtonText(L"清空历史");
            dialog.CloseButtonText(L"取消");
            dialog.DefaultButton(ContentDialogButton::Close);
            m_suspendLifecycle = true;
            auto const result = co_await dialog.ShowAsync();
            if (result == ContentDialogResult::Primary)
            {
                ApplyAction(0, "clear");
            }
            else
            {
                m_suspendLifecycle = false;
                SetStatus(L"已取消清空历史记录。");
            }
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"无法确认清空历史：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"无法确认清空历史：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::HideButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (!m_showTimer)
        {
            m_showTimer = DispatcherQueue().CreateTimer();
            m_showTimer.Interval(std::chrono::seconds(5));
            m_showTimer.IsRepeating(false);
            m_showTimer.Tick([this](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
                ShowWindow(GetWindowHandle(), SW_SHOW);
                Activate();
                SetStatus(L"窗口已在 5 秒后恢复，Rust 核心始终保持在当前进程中。");
            });
        }

        m_suspendLifecycle = true;
        SetStatus(L"窗口将隐藏 5 秒，可在此期间采样进程并比较空闲内存。");
        m_showTimer.Start();
        ShowWindow(GetWindowHandle(), SW_HIDE);
    }

    void MainWindow::SettingsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法加载设置。");
            return;
        }

        try
        {
            EnsureSettingsDialog();
            LoadSettings();
            RefreshAutostartStateAsync(false);
            m_suspendLifecycle = true;
            m_settingsDialog.XamlRoot(RootGrid().XamlRoot());
            (void)m_settingsDialog.ShowAsync();
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"打开设置失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"打开设置失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::PinWindowCheck_Changed(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_settingsLoading)
        {
            return;
        }
        auto const pinned = PinWindowCheck().IsChecked().Value();
        if (PersistSetting(
            "app.window_pinned",
            pinned ? "true" : "false",
            L"固定窗口"))
        {
            ApplyPinnedWindow(pinned);
        }
        else
        {
            LoadSettings();
        }
    }

    void MainWindow::TypeAllButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        SetTypeFilter({});
    }

    void MainWindow::TypeChip_Click(IInspectable const& sender, RoutedEventArgs const&)
    {
        auto const button = sender.as<ToggleButton>();
        auto const tag = winrt::unbox_value_or<winrt::hstring>(button.Tag(), L"");
        auto const filter = winrt::to_string(tag);
        if (!button.IsChecked().Value())
        {
            SetTypeFilter({});
            return;
        }
        SetTypeFilter(filter);
    }

    void MainWindow::TagsTextBox_KeyDown(IInspectable const&, KeyRoutedEventArgs const& args)
    {
        if (args.Key() != VirtualKey::Enter)
        {
            return;
        }
        if (m_imeComposing || m_ignoreNextEnter || (GetKeyState(VK_PROCESSKEY) & 0x8000))
        {
            m_ignoreNextEnter = false;
            return;
        }
        if (!m_readOnly)
        {
            SaveSelectedTags();
            args.Handled(true);
        }
    }

    void MainWindow::SaveTagsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        SaveSelectedTags();
    }

    void MainWindow::OpenSelectedButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_detailsEntryId)
        {
            OpenEntry(*m_detailsEntryId);
        }
        else
        {
            SetStatus(L"请先选择一条剪贴板记录。");
        }
    }

    void MainWindow::AnalyzeImageButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        AnalyzeSelectedImageAsync();
    }

    void MainWindow::CopyImageAnalysisButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        if (m_imageAnalysisCopyText.empty())
        {
            SetStatus(L"当前没有可复制的图片识别结果。");
            return;
        }

        try
        {
            DataPackage package;
            package.SetText(winrt::hstring{ m_imageAnalysisCopyText });
            Clipboard::SetContent(package);
            Clipboard::Flush();
            SetStatus(L"图片识别结果已复制到剪贴板。");
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"复制图片识别结果失败：", error.message()));
        }
    }

    void MainWindow::OnToggleHotkey()
    {
        if (IsWindowVisible(GetWindowHandle()))
        {
            HideMainWindow();
            return;
        }
        ShowMainWindow(true);
    }

    void MainWindow::RefreshItems()
    {
        HideHoverPreview();
        if (!m_core)
        {
            return;
        }

        try
        {
            auto const snapshot = m_core->Snapshot(CurrentQuery());
            auto const root = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(snapshot));
            auto const items = root.GetNamedArray(L"items");
            auto const adapter = root.GetNamedString(L"adapter");
            auto const readOnly = root.GetNamedBoolean(L"read_only");
            m_readOnly = readOnly;
            ClearHistoryButton().IsEnabled(!readOnly);

            AdapterText().Text(AdapterLabel(adapter));
            if (readOnly)
            {
                ReadOnlyText().Text(L"真实 TieZ 历史 · 操作已禁用");
            }
            else if (adapter == L"sqlite")
            {
                ReadOnlyText().Text(L"真实 TieZ 历史 · 可写");
            }
            else
            {
                ReadOnlyText().Text(L"示例数据 · 操作已启用");
            }

            auto const previousId = (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
                ? std::optional<std::int64_t>{ m_entryIds[static_cast<std::size_t>(m_selectedIndex)] }
                : std::nullopt;

            ItemsPanel().Spacing(m_compactMode ? 6 : 12);
            ItemsPanel().Children().Clear();
            m_entryIds.clear();
            m_pinnedIds.clear();
            m_cards.clear();
            m_tagsById.clear();
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                auto const item = items.GetObjectAt(index);
                if (item.GetNamedBoolean(L"is_pinned"))
                {
                    m_pinnedIds.push_back(
                        static_cast<std::int64_t>(item.GetNamedNumber(L"id")));
                }
            }
            m_canReorderPinned = !readOnly
                && CurrentQuery().empty()
                && m_pinnedIds.size() > 1;
            for (std::uint32_t index = 0; index < items.Size(); ++index)
            {
                auto const item = items.GetObjectAt(index);
                auto const entryId = static_cast<std::int64_t>(item.GetNamedNumber(L"id"));
                m_entryIds.push_back(entryId);
                m_tagsById.emplace(entryId, ItemTags(item));
                auto const card = CreateItemCard(item, readOnly, index);
                ItemsPanel().Children().Append(card);
            }

            m_selectedIndex = -1;
            if (previousId)
            {
                for (std::size_t index = 0; index < m_entryIds.size(); ++index)
                {
                    if (m_entryIds[index] == *previousId)
                    {
                        m_selectedIndex = static_cast<int>(index);
                        break;
                    }
                }
            }
            if (m_selectedIndex < 0 && !m_entryIds.empty())
            {
                m_selectedIndex = 0;
            }
            UpdateSelectionVisuals();

            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                auto const selectedId = m_entryIds[static_cast<std::size_t>(m_selectedIndex)];
                TagsTextBox().IsEnabled(!readOnly);
                SaveTagsButton().IsEnabled(!readOnly);
                if (TagsTextBox().FocusState() == FocusState::Unfocused)
                {
                    TagsTextBox().Text(JoinTags(m_tagsById[selectedId]));
                    ShowContent(selectedId);
                }
            }
            else
            {
                m_detailsEntryId.reset();
                DetailsTitleText().Text(L"剪贴板详情");
                DetailsMetadataText().Text(L"没有可显示的记录");
                DetailsContentText().Text(L"");
                OpenSelectedButton().IsEnabled(false);
                ShowDetailsImage({}, {});
                TagsTextBox().Text(L"");
                TagsTextBox().IsEnabled(false);
                SaveTagsButton().IsEnabled(false);
            }

            EmptyState().Visibility(items.Size() == 0 ? Visibility::Visible : Visibility::Collapsed);

            std::wstringstream status;
            status << AdapterLabel(adapter).c_str()
                   << (readOnly ? L" · 只读 · " : L" · 可写 · ")
                   << L"Rust ABI " << static_cast<std::uint32_t>(root.GetNamedNumber(L"abi_version"))
                   << L" · 第 " << static_cast<std::uint64_t>(root.GetNamedNumber(L"generation"))
                   << L" 代 · " << items.Size() << L" 条可见记录 · 适配器已就绪";
            SetStatus(winrt::hstring{ status.str() });
            WriteReadyMarker();
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"刷新失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"刷新失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    UIElement MainWindow::CreateItemCard(JsonObject const& item, bool readOnly, std::uint32_t index)
    {
        auto const entryId = static_cast<std::int64_t>(item.GetNamedNumber(L"id"));
        auto const isPinned = item.GetNamedBoolean(L"is_pinned");
        auto const isSensitive = item.GetNamedBoolean(L"is_sensitive");
        auto const typeLabel = ContentTypeLabel(item.GetNamedString(L"content_type"));
        auto const sourceLabel = item.GetNamedString(L"source_app");
        auto const capturedAtLabel = CapturedAtLabel(item.GetNamedString(L"captured_at"));
        auto const previewLabel = item.GetNamedString(L"preview");

        Border card;
        card.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"ClipboardCardStyle")).as<Style>());
        card.Padding(m_compactMode
            ? ThicknessHelper::FromLengths(12, 8, 12, 8)
            : ThicknessHelper::FromLengths(16, 16, 16, 16));
        card.PointerPressed([this, entryId, index](auto const&, auto const&)
        {
            m_selectedIndex = static_cast<int>(index);
            UpdateSelectionVisuals();
            ShowContent(entryId);
        });
        card.PointerEntered([this, entryId](auto const&, auto const&)
        {
            if (m_compactMode)
            {
                ShowHoverPreview(entryId);
            }
        });
        card.PointerExited([this](auto const&, auto const&)
        {
            HideHoverPreview();
        });
        card.DoubleTapped([this, entryId, readOnly](auto const&, auto const&)
        {
            if (!readOnly)
            {
                ApplyAction(entryId, "paste-plain");
            }
        });
        AutomationProperties::SetName(
            card,
            isSensitive ? L"敏感内容，预览已隐藏" : previewLabel);
        AttachCardCommands(card, entryId, readOnly, isSensitive);
        AttachPinnedReorder(card, entryId, isPinned && m_canReorderPinned);
        m_cards.push_back(card);

        StackPanel content;
        content.Spacing(m_compactMode ? 5 : 10);

        Grid metadata;
        metadata.ColumnSpacing(12);
        ColumnDefinition typeColumn;
        typeColumn.Width(GridLengthHelper::Auto());
        ColumnDefinition sourceColumn;
        sourceColumn.Width(GridLength{ 1, GridUnitType::Star });
        ColumnDefinition timeColumn;
        timeColumn.Width(GridLengthHelper::Auto());
        metadata.ColumnDefinitions().Append(typeColumn);
        metadata.ColumnDefinitions().Append(sourceColumn);
        metadata.ColumnDefinitions().Append(timeColumn);

        TextBlock type;
        type.Text(typeLabel);
        type.FontWeight(Windows::UI::Text::FontWeights::SemiBold());

        TextBlock source;
        source.Text(sourceLabel);
        source.Opacity(0.72);
        Grid::SetColumn(source, 1);

        TextBlock capturedAt;
        capturedAt.Text(capturedAtLabel);
        capturedAt.Opacity(0.72);
        Grid::SetColumn(capturedAt, 2);

        metadata.Children().Append(type);
        metadata.Children().Append(source);
        metadata.Children().Append(capturedAt);

        if (isSensitive)
        {
            TextBlock sensitive;
            sensitive.Text(L"敏感内容 · 预览已隐藏");
            sensitive.Foreground(SolidColorBrush{ Windows::UI::Color{ 255, 196, 43, 28 } });
            content.Children().Append(sensitive);
        }

        auto const tags = ItemTags(item);
        if (!tags.empty())
        {
            StackPanel tagPanel;
            tagPanel.Orientation(Orientation::Horizontal);
            tagPanel.Spacing(6);
            for (auto const& tag : tags)
            {
                Border chip;
                chip.Padding(ThicknessHelper::FromLengths(8, 3, 8, 3));
                chip.CornerRadius(CornerRadiusHelper::FromUniformRadius(10));
                chip.Background(Application::Current().Resources()
                    .Lookup(winrt::box_value(L"AccentFillColorSecondaryBrush"))
                    .as<Brush>());
                TextBlock label;
                label.Text(tag);
                label.FontSize(12);
                chip.Child(label);
                std::wstring automationName{ L"标签：" };
                automationName.append(tag.c_str(), tag.size());
                AutomationProperties::SetName(chip, winrt::hstring{ automationName });
                tagPanel.Children().Append(chip);
            }
            content.Children().Append(tagPanel);
        }

        TextBlock preview;
        preview.Text(previewLabel);
        AutomationProperties::SetName(
            preview,
            isSensitive ? L"敏感内容预览已隐藏" : previewLabel);
        preview.TextWrapping(TextWrapping::WrapWholeWords);
        preview.IsTextSelectionEnabled(true);
        preview.MaxHeight(m_compactMode ? 48 : 112);

        StackPanel actions;
        actions.Orientation(Orientation::Horizontal);
        actions.Spacing(8);
        auto detailsButton = ActionButton(
            L"查看详情",
            [this, entryId, index]
            {
                m_selectedIndex = static_cast<int>(index);
                UpdateSelectionVisuals();
                ShowContent(entryId);
            });
        auto pinButton = ActionButton(
            isPinned ? L"取消置顶" : L"置顶",
            [this, entryId] { ApplyAction(entryId, "pin"); });
        Button moveUpButton{ nullptr };
        Button moveDownButton{ nullptr };
        if (isPinned)
        {
            auto const position = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), entryId);
            auto const pinnedIndex = position == m_pinnedIds.end()
                ? m_pinnedIds.size()
                : static_cast<std::size_t>(std::distance(m_pinnedIds.begin(), position));
            moveUpButton = ActionButton(
                L"上移",
                [this, entryId] { MovePinnedEntry(entryId, -1); });
            moveDownButton = ActionButton(
                L"下移",
                [this, entryId] { MovePinnedEntry(entryId, 1); });
            moveUpButton.IsEnabled(m_canReorderPinned && pinnedIndex > 0);
            moveDownButton.IsEnabled(
                m_canReorderPinned && pinnedIndex + 1 < m_pinnedIds.size());
        }
        auto pastePlainButton = ActionButton(
            L"纯文本粘贴",
            [this, entryId] { ApplyAction(entryId, "paste-plain"); });
        auto pasteRichButton = ActionButton(
            L"富文本粘贴",
            [this, entryId] { ApplyAction(entryId, "paste-rich"); });
        auto copyButton = ActionButton(
            L"复制",
            [this, entryId] { ApplyAction(entryId, "copy-plain"); });
        auto deleteButton = ActionButton(
            L"删除",
            [this, entryId] { ApplyAction(entryId, "delete"); });

        pinButton.IsEnabled(!readOnly);
        pastePlainButton.IsEnabled(!readOnly);
        pasteRichButton.IsEnabled(!readOnly);
        copyButton.IsEnabled(!readOnly);
        deleteButton.IsEnabled(!readOnly);

        actions.Children().Append(detailsButton);
        actions.Children().Append(pinButton);
        if (moveUpButton)
        {
            actions.Children().Append(moveUpButton);
            actions.Children().Append(moveDownButton);
        }
        actions.Children().Append(pastePlainButton);
        actions.Children().Append(pasteRichButton);
        actions.Children().Append(copyButton);
        actions.Children().Append(deleteButton);

        content.Children().Append(metadata);
        content.Children().Append(preview);
        if (!m_compactMode)
        {
            content.Children().Append(actions);
        }
        card.Child(content);

        ListViewItem cardHost;
        cardHost.Padding(ThicknessHelper::FromUniformLength(0));
        cardHost.HorizontalContentAlignment(HorizontalAlignment::Stretch);
        cardHost.IsTabStop(true);
        cardHost.Content(card);
        cardHost.ContextFlyout(card.ContextFlyout());

        std::wstring automationName{ isPinned ? L"已置顶" : L"未置顶" };
        if (isSensitive)
        {
            automationName.append(L"，敏感内容");
        }
        automationName.append(L"剪贴板记录，类型：");
        automationName.append(typeLabel.c_str(), typeLabel.size());
        automationName.append(L"，来源：");
        automationName.append(sourceLabel.c_str(), sourceLabel.size());
        automationName.append(L"，时间：");
        automationName.append(capturedAtLabel.c_str(), capturedAtLabel.size());
        if (isSensitive)
        {
            automationName.append(L"，预览：已隐藏");
        }
        else
        {
            automationName.append(L"，预览：");
            automationName.append(previewLabel.c_str(), previewLabel.size());
        }
        AutomationProperties::SetName(cardHost, winrt::hstring{ automationName });
        AutomationProperties::SetHelpText(
            cardHost,
            readOnly
                ? L"按 Enter 或空格查看详情；按 Shift+F10 打开只读操作菜单。"
                : L"按 Enter 或空格查看详情；双击执行纯文本粘贴；按 Shift+F10 打开更多操作。");
        cardHost.KeyDown([this, entryId, index](auto const&, KeyRoutedEventArgs const& args)
        {
            if (args.Key() != VirtualKey::Enter && args.Key() != VirtualKey::Space)
            {
                return;
            }
            m_selectedIndex = static_cast<int>(index);
            UpdateSelectionVisuals();
            ShowContent(entryId);
            args.Handled(true);
        });
        return cardHost;
    }

    void MainWindow::ShowContent(std::int64_t entryId)
    {
        if (!m_core)
        {
            return;
        }

        OpenSelectedButton().IsEnabled(false);
        ImageAnalysisPanel().Visibility(Visibility::Collapsed);
        ImageAnalysisProgress().IsActive(false);
        ImageAnalysisProgress().Visibility(Visibility::Collapsed);
        ImageAnalysisResultText().Text(L"");
        CopyImageAnalysisButton().Visibility(Visibility::Collapsed);
        m_imageAnalysisLoaded = false;
        m_imageAnalysisCopyText.clear();

        try
        {
            auto const value = m_core->Content(entryId);
            auto const content = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const contentType = content.GetNamedString(L"content_type");
            auto const available = content.GetNamedBoolean(L"available");
            auto const isSensitive = content.GetNamedBoolean(L"is_sensitive");
            OpenSelectedButton().IsEnabled(available && !isSensitive);

            std::wstringstream title;
            title << L"记录 " << entryId;
            DetailsTitleText().Text(winrt::hstring{ title.str() });
            m_detailsEntryId = entryId;
            if (auto const tags = m_tagsById.find(entryId); tags != m_tagsById.end())
            {
                TagsTextBox().Text(JoinTags(tags->second));
            }
            TagsTextBox().IsEnabled(!m_readOnly);
            SaveTagsButton().IsEnabled(!m_readOnly);

            std::wstring metadata{ ContentTypeLabel(contentType).c_str() };
            metadata.append(isSensitive ? L" · 敏感内容" : L" · 内容可用");
            DetailsMetadataText().Text(metadata);

            auto const canAnalyzeImage = m_productionData
                && entryId > 0
                && contentType == L"image"
                && available
                && !isSensitive;
            if (canAnalyzeImage)
            {
                ImageAnalysisPanel().Visibility(Visibility::Visible);
                if (m_imageAnalysisBusy)
                {
                    SetImageAnalysisBusy(
                        true,
                        m_imageAnalysisEntryId == entryId
                            ? L"正在识别当前图片，请稍候……"
                            : L"正在后台识别另一张图片，请稍候……");
                }
                else
                {
                    try
                    {
                        auto const response = JsonObject::Parse(
                            tiez::probe::RustCoreBridge::Utf8ToHstring(
                                m_core->ImageAnalysis(entryId)));
                        ShowImageAnalysis(response);
                    }
                    catch (winrt::hresult_error const& error)
                    {
                        SetImageAnalysisBusy(
                            false,
                            StatusMessage(L"读取图片识别缓存失败：", error.message()));
                    }
                    catch (std::exception const& error)
                    {
                        SetImageAnalysisBusy(
                            false,
                            StatusMessage(
                                L"读取图片识别缓存失败：",
                                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
                    }
                }
            }

            if (available && !isSensitive)
            {
                auto displayContent = content.GetNamedString(L"content");
                if (displayContent.empty())
                {
                    auto const htmlContent = content.GetNamedValue(L"html_content");
                    if (htmlContent.ValueType() == JsonValueType::String)
                    {
                        displayContent = htmlContent.GetString();
                    }
                }

                DetailsContentText().Text(displayContent);
                ShowDetailsImage(contentType, displayContent);
                SetStatus(L"已从独立于 Tauri 的 Rust 核心加载完整内容。");
            }
            else
            {
                DetailsContentText().Text(isSensitive
                    ? L"此内容受隐私保护，无法显示。"
                    : L"此内容当前不可用。");
                ShowDetailsImage(contentType, {});
                SetStatus(L"已加载内容元数据，原始内容仍受保护。");
            }
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"查询内容失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"查询内容失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::SetImageAnalysisBusy(bool busy, winrt::hstring const& message)
    {
        m_imageAnalysisBusy = busy;
        ImageAnalysisProgress().IsActive(busy);
        ImageAnalysisProgress().Visibility(busy ? Visibility::Visible : Visibility::Collapsed);
        AnalyzeImageButton().IsEnabled(
            m_productionData
            && ImageAnalysisPanel().Visibility() == Visibility::Visible
            && !busy);
        if (!message.empty())
        {
            ImageAnalysisStatusText().Text(message);
        }
    }

    void MainWindow::ShowImageAnalysis(JsonObject const& response)
    {
        auto const value = response.GetNamedValue(L"analysis");
        if (value.ValueType() == JsonValueType::Null)
        {
            m_imageAnalysisLoaded = false;
            m_imageAnalysisCopyText.clear();
            AnalyzeImageButton().Content(winrt::box_value(L"开始识别"));
            ImageAnalysisResultText().Text(L"");
            CopyImageAnalysisButton().Visibility(Visibility::Collapsed);
            SetImageAnalysisBusy(
                false,
                L"尚未识别。非敏感可写记录会把结果加入本地搜索索引。");
            return;
        }

        auto const analysis = value.GetObject();
        auto const text = analysis.GetNamedString(L"text", L"");
        auto const qrCodes = analysis.GetNamedArray(L"qrCodes", JsonArray{});
        auto const optionalString = [&analysis](winrt::hstring const& name)
        {
            if (!analysis.HasKey(name))
            {
                return winrt::hstring{};
            }
            auto const field = analysis.GetNamedValue(name);
            return field.ValueType() == JsonValueType::String
                ? field.GetString()
                : winrt::hstring{};
        };
        auto const language = optionalString(L"language");
        auto const cached = analysis.GetNamedBoolean(L"cached", false);
        auto const persisted = analysis.GetNamedBoolean(L"persisted", false);
        auto const ocrError = optionalString(L"ocrError");

        std::wstringstream display;
        std::wstringstream copy;
        if (!text.empty())
        {
            display << L"识别文字";
            if (!language.empty())
            {
                display << L"（" << language.c_str() << L"）";
            }
            display << L"\n" << text.c_str();
            copy << text.c_str();
        }
        for (std::uint32_t index = 0; index < qrCodes.Size(); ++index)
        {
            auto const code = qrCodes.GetStringAt(index);
            if (!display.str().empty())
            {
                display << L"\n\n";
            }
            display << L"二维码 " << (index + 1) << L"\n" << code.c_str();
            if (!copy.str().empty())
            {
                copy << L"\n\n";
            }
            copy << code.c_str();
        }

        m_imageAnalysisLoaded = true;
        m_imageAnalysisCopyText = copy.str();
        AnalyzeImageButton().Content(winrt::box_value(L"重新识别"));
        ImageAnalysisResultText().Text(winrt::hstring{ display.str() });
        CopyImageAnalysisButton().Visibility(
            m_imageAnalysisCopyText.empty() ? Visibility::Collapsed : Visibility::Visible);

        std::wstring status = cached ? L"已加载本地识别缓存。" : L"图片识别完成。";
        if (m_imageAnalysisCopyText.empty())
        {
            status = ocrError.empty()
                ? L"没有识别到文字或二维码。"
                : std::wstring{ L"系统 OCR 不可用：" } + ocrError.c_str();
        }
        if (!persisted)
        {
            status.append(L" 本次结果仅在内存中显示，未写入搜索索引。");
        }
        SetImageAnalysisBusy(false, winrt::hstring{ status });
    }

    winrt::fire_and_forget MainWindow::AnalyzeSelectedImageAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || !m_detailsEntryId || m_imageAnalysisBusy || !m_productionData)
        {
            co_return;
        }

        auto const entryId = *m_detailsEntryId;
        auto const force = m_imageAnalysisLoaded;
        auto const uiThread = winrt::apartment_context{};
        m_imageAnalysisEntryId = entryId;
        SetImageAnalysisBusy(true, L"正在使用 Windows OCR 和本地二维码解码器识别图片……");
        std::string response;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            response = m_core->AnalyzeImage(entryId, force);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        m_imageAnalysisBusy = false;
        m_imageAnalysisEntryId.reset();
        if (!m_detailsEntryId || *m_detailsEntryId != entryId)
        {
            if (m_detailsEntryId)
            {
                ShowContent(*m_detailsEntryId);
            }
            co_return;
        }
        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"图片识别失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetImageAnalysisBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            auto const analysis = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            ShowImageAnalysis(analysis);
            SetStatus(L"图片识别完成；可复制结果，已持久化的结果也可直接搜索。");
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取图片识别结果：", error.message());
            SetImageAnalysisBusy(false, message);
            SetStatus(message);
        }
    }

    void MainWindow::OpenEntry(std::int64_t entryId)
    {
        if (!m_core)
        {
            SetStatus(L"Rust 核心尚未就绪，暂时无法打开内容。");
            return;
        }

        HideHoverPreview();
        try
        {
            auto const value = m_core->PrepareOpenContent(entryId);
            auto const plan = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            if (!plan.GetNamedBoolean(L"requires_confirmation"))
            {
                LaunchOpenPlan(plan);
                return;
            }

            ContentDialog confirmation;
            confirmation.Title(winrt::box_value(L"确认打开外部内容"));
            confirmation.PrimaryButtonText(L"继续打开");
            confirmation.CloseButtonText(L"取消");
            confirmation.DefaultButton(ContentDialogButton::Close);
            confirmation.XamlRoot(RootGrid().XamlRoot());

            StackPanel message;
            message.Spacing(8);
            TextBlock warning;
            warning.Text(plan.GetNamedString(L"kind") == L"url"
                ? L"该记录使用自定义链接协议。继续后，Windows 会把它交给已注册的外部应用。"
                : L"该富文本记录会作为本地 HTML 临时文件交给默认浏览器。请仅打开你信任的内容。");
            warning.TextWrapping(TextWrapping::Wrap);
            TextBlock target;
            target.Text(plan.GetNamedString(L"target"));
            target.FontFamily(FontFamily{ L"Consolas" });
            target.TextWrapping(TextWrapping::Wrap);
            target.IsTextSelectionEnabled(true);
            message.Children().Append(warning);
            message.Children().Append(target);
            confirmation.Content(message);

            m_suspendLifecycle = true;
            confirmation.PrimaryButtonClick([this, plan](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
                LaunchOpenPlan(plan);
            });
            confirmation.Closed([this](auto const&, auto const&)
            {
                m_suspendLifecycle = false;
            });
            (void)confirmation.ShowAsync();
        }
        catch (winrt::hresult_error const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(L"打开内容失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_suspendLifecycle = false;
            SetStatus(StatusMessage(
                L"打开内容失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::LaunchOpenPlan(JsonObject const& plan)
    {
        try
        {
            auto const target = plan.GetNamedString(L"target");
            auto const result = reinterpret_cast<std::intptr_t>(ShellExecuteW(
                GetWindowHandle(),
                L"open",
                target.c_str(),
                nullptr,
                nullptr,
                SW_SHOWNORMAL));
            if (result <= 32)
            {
                throw std::runtime_error(
                    "ShellExecuteW failed with code " + std::to_string(result));
            }

            SetStatus(plan.GetNamedBoolean(L"temporary")
                ? L"已创建受控临时文件，并交给系统默认应用打开。"
                : L"已交给 Windows 默认应用打开。");
        }
        catch (winrt::hresult_error const& error)
        {
            SetStatus(StatusMessage(L"启动默认应用失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"启动默认应用失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::ApplyAction(std::int64_t entryId, std::string_view action)
    {
        auto const isPaste = action == "paste-plain" || action == "paste-rich";
        if (isPaste)
        {
            PreparePasteTarget();
        }

        try
        {
            auto const value = m_core->ApplyAction(entryId, action);
            auto const mutation = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            RefreshItems();

            std::wstringstream status;
            status << ActionStatus(action).c_str()
                   << L" · 第 "
                   << static_cast<std::uint64_t>(mutation.GetNamedNumber(L"generation"));
            auto const replacement = mutation.GetNamedValue(L"replacement_id");
            if (replacement.ValueType() == JsonValueType::Number)
            {
                status << L" 代 · 替换记录 ID "
                       << static_cast<std::int64_t>(replacement.GetNumber());
            }
            else
            {
                status << L" 代";
            }
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"操作失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }

        m_suspendLifecycle = false;
    }

    void MainWindow::PasteTransientText(winrt::hstring const& text)
    {
        PreparePasteTarget();
        try
        {
            m_core->PasteText(winrt::to_string(text));
            std::wstring status{ L"已粘贴表情：" };
            status.append(text.c_str(), text.size());
            SetStatus(winrt::hstring{ status });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"粘贴表情失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
        m_suspendLifecycle = false;
    }

    void MainWindow::PasteFavoriteImage(
        winrt::hstring const& path,
        winrt::hstring const& fileName)
    {
        PreparePasteTarget();
        try
        {
            m_core->PasteEmojiFavorite(winrt::to_string(path));
            std::wstring status{ L"已粘贴图片表情：" };
            status.append(fileName.c_str(), fileName.size());
            SetStatus(winrt::hstring{ status });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"粘贴图片表情失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
        m_suspendLifecycle = false;
    }

    void MainWindow::SetStatus(winrt::hstring const& message)
    {
        StatusText().Text(message);
    }

    void MainWindow::WriteReadyMarker()
    {
        if (m_readyMarkerWritten)
        {
            return;
        }

        wchar_t path[32768]{};
        auto const length = GetEnvironmentVariableW(
            L"TIEZ_WINUI_READY_FILE",
            path,
            static_cast<DWORD>(std::size(path)));
        if (length > 0 && length < std::size(path))
        {
            std::ofstream marker{ std::filesystem::path{ path }, std::ios::trunc };
            marker << "pid=" << GetCurrentProcessId() << '\n';
            marker << "ready_tick_ms=" << GetTickCount64() << '\n';
            marker << "abi_version=" << m_core->AbiVersion() << '\n';
            marker << "start_hidden=" << (m_startHidden ? "true" : "false") << '\n';
        }

        m_readyMarkerWritten = true;
    }

    HWND MainWindow::GetWindowHandle()
    {
        if (m_hwnd == nullptr)
        {
            Window window = *this;
            winrt::check_hresult(window.as<::IWindowNative>()->get_WindowHandle(&m_hwnd));
        }
        return m_hwnd;
    }

    void MainWindow::SetInitialWindowSize()
    {
        auto const hwnd = GetWindowHandle();
        auto const dpiScale = static_cast<float>(GetDpiForWindow(hwnd)) / 96.0F;
        auto const width = static_cast<int>(900 * dpiScale);
        auto const height = static_cast<int>(760 * dpiScale);
        SetWindowPos(hwnd, nullptr, 0, 0, width, height, SWP_NOMOVE | SWP_NOZORDER);
    }

    void MainWindow::SetupLifecycle()
    {
        Activated([this](auto const&, WindowActivatedEventArgs const& args)
        {
            if (m_suspendLifecycle)
            {
                return;
            }
            if (args.WindowActivationState() == WindowActivationState::Deactivated
                && !m_pinned
                && IsWindowVisible(GetWindowHandle()))
            {
                HideMainWindow();
            }
        });
        Closed([this](auto const&, auto const&)
        {
            TeardownLifecycle();
        });

        m_hotkeyHwnd = CreateWindowExW(
            0,
            L"STATIC",
            L"TiezWinUIProbeHotkey",
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            nullptr,
            GetModuleHandleW(nullptr),
            nullptr);
        if (m_hotkeyHwnd == nullptr)
        {
            SetStatus(L"无法创建全局快捷键消息窗口。");
            return;
        }

        SetWindowSubclass(
            m_hotkeyHwnd,
            HotkeySubclassProc,
            kMessageWindowSubclassId,
            reinterpret_cast<DWORD_PTR>(this));
        SetWindowSubclass(
            GetWindowHandle(),
            HotkeySubclassProc,
            kMainWindowSubclassId,
            reinterpret_cast<DWORD_PTR>(this));
        ApplyToggleHotkey(L"Alt+C");
        SetupTrayIcon();
    }

    bool MainWindow::ApplyToggleHotkey(winrt::hstring const& configuredHotkey)
    {
        if (m_hotkeyHwnd == nullptr)
        {
            HotkeyText().Text(L"呼出快捷键：消息窗口不可用");
            return false;
        }

        auto const display = TrimHotkeyText(configuredHotkey.c_str());
        auto const normalized = UpperAscii(display);
        auto const useMouseMiddle = normalized == L"MOUSEMIDDLE" || normalized == L"MBUTTON";
        auto const parsed = useMouseMiddle
            ? std::optional<HotkeySpec>{}
            : ParseHotkey(configuredHotkey.c_str());
        if (!useMouseMiddle && !parsed)
        {
            std::wstring message{ L"快捷键格式无效：" };
            message.append(configuredHotkey.c_str(), configuredHotkey.size());
            if (m_hotkeyRegistered)
            {
                message.append(L"；已继续使用 ");
                message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
                HotkeyText().Text(HotkeyLabel(m_registeredHotkey));
            }
            else
            {
                message.append(L"；可通过系统托盘显示 TieZ。");
                HotkeyText().Text(L"呼出快捷键：配置格式无效");
            }
            SetStatus(winrt::hstring{ message });
            return false;
        }

        if (useMouseMiddle && m_hotkeyRegistered && m_mouseHotkeyHook != nullptr)
        {
            m_registeredHotkey = winrt::hstring{ display };
            HotkeyText().Text(HotkeyLabel(m_registeredHotkey));
            return true;
        }

        if (!useMouseMiddle && parsed->virtualKey == 0)
        {
            if (m_hotkeyRegistered)
            {
                auto const removed = m_mouseHotkeyHook != nullptr
                    ? UnhookWindowsHookEx(m_mouseHotkeyHook)
                    : UnregisterHotKey(m_hotkeyHwnd, kToggleHotkeyId);
                if (!removed)
                {
                    SetStatus(L"无法停用全局呼出快捷键；原快捷键保持不变。");
                    return false;
                }
            }
            g_mouseMiddleTargetHwnd.store(nullptr, std::memory_order_release);
            m_mouseHotkeyHook = nullptr;
            m_hotkeyRegistered = false;
            m_hotkeyModifiers = 0;
            m_hotkeyVirtualKey = 0;
            m_registeredHotkey = L"";
            HotkeyText().Text(L"呼出快捷键：未设置（可使用系统托盘）");
            return true;
        }

        if (!useMouseMiddle
            && m_hotkeyRegistered
            && m_mouseHotkeyHook == nullptr
            && m_hotkeyModifiers == parsed->modifiers
            && m_hotkeyVirtualKey == parsed->virtualKey)
        {
            m_registeredHotkey = winrt::hstring{ parsed->display };
            HotkeyText().Text(HotkeyLabel(m_registeredHotkey));
            return true;
        }

        auto const previousRegistered = m_hotkeyRegistered;
        auto const previousModifiers = m_hotkeyModifiers;
        auto const previousVirtualKey = m_hotkeyVirtualKey;
        auto const previousDisplay = m_registeredHotkey;
        auto const previousWasMouseMiddle = m_mouseHotkeyHook != nullptr;
        if (previousRegistered)
        {
            auto const removed = previousWasMouseMiddle
                ? UnhookWindowsHookEx(m_mouseHotkeyHook)
                : UnregisterHotKey(m_hotkeyHwnd, kToggleHotkeyId);
            if (!removed)
            {
                SetStatus(L"无法更新全局呼出快捷键；原快捷键保持不变。");
                return false;
            }
        }

        g_mouseMiddleTargetHwnd.store(nullptr, std::memory_order_release);
        m_mouseHotkeyHook = nullptr;
        m_hotkeyRegistered = false;
        auto registered = false;
        if (useMouseMiddle)
        {
            g_mouseMiddleTargetHwnd.store(m_hotkeyHwnd, std::memory_order_release);
            m_mouseHotkeyHook = SetWindowsHookExW(
                WH_MOUSE_LL,
                MouseMiddleHookProc,
                GetModuleHandleW(nullptr),
                0);
            registered = m_mouseHotkeyHook != nullptr;
            if (!registered)
            {
                g_mouseMiddleTargetHwnd.store(nullptr, std::memory_order_release);
            }
        }
        else
        {
            registered = RegisterHotKey(
                m_hotkeyHwnd,
                kToggleHotkeyId,
                parsed->modifiers,
                parsed->virtualKey);
        }
        if (registered)
        {
            m_hotkeyRegistered = true;
            m_hotkeyModifiers = useMouseMiddle ? 0 : parsed->modifiers;
            m_hotkeyVirtualKey = useMouseMiddle ? 0 : parsed->virtualKey;
            m_registeredHotkey = winrt::hstring{ display };
            HotkeyText().Text(HotkeyLabel(m_registeredHotkey));
            return true;
        }

        auto restored = false;
        if (previousRegistered)
        {
            if (previousWasMouseMiddle)
            {
                g_mouseMiddleTargetHwnd.store(m_hotkeyHwnd, std::memory_order_release);
                m_mouseHotkeyHook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    MouseMiddleHookProc,
                    GetModuleHandleW(nullptr),
                    0);
                restored = m_mouseHotkeyHook != nullptr;
                if (!restored)
                {
                    g_mouseMiddleTargetHwnd.store(nullptr, std::memory_order_release);
                }
            }
            else
            {
                restored = RegisterHotKey(
                    m_hotkeyHwnd,
                    kToggleHotkeyId,
                    previousModifiers,
                    previousVirtualKey);
            }
        }
        m_hotkeyRegistered = restored;
        m_hotkeyModifiers = restored && !previousWasMouseMiddle ? previousModifiers : 0;
        m_hotkeyVirtualKey = restored && !previousWasMouseMiddle ? previousVirtualKey : 0;
        m_registeredHotkey = restored ? previousDisplay : winrt::hstring{};

        std::wstring message{ L"无法启用全局呼出方式 " };
        message.append(display);
        if (restored)
        {
            message.append(L"；已继续使用 ");
            message.append(previousDisplay.c_str(), previousDisplay.size());
            HotkeyText().Text(HotkeyLabel(previousDisplay));
        }
        else
        {
            message.append(L"；可通过系统托盘显示 TieZ。");
            HotkeyText().Text(L"呼出快捷键：不可用（可使用系统托盘）");
        }
        SetStatus(winrt::hstring{ message });
        return false;
    }

    void MainWindow::SaveToggleHotkey()
    {
        if (!m_core || m_settingsReadOnly || !m_hotkeyEditor)
        {
            auto const message = m_settingsReadOnly
                ? winrt::hstring{ L"当前数据库以只读方式打开，无法修改呼出快捷键。" }
                : winrt::hstring{ L"Rust 核心尚未就绪，无法修改呼出快捷键。" };
            if (m_hotkeySettingsStatus)
            {
                m_hotkeySettingsStatus.Text(message);
            }
            SetStatus(message);
            return;
        }
        if (ReadEnvironmentText(L"TIEZ_WINUI_HOTKEY"))
        {
            auto const message = winrt::hstring{
                L"诊断环境变量正在覆盖呼出快捷键；关闭覆盖后才能保存。" };
            m_hotkeySettingsStatus.Text(message);
            SetStatus(message);
            return;
        }

        auto const candidateText = TrimHotkeyText(m_hotkeyEditor.Text().c_str());
        auto const candidate = winrt::hstring{ candidateText };
        auto const previousConfigured = m_configuredHotkey;
        auto const previousRegistered = m_hotkeyRegistered;
        auto const previousRegisteredHotkey = m_registeredHotkey;
        if (!ApplyToggleHotkey(candidate))
        {
            m_hotkeySettingsStatus.Text(
                candidate.empty()
                ? L"无法停用当前呼出快捷键，原快捷键保持不变。"
                : L"此快捷键格式无效、无法启用或已被占用，原快捷键保持不变。");
            return;
        }

        try
        {
            (void)m_core->UpdateSetting("app.hotkey", winrt::to_string(candidate));
            m_configuredHotkey = candidate;
            m_hotkeyEditor.Text(candidate);
            if (candidate.empty())
            {
                m_hotkeySettingsStatus.Text(L"呼出快捷键已停用，仍可通过系统托盘打开 TieZ。");
                SetStatus(L"全局呼出快捷键已停用。系统托盘仍可使用。");
            }
            else
            {
                std::wstring message{ L"已保存并启用：" };
                message.append(candidate.c_str(), candidate.size());
                m_hotkeySettingsStatus.Text(winrt::hstring{ message });
                message.append(L"。");
                SetStatus(winrt::hstring{ message });
            }
        }
        catch (std::exception const& error)
        {
            auto const rollback = previousRegistered
                ? previousRegisteredHotkey
                : winrt::hstring{};
            auto const restored = ApplyToggleHotkey(rollback);
            m_configuredHotkey = previousConfigured;
            m_hotkeyEditor.Text(previousConfigured);

            auto message = StatusMessage(
                L"保存呼出快捷键失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            std::wstring detail{ message.c_str(), message.size() };
            detail.append(restored
                ? L"；已恢复原快捷键。"
                : L"；无法恢复原快捷键，请使用系统托盘。");
            auto const finalMessage = winrt::hstring{ detail };
            m_hotkeySettingsStatus.Text(finalMessage);
            SetStatus(finalMessage);
        }
    }

    void MainWindow::TeardownLifecycle()
    {
        RemoveTrayIcon();
        auto const keyboardHotkeyRegistered = m_hotkeyRegistered && m_mouseHotkeyHook == nullptr;
        g_mouseMiddleTargetHwnd.store(nullptr, std::memory_order_release);
        if (m_mouseHotkeyHook != nullptr)
        {
            UnhookWindowsHookEx(m_mouseHotkeyHook);
            m_mouseHotkeyHook = nullptr;
        }
        if (m_hwnd != nullptr && IsWindow(m_hwnd))
        {
            RemoveWindowSubclass(m_hwnd, HotkeySubclassProc, kMainWindowSubclassId);
        }
        if (m_hotkeyHwnd != nullptr)
        {
            if (keyboardHotkeyRegistered)
            {
                UnregisterHotKey(m_hotkeyHwnd, kToggleHotkeyId);
            }
            m_hotkeyRegistered = false;
            RemoveWindowSubclass(
                m_hotkeyHwnd,
                HotkeySubclassProc,
                kMessageWindowSubclassId);
            DestroyWindow(m_hotkeyHwnd);
            m_hotkeyHwnd = nullptr;
        }
    }

    void MainWindow::SetupTrayIcon()
    {
        m_taskbarCreatedMessage = RegisterWindowMessageW(L"TaskbarCreated");
        auto const loadedIcon = LoadImageW(
            GetModuleHandleW(nullptr),
            MAKEINTRESOURCEW(kAppIconResourceId),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR);
        m_trayIcon = static_cast<HICON>(loadedIcon);
        if (m_trayIcon == nullptr)
        {
            m_trayIcon = CopyIcon(LoadIconW(nullptr, IDI_APPLICATION));
        }
        if (m_trayVisible)
        {
            AddTrayIcon();
        }
    }

    void MainWindow::AddTrayIcon()
    {
        if (m_trayAdded)
        {
            return;
        }
        if (m_hotkeyHwnd == nullptr || m_trayIcon == nullptr)
        {
            std::wstring message{ L"无法创建系统托盘图标" };
            if (m_hotkeyRegistered)
            {
                message.append(L"；仍可使用 ");
                message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
                message.append(L" 显示 TieZ。");
            }
            else
            {
                message.append(L"，且当前没有可用的全局呼出快捷键。");
            }
            SetStatus(winrt::hstring{ message });
            return;
        }

        NOTIFYICONDATAW data{};
        data.cbSize = sizeof(data);
        data.hWnd = m_hotkeyHwnd;
        data.uID = kTrayIconId;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = kTrayCallbackMessage;
        data.hIcon = m_trayIcon;
        wcscpy_s(data.szTip, L"TieZ 剪贴板");
        if (!Shell_NotifyIconW(NIM_ADD, &data))
        {
            m_trayAdded = false;
            std::wstring message{ L"无法创建系统托盘图标" };
            if (m_hotkeyRegistered)
            {
                message.append(L"；仍可使用 ");
                message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
                message.append(L" 显示 TieZ。");
            }
            else
            {
                message.append(L"，且当前没有可用的全局呼出快捷键。");
            }
            SetStatus(winrt::hstring{ message });
            return;
        }

        data.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &data);
        m_trayAdded = true;
    }

    void MainWindow::RemoveTrayIcon()
    {
        if (m_trayAdded && m_hotkeyHwnd != nullptr)
        {
            NOTIFYICONDATAW data{};
            data.cbSize = sizeof(data);
            data.hWnd = m_hotkeyHwnd;
            data.uID = kTrayIconId;
            Shell_NotifyIconW(NIM_DELETE, &data);
            m_trayAdded = false;
        }
        if (m_trayIcon != nullptr)
        {
            DestroyIcon(m_trayIcon);
            m_trayIcon = nullptr;
        }
    }

    void MainWindow::SetTrayVisible(bool visible)
    {
        m_trayVisible = visible;
        if (visible)
        {
            AddTrayIcon();
            return;
        }

        if (m_trayAdded && m_hotkeyHwnd != nullptr)
        {
            NOTIFYICONDATAW data{};
            data.cbSize = sizeof(data);
            data.hWnd = m_hotkeyHwnd;
            data.uID = kTrayIconId;
            Shell_NotifyIconW(NIM_DELETE, &data);
            m_trayAdded = false;
        }
    }

    void MainWindow::ShowTrayMenu()
    {
        auto const menu = CreatePopupMenu();
        if (menu == nullptr)
        {
            return;
        }
        AppendMenuW(menu, MF_STRING, kTrayShowCommand, L"显示主界面");
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, kTrayExitCommand, L"退出 TieZ");

        POINT position{};
        GetCursorPos(&position);
        m_suspendLifecycle = true;
        SetForegroundWindow(m_hotkeyHwnd);
        auto const command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            position.x,
            position.y,
            0,
            m_hotkeyHwnd,
            nullptr);
        DestroyMenu(menu);
        PostMessageW(m_hotkeyHwnd, WM_NULL, 0, 0);
        m_suspendLifecycle = false;

        if (command == kTrayShowCommand)
        {
            ShowMainWindow(false);
        }
        else if (command == kTrayExitCommand)
        {
            RequestExit();
        }
    }

    void MainWindow::RequestExit()
    {
        if (m_exitRequested)
        {
            return;
        }
        m_exitRequested = true;
        m_suspendLifecycle = true;
        RemoveTrayIcon();
        Close();
    }

    bool MainWindow::OnNativeMessage(
        HWND hwnd,
        UINT message,
        WPARAM wParam,
        LPARAM lParam)
    {
        if (hwnd == m_hotkeyHwnd && message == kMouseMiddleHotkeyMessage)
        {
            OnToggleHotkey();
            return true;
        }
        if (hwnd == m_hotkeyHwnd && message == WM_HOTKEY && wParam == kToggleHotkeyId)
        {
            OnToggleHotkey();
            return true;
        }
        if (hwnd == m_hotkeyHwnd
            && m_taskbarCreatedMessage != 0
            && message == m_taskbarCreatedMessage)
        {
            m_trayAdded = false;
            if (m_trayVisible)
            {
                AddTrayIcon();
            }
            return true;
        }
        if (hwnd == m_hotkeyHwnd && message == kTrayCallbackMessage)
        {
            auto const notification = LOWORD(lParam);
            if (notification == NIN_SELECT
                || notification == NIN_KEYSELECT
                || notification == WM_LBUTTONUP
                || notification == WM_LBUTTONDBLCLK)
            {
                ShowMainWindow(false);
            }
            else if (notification == WM_CONTEXTMENU || notification == WM_RBUTTONUP)
            {
                ShowTrayMenu();
            }
            return true;
        }
        if (hwnd == GetWindowHandle() && message == WM_CLOSE && !m_exitRequested)
        {
            HideMainWindow();
            return true;
        }
        return false;
    }

    void MainWindow::HideMainWindow()
    {
        HideHoverPreview();
        ShowWindow(GetWindowHandle(), SW_HIDE);
        if (m_hotkeyRegistered)
        {
            std::wstring message{ L"窗口已隐藏，使用 " };
            message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
            message.append(L" 或点击系统托盘图标可重新显示。");
            SetStatus(winrt::hstring{ message });
        }
        else
        {
            SetStatus(L"窗口已隐藏，可点击系统托盘图标重新显示。");
        }
    }

    void MainWindow::ShowMainWindow(bool captureForeground)
    {
        if (captureForeground)
        {
            auto const foreground = GetForegroundWindow();
            if (foreground != nullptr && foreground != GetWindowHandle())
            {
                m_lastHwnd = foreground;
            }
        }
        ShowWindow(GetWindowHandle(), SW_SHOW);
        Activate();
        SearchBox().Focus(FocusState::Programmatic);
        if (captureForeground)
        {
            std::wstring message{ L"已通过 " };
            message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
            message.append(L" 显示窗口，并记录粘贴目标窗口。");
            SetStatus(winrt::hstring{ message });
        }
        else
        {
            SetStatus(L"已通过系统托盘显示主界面。");
        }
    }

    void MainWindow::PreparePasteTarget()
    {
        m_suspendLifecycle = true;
        if (!m_pinned)
        {
            ShowWindow(GetWindowHandle(), SW_HIDE);
        }
        if (m_lastHwnd != nullptr && m_lastHwnd != GetWindowHandle())
        {
            SetForegroundWindow(m_lastHwnd);
            Sleep(50);
        }
    }

    bool MainWindow::HandleNavigationKey(VirtualKey key)
    {
        if (key == VirtualKey::Escape)
        {
            HideMainWindow();
            return true;
        }
        if (TagsTextBox().FocusState() != FocusState::Unfocused)
        {
            return false;
        }
        if (key == VirtualKey::Down)
        {
            MoveSelection(1);
            return true;
        }
        if (key == VirtualKey::Up)
        {
            MoveSelection(-1);
            return true;
        }
        if (key == VirtualKey::Enter)
        {
            if (m_imeComposing || m_ignoreNextEnter || (GetKeyState(VK_PROCESSKEY) & 0x8000))
            {
                m_ignoreNextEnter = false;
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                auto const rich = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
                ApplyAction(
                    m_entryIds[static_cast<std::size_t>(m_selectedIndex)],
                    rich ? "paste-rich" : "paste-plain");
            }
            return true;
        }
        if (key == VirtualKey::Delete)
        {
            if (SearchBoxHasFocus())
            {
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                ApplyAction(m_entryIds[static_cast<std::size_t>(m_selectedIndex)], "delete");
            }
            return true;
        }
        if (key == VirtualKey::C && (GetKeyState(VK_CONTROL) & 0x8000))
        {
            if (SearchBoxHasFocus() && !SearchBox().SelectedText().empty())
            {
                return false;
            }
            if (m_selectedIndex >= 0
                && m_selectedIndex < static_cast<int>(m_entryIds.size()))
            {
                ApplyAction(m_entryIds[static_cast<std::size_t>(m_selectedIndex)], "copy-plain");
            }
            return true;
        }
        return false;
    }

    bool MainWindow::SearchBoxHasFocus()
    {
        return SearchBox().FocusState() != FocusState::Unfocused;
    }

    void MainWindow::AttachCardCommands(
        Border const& card,
        std::int64_t entryId,
        bool readOnly,
        bool isSensitive)
    {
        MenuFlyout flyout;
        flyout.Items().Append(CommandItem(
            L"打开",
            !isSensitive,
            [this, entryId] { OpenEntry(entryId); }));
        flyout.Items().Append(CommandItem(
            L"纯文本粘贴",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "paste-plain"); }));
        flyout.Items().Append(CommandItem(
            L"富文本粘贴",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "paste-rich"); }));
        flyout.Items().Append(CommandItem(
            L"复制",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "copy-plain"); }));
        flyout.Items().Append(CommandItem(
            L"置顶 / 取消置顶",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "pin"); }));
        flyout.Items().Append(CommandItem(
            L"删除",
            !readOnly,
            [this, entryId] { ApplyAction(entryId, "delete"); }));
        flyout.Items().Append(CommandItem(
            L"查看详情",
            true,
            [this, entryId] { SelectEntry(entryId); }));
        card.ContextFlyout(flyout);
    }

    void MainWindow::AttachPinnedReorder(
        Border const& card,
        std::int64_t entryId,
        bool enabled)
    {
        card.CanDrag(enabled);
        card.AllowDrop(enabled);
        if (!enabled)
        {
            return;
        }

        AutomationProperties::SetHelpText(
            card,
            L"可拖动调整置顶顺序，也可使用上移和下移按钮。");
        card.DragStarting([this, entryId](auto const&, DragStartingEventArgs const& args)
        {
            m_draggedPinnedId = entryId;
            args.Data().SetText(winrt::to_hstring(entryId));
            args.Data().RequestedOperation(DataPackageOperation::Move);
        });
        card.DragOver([this, entryId](auto const&, DragEventArgs const& args)
        {
            if (m_draggedPinnedId && *m_draggedPinnedId != entryId)
            {
                args.AcceptedOperation(DataPackageOperation::Move);
                args.Handled(true);
            }
        });
        card.Drop([this, entryId](IInspectable const& sender, DragEventArgs const& args)
        {
            if (m_draggedPinnedId && *m_draggedPinnedId != entryId)
            {
                auto const targetCard = sender.as<Border>();
                auto const position = args.GetPosition(targetCard);
                auto const afterTarget = position.Y > targetCard.ActualHeight() / 2.0;
                auto const sourceId = *m_draggedPinnedId;
                m_draggedPinnedId.reset();
                DropPinnedEntry(sourceId, entryId, afterTarget);
                args.Handled(true);
            }
        });
        card.DropCompleted([this](auto const&, DropCompletedEventArgs const&)
        {
            m_draggedPinnedId.reset();
        });
    }

    void MainWindow::MoveSelection(int delta)
    {
        if (m_entryIds.empty())
        {
            return;
        }
        auto const count = static_cast<int>(m_entryIds.size());
        if (m_selectedIndex < 0)
        {
            m_selectedIndex = delta > 0 ? 0 : count - 1;
        }
        else
        {
            m_selectedIndex = (m_selectedIndex + delta + count) % count;
        }
        UpdateSelectionVisuals();
        ShowContent(m_entryIds[static_cast<std::size_t>(m_selectedIndex)]);
    }

    void MainWindow::MovePinnedEntry(std::int64_t entryId, int delta)
    {
        if (!m_canReorderPinned || delta == 0)
        {
            return;
        }
        auto const position = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), entryId);
        if (position == m_pinnedIds.end())
        {
            return;
        }
        auto const index = static_cast<std::ptrdiff_t>(
            std::distance(m_pinnedIds.begin(), position));
        auto const target = index + delta;
        if (target < 0 || target >= static_cast<std::ptrdiff_t>(m_pinnedIds.size()))
        {
            return;
        }
        std::iter_swap(
            m_pinnedIds.begin() + index,
            m_pinnedIds.begin() + target);
        PersistPinnedOrder();
    }

    void MainWindow::DropPinnedEntry(
        std::int64_t sourceId,
        std::int64_t targetId,
        bool afterTarget)
    {
        if (!m_canReorderPinned || sourceId == targetId)
        {
            return;
        }
        auto source = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), sourceId);
        auto target = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), targetId);
        if (source == m_pinnedIds.end() || target == m_pinnedIds.end())
        {
            return;
        }

        m_pinnedIds.erase(source);
        target = std::find(m_pinnedIds.begin(), m_pinnedIds.end(), targetId);
        auto insert = target + (afterTarget ? 1 : 0);
        m_pinnedIds.insert(insert, sourceId);
        PersistPinnedOrder();
    }

    void MainWindow::PersistPinnedOrder()
    {
        try
        {
            JsonArray ids;
            for (auto const entryId : m_pinnedIds)
            {
                ids.Append(JsonValue::CreateNumberValue(static_cast<double>(entryId)));
            }
            auto const value = m_core->UpdatePinnedOrder(
                winrt::to_string(ids.Stringify()));
            auto const result = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            RefreshItems();

            std::wstringstream status;
            status << L"置顶顺序已保存 · " << m_pinnedIds.size()
                   << L" 条 · 第 "
                   << static_cast<std::uint64_t>(result.GetNamedNumber(L"generation"))
                   << L" 代";
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            RefreshItems();
            SetStatus(StatusMessage(
                L"保存置顶顺序失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::UpdateSelectionVisuals()
    {
        for (std::size_t index = 0; index < m_cards.size(); ++index)
        {
            auto const selected = static_cast<int>(index) == m_selectedIndex;
            m_cards[index].BorderThickness(ThicknessHelper::FromUniformLength(selected ? 2.0 : 1.0));
            if (selected)
            {
                m_cards[index].BorderBrush(SolidColorBrush{ Windows::UI::Color{ 255, 0, 120, 212 } });
                m_cards[index].StartBringIntoView();
            }
            else
            {
                m_cards[index].BorderBrush(SolidColorBrush{ Windows::UI::Color{ 255, 200, 200, 200 } });
            }
        }
    }

    void MainWindow::SaveSelectedTags()
    {
        if (!m_core || !m_detailsEntryId || m_readOnly)
        {
            SetStatus(m_readOnly
                ? L"当前历史以只读方式打开，无法保存标签。"
                : L"请先选择一条记录再保存标签。");
            return;
        }

        try
        {
            auto const tags = SplitTags(TagsTextBox().Text());
            JsonArray values;
            for (auto const& tag : tags)
            {
                values.Append(JsonValue::CreateStringValue(tag));
            }
            auto const requestedId = *m_detailsEntryId;
            auto const result = m_core->UpdateTags(
                requestedId,
                winrt::to_string(values.Stringify()));
            auto const mutation = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(result));

            auto effectiveId = requestedId;
            auto const replacement = mutation.GetNamedValue(L"replacement_id");
            if (replacement.ValueType() == JsonValueType::Number)
            {
                effectiveId = static_cast<std::int64_t>(replacement.GetNumber());
            }
            m_detailsEntryId = effectiveId;
            RefreshItems();
            m_tagsById[effectiveId] = tags;
            SelectEntry(effectiveId);

            std::wstringstream status;
            status << L"标签已保存";
            if (effectiveId != requestedId)
            {
                status << L" · 会话记录已安全保存为 ID " << effectiveId;
            }
            else
            {
                status << L" · 记录 ID " << effectiveId;
            }
            SetStatus(winrt::hstring{ status.str() });
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"保存标签失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::SelectEntry(std::int64_t entryId)
    {
        for (std::size_t index = 0; index < m_entryIds.size(); ++index)
        {
            if (m_entryIds[index] == entryId)
            {
                m_selectedIndex = static_cast<int>(index);
                UpdateSelectionVisuals();
                ShowContent(entryId);
                return;
            }
        }
        ShowContent(entryId);
    }

    std::string MainWindow::CurrentQuery()
    {
        auto text = winrt::to_string(SearchBox().Text());
        if (m_typeFilter.empty())
        {
            return text;
        }
        if (text.empty())
        {
            return "type:" + m_typeFilter;
        }
        return "type:" + m_typeFilter + " " + text;
    }

    void MainWindow::SetTypeFilter(std::string filter)
    {
        m_typeFilter = std::move(filter);
        TypeAllButton().IsChecked(m_typeFilter.empty());
        TypeTextButton().IsChecked(m_typeFilter == "text");
        TypeImageButton().IsChecked(m_typeFilter == "image");
        TypeUrlButton().IsChecked(m_typeFilter == "url");
        TypeCodeButton().IsChecked(m_typeFilter == "code");
        TypeFilesButton().IsChecked(m_typeFilter == "file");
        RefreshItems();
    }

    void MainWindow::SetBackupBusy(bool busy, winrt::hstring const& message)
    {
        m_backupBusy = busy;
        if (m_exportBackupButton)
        {
            m_exportBackupButton.IsEnabled(m_productionData && !busy);
        }
        if (m_restoreBackupButton)
        {
            m_restoreBackupButton.IsEnabled(
                m_productionData && !m_settingsReadOnly && !busy);
        }
        if (m_backupProgress)
        {
            m_backupProgress.IsActive(busy);
            m_backupProgress.Visibility(busy ? Visibility::Visible : Visibility::Collapsed);
        }
        if (m_backupStatus)
        {
            m_backupStatus.Text(message);
        }
    }

    winrt::fire_and_forget MainWindow::ExportBackupAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || m_backupBusy || !m_productionData)
        {
            co_return;
        }

        std::optional<std::filesystem::path> destination;
        try
        {
            destination = SelectBackupPath(GetWindowHandle(), true);
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法选择备份位置：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"无法选择备份位置：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        if (!destination)
        {
            co_return;
        }

        auto const path = winrt::to_string(winrt::hstring{ destination->wstring() });
        auto const uiThread = winrt::apartment_context{};
        SetBackupBusy(true, L"正在创建一致性快照并校验备份，请勿退出 TieZ……");
        std::string response;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            response = m_core->CreateBackup(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"导出备份失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            auto const information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            auto const message = BackupSummary(information, L"备份已导出：");
            SetBackupBusy(false, message);
            SetStatus(L"备份已安全导出。原始数据未被修改。");
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取备份结果：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
        }
    }

    winrt::fire_and_forget MainWindow::RestoreBackupAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || m_backupBusy || !m_productionData || m_settingsReadOnly)
        {
            co_return;
        }

        std::optional<std::filesystem::path> source;
        try
        {
            source = SelectBackupPath(GetWindowHandle(), false);
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法选择备份文件：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"无法选择备份文件：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }
        if (!source)
        {
            co_return;
        }

        auto const path = winrt::to_string(winrt::hstring{ source->wstring() });
        auto const uiThread = winrt::apartment_context{};
        SetBackupBusy(true, L"正在检查备份结构、数据库和全部 SHA-256 校验值……");
        std::string inspectionResponse;
        std::string failure;
        co_await winrt::resume_background();
        try
        {
            inspectionResponse = m_core->InspectBackup(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"备份校验失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        JsonObject information;
        try
        {
            information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(inspectionResponse));
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取备份信息：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        auto summary = std::wstring{ BackupSummary(information, L"已验证备份：").c_str() };
        summary.append(
            L"\n\n继续后将把备份复制到 TieZ 的安全待恢复位置。当前数据不会立即改变；下次启动会在打开数据库前恢复，并保留七天回滚副本。是否继续？");
        if (MessageBoxW(
            GetWindowHandle(),
            summary.c_str(),
            L"确认安排恢复",
            MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) != IDYES)
        {
            SetBackupBusy(false, L"已取消恢复，当前数据未改变。");
            co_return;
        }

        SetBackupBusy(true, L"正在复制并再次校验待恢复备份……");
        std::string scheduleResponse;
        failure.clear();
        co_await winrt::resume_background();
        try
        {
            scheduleResponse = m_core->ScheduleRestore(path);
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"安排恢复失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            information = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(scheduleResponse));
            SetBackupBusy(false, BackupSummary(information, L"恢复已安排："));
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取恢复结果：", error.message());
            SetBackupBusy(false, message);
            SetStatus(message);
            co_return;
        }

        SetStatus(L"恢复已安排；退出后再次启动 TieZ 即会安全应用。");
        if (MessageBoxW(
            GetWindowHandle(),
            L"恢复已安排。现在退出 TieZ，可在下次启动时应用备份。\n\n是否立即退出？",
            L"恢复已安排",
            MB_ICONINFORMATION | MB_YESNO | MB_DEFBUTTON1) == IDYES)
        {
            RequestExit();
        }
    }

    void MainWindow::SetCloudSyncBusy(bool busy, winrt::hstring const& message)
    {
        m_cloudSyncBusy = busy;
        auto const editable = !busy && !m_settingsReadOnly;
        if (m_cloudSyncEnabledToggle) m_cloudSyncEnabledToggle.IsEnabled(editable);
        if (m_cloudSyncAutoToggle) m_cloudSyncAutoToggle.IsEnabled(editable);
        if (m_cloudSyncUrlText) m_cloudSyncUrlText.IsEnabled(editable);
        if (m_cloudSyncUsernameText) m_cloudSyncUsernameText.IsEnabled(editable);
        if (m_cloudSyncPasswordBox) m_cloudSyncPasswordBox.IsEnabled(editable);
        if (m_cloudSyncBasePathText) m_cloudSyncBasePathText.IsEnabled(editable);
        if (m_cloudSyncIntervalNumber) m_cloudSyncIntervalNumber.IsEnabled(editable);
        if (m_cloudSyncSnapshotIntervalNumber)
        {
            m_cloudSyncSnapshotIntervalNumber.IsEnabled(editable);
        }
        if (m_cloudSyncTextToggle) m_cloudSyncTextToggle.IsEnabled(editable);
        if (m_cloudSyncImageToggle) m_cloudSyncImageToggle.IsEnabled(editable);
        if (m_cloudSyncFileToggle) m_cloudSyncFileToggle.IsEnabled(editable);
        if (m_cloudSyncEmojiToggle) m_cloudSyncEmojiToggle.IsEnabled(editable);
        if (m_cloudSyncSaveButton) m_cloudSyncSaveButton.IsEnabled(editable);
        if (m_cloudSyncNowButton)
        {
            m_cloudSyncNowButton.IsEnabled(
                editable
                && m_productionData
                && m_cloudSyncEnabledToggle
                && m_cloudSyncEnabledToggle.IsOn());
        }
        if (m_cloudSyncProbeButton) m_cloudSyncProbeButton.IsEnabled(!busy);
        if (m_cloudSyncClearPasswordButton)
        {
            m_cloudSyncClearPasswordButton.IsEnabled(
                editable && m_cloudSyncPasswordConfigured);
        }
        if (m_cloudSyncProgress)
        {
            m_cloudSyncProgress.IsActive(busy);
            m_cloudSyncProgress.Visibility(
                busy ? Visibility::Visible : Visibility::Collapsed);
        }
        if (m_cloudSyncStatus && !message.empty())
        {
            m_cloudSyncStatus.Text(message);
        }
    }

    void MainWindow::LoadCloudSyncSettings()
    {
        if (!m_core || !m_cloudSyncEnabledToggle)
        {
            return;
        }

        auto const value = m_core->CloudSyncSettings();
        auto const root = JsonObject::Parse(
            tiez::probe::RustCoreBridge::Utf8ToHstring(value));
        auto const preferences = root.GetNamedObject(L"content_prefs");
        m_cloudSyncPasswordConfigured = root.GetNamedBoolean(
            L"password_configured",
            false);
        m_cloudSyncEnabledToggle.IsOn(root.GetNamedBoolean(L"enabled", false));
        m_cloudSyncAutoToggle.IsOn(root.GetNamedBoolean(L"auto_sync", true));
        m_cloudSyncUrlText.Text(root.GetNamedString(L"webdav_url", L""));
        m_cloudSyncUsernameText.Text(root.GetNamedString(L"webdav_username", L""));
        m_cloudSyncPasswordBox.Password(L"");
        m_cloudSyncBasePathText.Text(root.GetNamedString(
            L"webdav_base_path",
            L"tiez-sync"));
        m_cloudSyncIntervalNumber.Value(root.GetNamedNumber(L"interval_secs", 120));
        m_cloudSyncSnapshotIntervalNumber.Value(root.GetNamedNumber(
            L"snapshot_interval_min",
            720));
        m_cloudSyncTextToggle.IsOn(preferences.GetNamedBoolean(L"text", true));
        m_cloudSyncImageToggle.IsOn(preferences.GetNamedBoolean(L"image", true));
        m_cloudSyncFileToggle.IsOn(preferences.GetNamedBoolean(L"file_path", true));
        m_cloudSyncEmojiToggle.IsOn(preferences.GetNamedBoolean(L"emoji", true));

        std::wstring status;
        auto const url = m_cloudSyncUrlText.Text();
        if (url.empty())
        {
            status = L"尚未配置 WebDAV 地址。保存密码后，原生界面只会显示“已配置”，不会回读密码。";
        }
        else
        {
            status = m_cloudSyncPasswordConfigured
                ? L"WebDAV 密码已配置且保持只写；"
                : L"尚未保存 WebDAV 密码；";
            status.append(root.GetNamedBoolean(L"secure_transport", false)
                ? L"当前地址使用 HTTPS。"
                : L"当前旧配置不是 HTTPS，原生界面将拒绝保存或连接。");
        }
        if (root.GetNamedBoolean(L"read_only", false))
        {
            status.append(L" 当前数据库为只读，只能测试已保存的配置。");
        }
        SetCloudSyncBusy(m_cloudSyncBusy, winrt::hstring{ status });
    }

    bool MainWindow::SaveCloudSyncSettings(bool clearPassword, bool reloadRunner)
    {
        if (!m_core || m_settingsReadOnly || m_cloudSyncBusy)
        {
            SetStatus(m_settingsReadOnly
                ? L"当前数据库以只读方式打开，无法保存云同步设置。"
                : L"云同步设置当前不可保存。");
            return false;
        }
        if (!std::isfinite(m_cloudSyncIntervalNumber.Value())
            || !std::isfinite(m_cloudSyncSnapshotIntervalNumber.Value()))
        {
            SetCloudSyncBusy(false, L"同步间隔必须是有效数字。");
            return false;
        }

        try
        {
            JsonObject request;
            request.SetNamedValue(
                L"enabled",
                JsonValue::CreateBooleanValue(m_cloudSyncEnabledToggle.IsOn()));
            request.SetNamedValue(
                L"auto_sync",
                JsonValue::CreateBooleanValue(m_cloudSyncAutoToggle.IsOn()));
            request.SetNamedValue(
                L"webdav_url",
                JsonValue::CreateStringValue(m_cloudSyncUrlText.Text()));
            request.SetNamedValue(
                L"webdav_username",
                JsonValue::CreateStringValue(m_cloudSyncUsernameText.Text()));
            request.SetNamedValue(
                L"clear_password",
                JsonValue::CreateBooleanValue(clearPassword));
            request.SetNamedValue(
                L"webdav_base_path",
                JsonValue::CreateStringValue(m_cloudSyncBasePathText.Text()));
            request.SetNamedValue(
                L"interval_secs",
                JsonValue::CreateNumberValue(static_cast<double>(std::llround(
                    m_cloudSyncIntervalNumber.Value()))));
            request.SetNamedValue(
                L"snapshot_interval_min",
                JsonValue::CreateNumberValue(static_cast<double>(std::llround(
                    m_cloudSyncSnapshotIntervalNumber.Value()))));
            auto const password = m_cloudSyncPasswordBox.Password();
            if (!clearPassword && !password.empty())
            {
                request.SetNamedValue(
                    L"webdav_password",
                    JsonValue::CreateStringValue(password));
            }

            JsonObject preferences;
            preferences.SetNamedValue(
                L"text",
                JsonValue::CreateBooleanValue(m_cloudSyncTextToggle.IsOn()));
            preferences.SetNamedValue(
                L"image",
                JsonValue::CreateBooleanValue(m_cloudSyncImageToggle.IsOn()));
            preferences.SetNamedValue(
                L"file_path",
                JsonValue::CreateBooleanValue(m_cloudSyncFileToggle.IsOn()));
            preferences.SetNamedValue(
                L"emoji",
                JsonValue::CreateBooleanValue(m_cloudSyncEmojiToggle.IsOn()));
            request.SetNamedValue(L"content_prefs", preferences);

            auto const response = m_core->UpdateCloudSyncSettings(
                winrt::to_string(request.Stringify()));
            auto const root = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            m_cloudSyncPasswordConfigured = root.GetNamedBoolean(
                L"password_configured",
                false);
            m_cloudSyncPasswordBox.Password(L"");
            if (m_productionData && reloadRunner)
            {
                m_core->StartCloudSync();
            }
            auto const message = clearPassword
                ? L"WebDAV 密码已从 TieZ 设置中清除。"
                : m_cloudSyncPasswordConfigured
                    ? L"云同步设置已保存；密码保持只写，不会显示在界面中。"
                    : L"云同步设置已保存；当前没有已保存的密码。";
            SetCloudSyncBusy(false, message);
            SetStatus(message);
            return true;
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"保存云同步设置失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetCloudSyncBusy(false, message);
            SetStatus(message);
            return false;
        }
    }

    void MainWindow::RequestCloudSyncNow()
    {
        if (!m_core || !m_productionData || m_settingsReadOnly || m_cloudSyncBusy)
        {
            SetStatus(L"当前数据库不能执行云同步。");
            return;
        }
        if (!SaveCloudSyncSettings(false, false))
        {
            return;
        }

        try
        {
            m_core->RequestCloudSync();
            if (m_cloudSyncProgress)
            {
                m_cloudSyncProgress.IsActive(true);
                m_cloudSyncProgress.Visibility(Visibility::Visible);
            }
            if (m_cloudSyncNowButton)
            {
                m_cloudSyncNowButton.IsEnabled(false);
            }
            m_cloudSyncStatus.Text(L"已提交立即同步请求，正在等待后台运行器处理……");
            SetStatus(L"已提交立即同步请求。");
        }
        catch (std::exception const& error)
        {
            auto const message = StatusMessage(
                L"无法启动立即同步：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what()));
            SetCloudSyncBusy(false, message);
            SetStatus(message);
        }
    }

    void MainWindow::UpdateCloudSyncStatus()
    {
        if (!m_core || !m_cloudSyncStatus || m_cloudSyncBusy)
        {
            return;
        }

        try
        {
            auto const value = m_core->CloudSyncStatus();
            auto const root = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const state = root.GetNamedString(L"state", L"unavailable");
            auto const syncing = root.GetNamedBoolean(L"syncing", false);
            auto const automatic = root.GetNamedBoolean(L"automatic", false);
            auto const settingsRevision = static_cast<std::uint64_t>(
                root.GetNamedNumber(L"settings_revision", 0));
            auto const uploaded = static_cast<std::uint64_t>(
                root.GetNamedNumber(L"uploaded_items", 0));
            auto const received = static_cast<std::uint64_t>(
                root.GetNamedNumber(L"received_items", 0));
            if (settingsRevision > m_cloudSyncSettingsRevision)
            {
                m_cloudSyncSettingsRevision = settingsRevision;
                LoadSettings();
            }

            if (m_cloudSyncProgress)
            {
                m_cloudSyncProgress.IsActive(syncing);
                m_cloudSyncProgress.Visibility(
                    syncing ? Visibility::Visible : Visibility::Collapsed);
            }
            if (m_cloudSyncNowButton)
            {
                m_cloudSyncNowButton.IsEnabled(
                    !syncing
                    && m_productionData
                    && !m_settingsReadOnly
                    && m_cloudSyncEnabledToggle
                    && m_cloudSyncEnabledToggle.IsOn());
            }

            std::wstring message;
            if (state == L"syncing")
            {
                message = L"正在通过 WebDAV 同步剪贴板、设置和表情收藏……";
            }
            else if (state == L"idle")
            {
                message = L"同步完成：上传 ";
                message.append(std::to_wstring(uploaded));
                message.append(L" 项，接收 ");
                message.append(std::to_wstring(received));
                message.append(automatic
                    ? L" 项。后台服务会按设置的间隔继续同步。"
                    : L" 项。自动同步已关闭，可继续手动同步。");
            }
            else if (state == L"waiting")
            {
                message = L"云同步已启用，自动同步已关闭；可点击“立即同步”。";
            }
            else if (state == L"disabled")
            {
                message = L"云同步已停用；远端数据不会被读取或写入。";
            }
            else if (state == L"starting")
            {
                message = L"正在启动原生后台同步服务……";
            }
            else if (state == L"read_only")
            {
                message = L"当前数据库为只读，后台同步不会启动。";
            }
            else if (state == L"stopped")
            {
                message = L"原生后台同步服务尚未运行。";
            }
            else if (state == L"error")
            {
                message = L"云同步失败";
                auto const errorValue = root.GetNamedValue(
                    L"last_error",
                    JsonValue::CreateNullValue());
                if (errorValue.ValueType() == JsonValueType::String)
                {
                    message.append(L"：");
                    message.append(errorValue.GetString());
                }
                message.append(automatic
                    ? L"。后台服务会按同步间隔重试。"
                    : L"。请检查设置后重试。");
            }
            else
            {
                message = L"云同步仅在可写的 WinUI 生产数据模式下可用。";
            }
            m_cloudSyncStatus.Text(winrt::hstring{ message });
        }
        catch (std::exception const& error)
        {
            m_cloudSyncStatus.Text(StatusMessage(
                L"无法读取云同步状态：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    winrt::fire_and_forget MainWindow::ProbeCloudSyncAsync()
    {
        auto lifetime = get_strong();
        if (!m_core || m_cloudSyncBusy)
        {
            co_return;
        }
        if (!m_settingsReadOnly && !SaveCloudSyncSettings(false, false))
        {
            co_return;
        }

        SetCloudSyncBusy(true, L"正在以只读 PROPFIND 测试 WebDAV 地址和凭据……");
        auto const uiThread = winrt::apartment_context{};
        auto const restartRunner = m_productionData && !m_settingsReadOnly;
        std::string response;
        std::string failure;
        co_await winrt::resume_background();
        if (restartRunner)
        {
            m_core->StopCloudSync();
        }
        try
        {
            response = m_core->ProbeCloudSync();
        }
        catch (std::exception const& error)
        {
            failure = error.what();
        }
        if (restartRunner)
        {
            try
            {
                m_core->StartCloudSync();
            }
            catch (std::exception const& error)
            {
                if (failure.empty())
                {
                    failure = error.what();
                }
            }
        }
        try
        {
            co_await uiThread;
        }
        catch (...)
        {
            co_return;
        }

        if (!failure.empty())
        {
            auto const message = StatusMessage(
                L"WebDAV 连接测试失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(failure));
            SetCloudSyncBusy(false, message);
            SetStatus(message);
            co_return;
        }

        try
        {
            auto const root = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(response));
            auto const reachable = root.GetNamedBoolean(L"reachable", false);
            auto const statusValue = root.GetNamedValue(L"status_code");
            auto const statusCode = statusValue.ValueType() == JsonValueType::Number
                ? static_cast<int>(statusValue.GetNumber())
                : 0;
            std::wstring message;
            if (reachable)
            {
                message = L"WebDAV 连接成功；地址和凭据可用，测试期间未写入远端数据。";
            }
            else if (statusCode == 401 || statusCode == 403)
            {
                message = L"WebDAV 已响应，但拒绝了当前用户名或密码。";
            }
            else if (statusCode >= 300 && statusCode < 400)
            {
                message = L"WebDAV 返回重定向。为防止凭据跨站发送，请保存重定向后的最终 HTTPS 地址。";
            }
            else
            {
                message = L"WebDAV 已响应，但未通过连通性检查";
                if (statusCode > 0)
                {
                    message.append(L"（HTTP ");
                    message.append(std::to_wstring(statusCode));
                    message.append(L"）");
                }
                message.append(L"。");
            }
            SetCloudSyncBusy(false, winrt::hstring{ message });
            SetStatus(winrt::hstring{ message });
        }
        catch (winrt::hresult_error const& error)
        {
            auto const message = StatusMessage(L"无法读取连接测试结果：", error.message());
            SetCloudSyncBusy(false, message);
            SetStatus(message);
        }
    }

    void MainWindow::SetUpdateBusy(bool busy, winrt::hstring const& message)
    {
        m_updateBusy = busy;
        if (m_checkUpdateButton)
        {
            m_checkUpdateButton.IsEnabled(!busy);
            m_checkUpdateButton.Content(winrt::box_value(
                busy ? L"正在检查……" : L"检查更新"));
        }
        if (m_installUpdateButton)
        {
            m_installUpdateButton.IsEnabled(
                !busy && m_updateAvailable && !m_appInstallerUri.empty());
        }
        if (m_updateProgress)
        {
            m_updateProgress.IsActive(busy);
            m_updateProgress.Visibility(busy ? Visibility::Visible : Visibility::Collapsed);
        }
        if (m_updateStatus && !message.empty())
        {
            m_updateStatus.Text(message);
        }
    }

    winrt::fire_and_forget MainWindow::CheckForUpdatesAsync()
    {
        auto lifetime = get_strong();
        if (m_updateBusy)
        {
            co_return;
        }

        m_updateAvailable = false;
        m_appInstallerUri = L"";
        SetUpdateBusy(true, L"正在通过 Windows App Installer 检查签名更新……");
        try
        {
            auto const current = Package::Current();
            PackageManager manager;
            auto const package = manager.FindPackageForUser(
                L"",
                current.Id().FullName());
            if (!package)
            {
                SetUpdateBusy(false, L"Windows 找不到当前 TieZ 安装包，无法检查更新。");
                co_return;
            }

            auto const installer = package.GetAppInstallerInfo();
            if (!installer)
            {
                SetUpdateBusy(
                    false,
                    L"当前安装未关联 .appinstaller 更新源。请从正式 TieZ-x64.appinstaller 安装后再试。");
                co_return;
            }
            auto const feed = installer.Uri();
            if (!feed || feed.SchemeName() != L"https")
            {
                SetUpdateBusy(false, L"更新源不是受支持的 HTTPS 地址，已拒绝打开。");
                co_return;
            }
            m_appInstallerUri = feed.AbsoluteUri();

            auto const result = co_await package.CheckUpdateAvailabilityAsync();
            switch (result.Availability())
            {
            case PackageUpdateAvailability::Available:
                m_updateAvailable = true;
                SetUpdateBusy(
                    false,
                    L"发现可用更新。点击“安装更新”，由 Windows 校验签名并完成升级。");
                break;
            case PackageUpdateAvailability::Required:
                m_updateAvailable = true;
                SetUpdateBusy(
                    false,
                    L"发现必须安装的更新。请点击“安装更新”继续使用受支持版本。");
                break;
            case PackageUpdateAvailability::NoUpdates:
                SetUpdateBusy(false, L"当前已是最新版本。");
                break;
            case PackageUpdateAvailability::Error:
                SetUpdateBusy(false, L"Windows App Installer 检查更新时返回错误，请稍后重试。");
                break;
            case PackageUpdateAvailability::Unknown:
            default:
                SetUpdateBusy(
                    false,
                    L"当前安装没有可用的更新信息；确认它是通过正式 .appinstaller 安装的版本。");
                break;
            }
        }
        catch (winrt::hresult_error const& error)
        {
            m_updateAvailable = false;
            m_appInstallerUri = L"";
            SetUpdateBusy(false, StatusMessage(L"检查更新失败：", error.message()));
        }
    }

    winrt::fire_and_forget MainWindow::OpenAppInstallerAsync()
    {
        auto lifetime = get_strong();
        if (m_updateBusy || !m_updateAvailable || m_appInstallerUri.empty())
        {
            co_return;
        }

        SetUpdateBusy(true, L"正在打开 Windows App Installer……");
        try
        {
            auto const escapedSource = Windows::Foundation::Uri::EscapeComponent(
                m_appInstallerUri);
            std::wstring activation{ L"ms-appinstaller:?source=" };
            activation.append(escapedSource.c_str(), escapedSource.size());
            auto const launched = co_await Windows::System::Launcher::LaunchUriAsync(
                Windows::Foundation::Uri{ winrt::hstring{ activation } });
            SetUpdateBusy(
                false,
                launched
                    ? L"Windows App Installer 已打开；确认发布者与版本后即可升级。"
                    : L"系统未能打开 App Installer，请检查 Windows 的应用安装程序组件。");
        }
        catch (winrt::hresult_error const& error)
        {
            SetUpdateBusy(false, StatusMessage(L"打开 App Installer 失败：", error.message()));
        }
    }

    void MainWindow::EnsureSettingsDialog()
    {
        if (m_settingsDialog)
        {
            return;
        }

        m_settingsDialog = ContentDialog();
        m_settingsDialog.Title(winrt::box_value(L"TieZ 设置"));
        m_settingsDialog.CloseButtonText(L"完成");
        m_settingsDialog.DefaultButton(ContentDialogButton::Close);

        m_settingsPanel = StackPanel();
        m_settingsPanel.Spacing(14);
        m_settingsPanel.MaxWidth(520);

        TextBlock introduction;
        introduction.Text(L"这些设置直接写入 TieZ 数据库，并立即作用于原生主窗口。敏感密钥不会在此界面读取或显示。");
        introduction.TextWrapping(TextWrapping::Wrap);
        introduction.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(introduction);

        TextBlock appearanceTitle;
        appearanceTitle.Text(L"外观与窗口");
        appearanceTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(appearanceTitle);

        m_colorModeCombo = ComboBox();
        m_colorModeCombo.Header(winrt::box_value(L"界面主题"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"跟随系统"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"浅色"));
        m_colorModeCombo.Items().Append(winrt::box_value(L"深色"));
        AutomationProperties::SetName(m_colorModeCombo, L"界面主题");
        m_settingsPanel.Children().Append(m_colorModeCombo);

        m_compactModeToggle = SettingToggle(
            L"紧凑列表",
            L"减少卡片间距并隐藏卡片按钮；仍可双击、使用键盘或右键菜单操作。");
        m_windowPinnedToggle = SettingToggle(
            L"固定窗口",
            L"让主窗口保持置顶，并在失去焦点时继续显示。");
        m_trayVisibleToggle = SettingToggle(
            L"显示系统托盘图标",
            L"关闭后仍可使用已配置的全局快捷键显示 TieZ。");
        m_autostartToggle = SettingToggle(
            L"开机启动 TieZ",
            L"登录 Windows 后只在系统托盘后台运行，不弹出主窗口。");
        m_autostartStatus = TextBlock();
        m_autostartStatus.Text(L"正在读取 Windows 登录启动状态……");
        m_autostartStatus.TextWrapping(TextWrapping::Wrap);
        m_autostartStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_autostartStatus, L"TieZ 开机启动状态");
        m_settingsPanel.Children().Append(m_compactModeToggle);
        m_settingsPanel.Children().Append(m_windowPinnedToggle);
        m_settingsPanel.Children().Append(m_trayVisibleToggle);
        m_settingsPanel.Children().Append(m_autostartToggle);
        m_settingsPanel.Children().Append(m_autostartStatus);

        TextBlock hotkeyTitle;
        hotkeyTitle.Text(L"呼出快捷键");
        hotkeyTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(hotkeyTitle);

        m_hotkeyEditor = TextBox();
        m_hotkeyEditor.Header(winrt::box_value(L"全局呼出快捷键"));
        m_hotkeyEditor.PlaceholderText(
            L"例如 Alt+C、Ctrl+Shift+F12 或 MouseMiddle；留空停用");
        m_hotkeyEditor.MaxLength(64);
        AutomationProperties::SetName(m_hotkeyEditor, L"全局呼出快捷键");
        AutomationProperties::SetHelpText(
            m_hotkeyEditor,
            L"输入修饰键和按键并用加号连接，或输入 MouseMiddle 使用鼠标中键；支持 Ctrl、Shift、Alt、Win、字母、数字、功能键和常用按键。留空可停用。");
        m_settingsPanel.Children().Append(m_hotkeyEditor);

        m_hotkeyApplyButton = Button();
        m_hotkeyApplyButton.Content(winrt::box_value(L"应用快捷键"));
        AutomationProperties::SetName(m_hotkeyApplyButton, L"应用全局呼出快捷键");
        m_settingsPanel.Children().Append(m_hotkeyApplyButton);

        m_hotkeySettingsStatus = TextBlock();
        m_hotkeySettingsStatus.Text(L"正在读取已保存的呼出快捷键……");
        m_hotkeySettingsStatus.TextWrapping(TextWrapping::Wrap);
        m_hotkeySettingsStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_hotkeySettingsStatus, L"呼出快捷键状态");
        AutomationProperties::SetLiveSetting(
            m_hotkeySettingsStatus,
            Microsoft::UI::Xaml::Automation::Peers::AutomationLiveSetting::Polite);
        m_settingsPanel.Children().Append(m_hotkeySettingsStatus);

        TextBlock historyTitle;
        historyTitle.Text(L"历史与捕获");
        historyTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(historyTitle);

        m_persistentToggle = SettingToggle(
            L"持久保存历史",
            L"关闭时新记录只保存在当前会话，置顶或添加标签后才会写入数据库。");
        m_persistentLimitEnabledToggle = SettingToggle(
            L"限制持久历史数量",
            L"仅清理未置顶、未加标签且未受保护的较旧记录。");
        m_persistentLimitNumber = NumberBox();
        m_persistentLimitNumber.Header(winrt::box_value(L"最多保留记录数"));
        m_persistentLimitNumber.Minimum(0);
        m_persistentLimitNumber.Maximum(100000);
        m_persistentLimitNumber.SmallChange(50);
        m_persistentLimitNumber.SpinButtonPlacementMode(NumberBoxSpinButtonPlacementMode::Inline);
        AutomationProperties::SetName(m_persistentLimitNumber, L"最多保留记录数");
        m_deduplicateToggle = SettingToggle(
            L"自动去重",
            L"忽略与最近记录相同的剪贴板内容。");
        m_captureFilesToggle = SettingToggle(
            L"捕获文件",
            L"记录从资源管理器等应用复制的文件路径。");
        m_captureRichTextToggle = SettingToggle(
            L"捕获富文本",
            L"保留 HTML 富文本；关闭时仍会按纯文本记录。");
        m_privacyProtectionToggle = SettingToggle(
            L"隐私保护",
            L"按现有规则识别敏感内容并加密持久数据。");
        m_settingsPanel.Children().Append(m_persistentToggle);
        m_settingsPanel.Children().Append(m_persistentLimitEnabledToggle);
        m_settingsPanel.Children().Append(m_persistentLimitNumber);
        m_settingsPanel.Children().Append(m_deduplicateToggle);
        m_settingsPanel.Children().Append(m_captureFilesToggle);
        m_settingsPanel.Children().Append(m_captureRichTextToggle);
        m_settingsPanel.Children().Append(m_privacyProtectionToggle);

        TextBlock cloudSyncTitle;
        cloudSyncTitle.Text(L"云同步（WebDAV）");
        cloudSyncTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(cloudSyncTitle);

        TextBlock cloudSyncDescription;
        cloudSyncDescription.Text(
            L"原生界面使用与旧版完全相同的数据库设置键。密码只允许写入或清除，不会从 Rust 边界返回；连接测试只发送 PROPFIND，不创建目录、不上传剪贴板。远程地址必须使用 HTTPS。");
        cloudSyncDescription.TextWrapping(TextWrapping::Wrap);
        cloudSyncDescription.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(cloudSyncDescription);

        m_cloudSyncEnabledToggle = SettingToggle(
            L"启用云同步",
            L"使用原生 Rust 后台服务同步剪贴板、设置和表情收藏，与旧版 WebDAV 数据保持兼容。");
        m_cloudSyncAutoToggle = SettingToggle(
            L"自动同步",
            L"按下面的时间间隔在后台自动同步；关闭后仍可手动立即同步。");
        m_settingsPanel.Children().Append(m_cloudSyncEnabledToggle);
        m_settingsPanel.Children().Append(m_cloudSyncAutoToggle);

        m_cloudSyncUrlText = TextBox();
        m_cloudSyncUrlText.Header(winrt::box_value(L"WebDAV HTTPS 地址"));
        m_cloudSyncUrlText.PlaceholderText(
            L"https://dav.example.com/remote.php/dav/files/用户名");
        AutomationProperties::SetName(m_cloudSyncUrlText, L"WebDAV HTTPS 地址");
        m_settingsPanel.Children().Append(m_cloudSyncUrlText);

        m_cloudSyncUsernameText = TextBox();
        m_cloudSyncUsernameText.Header(winrt::box_value(L"WebDAV 用户名"));
        AutomationProperties::SetName(m_cloudSyncUsernameText, L"WebDAV 用户名");
        m_settingsPanel.Children().Append(m_cloudSyncUsernameText);

        m_cloudSyncPasswordBox = PasswordBox();
        m_cloudSyncPasswordBox.Header(winrt::box_value(L"WebDAV 密码或应用专用密码"));
        m_cloudSyncPasswordBox.PlaceholderText(L"留空表示保留已保存密码");
        m_cloudSyncPasswordBox.PasswordRevealMode(PasswordRevealMode::Peek);
        AutomationProperties::SetName(m_cloudSyncPasswordBox, L"WebDAV 密码");
        m_settingsPanel.Children().Append(m_cloudSyncPasswordBox);

        m_cloudSyncBasePathText = TextBox();
        m_cloudSyncBasePathText.Header(winrt::box_value(L"远端同步目录"));
        m_cloudSyncBasePathText.PlaceholderText(L"tiez-sync");
        AutomationProperties::SetName(m_cloudSyncBasePathText, L"WebDAV 同步目录");
        m_settingsPanel.Children().Append(m_cloudSyncBasePathText);

        m_cloudSyncIntervalNumber = NumberBox();
        m_cloudSyncIntervalNumber.Header(winrt::box_value(L"自动同步间隔（秒）"));
        m_cloudSyncIntervalNumber.Minimum(5);
        m_cloudSyncIntervalNumber.Maximum(3600);
        m_cloudSyncIntervalNumber.SmallChange(5);
        m_cloudSyncIntervalNumber.SpinButtonPlacementMode(
            NumberBoxSpinButtonPlacementMode::Inline);
        AutomationProperties::SetName(m_cloudSyncIntervalNumber, L"自动同步间隔秒数");
        m_settingsPanel.Children().Append(m_cloudSyncIntervalNumber);

        m_cloudSyncSnapshotIntervalNumber = NumberBox();
        m_cloudSyncSnapshotIntervalNumber.Header(winrt::box_value(L"完整快照间隔（分钟）"));
        m_cloudSyncSnapshotIntervalNumber.Minimum(5);
        m_cloudSyncSnapshotIntervalNumber.Maximum(1440);
        m_cloudSyncSnapshotIntervalNumber.SmallChange(5);
        m_cloudSyncSnapshotIntervalNumber.SpinButtonPlacementMode(
            NumberBoxSpinButtonPlacementMode::Inline);
        AutomationProperties::SetName(
            m_cloudSyncSnapshotIntervalNumber,
            L"完整快照间隔分钟数");
        m_settingsPanel.Children().Append(m_cloudSyncSnapshotIntervalNumber);

        TextBlock cloudContentTitle;
        cloudContentTitle.Text(L"同步内容");
        cloudContentTitle.FontWeight(Windows::UI::Text::FontWeights::SemiBold());
        m_settingsPanel.Children().Append(cloudContentTitle);
        m_cloudSyncTextToggle = SettingToggle(L"文本、代码、链接和富文本", L"");
        m_cloudSyncImageToggle = SettingToggle(L"图片", L"");
        m_cloudSyncFileToggle = SettingToggle(L"文件与视频路径", L"");
        m_cloudSyncEmojiToggle = SettingToggle(L"表情收藏", L"");
        m_settingsPanel.Children().Append(m_cloudSyncTextToggle);
        m_settingsPanel.Children().Append(m_cloudSyncImageToggle);
        m_settingsPanel.Children().Append(m_cloudSyncFileToggle);
        m_settingsPanel.Children().Append(m_cloudSyncEmojiToggle);

        StackPanel cloudSyncActions;
        cloudSyncActions.Orientation(Orientation::Horizontal);
        cloudSyncActions.Spacing(8);
        m_cloudSyncSaveButton = Button();
        m_cloudSyncSaveButton.Content(winrt::box_value(L"保存云同步设置"));
        AutomationProperties::SetName(m_cloudSyncSaveButton, L"保存云同步设置");
        m_cloudSyncNowButton = Button();
        m_cloudSyncNowButton.Content(winrt::box_value(L"立即同步"));
        m_cloudSyncNowButton.IsEnabled(false);
        AutomationProperties::SetName(m_cloudSyncNowButton, L"立即执行 WebDAV 云同步");
        m_cloudSyncProbeButton = Button();
        m_cloudSyncProbeButton.Content(winrt::box_value(L"测试连接"));
        AutomationProperties::SetName(m_cloudSyncProbeButton, L"测试 WebDAV 连接");
        m_cloudSyncClearPasswordButton = Button();
        m_cloudSyncClearPasswordButton.Content(winrt::box_value(L"清除密码"));
        AutomationProperties::SetName(m_cloudSyncClearPasswordButton, L"清除 WebDAV 密码");
        m_cloudSyncProgress = ProgressRing();
        m_cloudSyncProgress.Width(22);
        m_cloudSyncProgress.Height(22);
        m_cloudSyncProgress.IsActive(false);
        m_cloudSyncProgress.Visibility(Visibility::Collapsed);
        cloudSyncActions.Children().Append(m_cloudSyncSaveButton);
        cloudSyncActions.Children().Append(m_cloudSyncNowButton);
        cloudSyncActions.Children().Append(m_cloudSyncProbeButton);
        cloudSyncActions.Children().Append(m_cloudSyncClearPasswordButton);
        cloudSyncActions.Children().Append(m_cloudSyncProgress);
        m_settingsPanel.Children().Append(cloudSyncActions);

        m_cloudSyncStatus = TextBlock();
        m_cloudSyncStatus.TextWrapping(TextWrapping::Wrap);
        m_cloudSyncStatus.IsTextSelectionEnabled(true);
        m_cloudSyncStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_cloudSyncStatus, L"云同步状态");
        m_settingsPanel.Children().Append(m_cloudSyncStatus);

        m_cloudSyncSaveButton.Click([this](auto const&, auto const&)
        {
            (void)SaveCloudSyncSettings(false);
        });
        m_cloudSyncNowButton.Click([this](auto const&, auto const&)
        {
            RequestCloudSyncNow();
        });
        m_cloudSyncProbeButton.Click([this](auto const&, auto const&)
        {
            ProbeCloudSyncAsync();
        });
        m_cloudSyncClearPasswordButton.Click([this](auto const&, auto const&)
        {
            if (MessageBoxW(
                GetWindowHandle(),
                L"这会清除 TieZ 数据库中保存的 WebDAV 密码。地址、用户名和其他设置会保留。是否继续？",
                L"确认清除 WebDAV 密码",
                MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) == IDYES)
            {
                (void)SaveCloudSyncSettings(true);
            }
        });

        TextBlock dataSafetyTitle;
        dataSafetyTitle.Text(L"数据安全");
        dataSafetyTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(dataSafetyTitle);

        TextBlock dataSafetyDescription;
        dataSafetyDescription.Text(
            L"备份包含一致的 SQLite 快照、附件和表情收藏，文件本身不会额外加密，请仅保存到可信位置。受保护字段仍绑定当前 Windows 账户的 DPAPI，换账户或设备可能无法解密。恢复前会校验结构、大小和 SHA-256，并保留七天回滚副本。");
        dataSafetyDescription.TextWrapping(TextWrapping::Wrap);
        dataSafetyDescription.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(dataSafetyDescription);

        StackPanel backupActions;
        backupActions.Orientation(Orientation::Horizontal);
        backupActions.Spacing(8);
        m_exportBackupButton = Button();
        m_exportBackupButton.Content(winrt::box_value(L"导出备份"));
        m_exportBackupButton.IsEnabled(false);
        AutomationProperties::SetName(m_exportBackupButton, L"导出 TieZ 备份");
        m_restoreBackupButton = Button();
        m_restoreBackupButton.Content(winrt::box_value(L"恢复备份"));
        m_restoreBackupButton.IsEnabled(false);
        AutomationProperties::SetName(m_restoreBackupButton, L"恢复 TieZ 备份");
        m_backupProgress = ProgressRing();
        m_backupProgress.Width(22);
        m_backupProgress.Height(22);
        m_backupProgress.IsActive(false);
        m_backupProgress.Visibility(Visibility::Collapsed);
        backupActions.Children().Append(m_exportBackupButton);
        backupActions.Children().Append(m_restoreBackupButton);
        backupActions.Children().Append(m_backupProgress);
        m_settingsPanel.Children().Append(backupActions);

        m_backupStatus = TextBlock();
        m_backupStatus.TextWrapping(TextWrapping::Wrap);
        m_backupStatus.IsTextSelectionEnabled(true);
        m_backupStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_backupStatus, L"备份状态");
        m_settingsPanel.Children().Append(m_backupStatus);

        m_exportBackupButton.Click([this](auto const&, auto const&)
        {
            ExportBackupAsync();
        });
        m_restoreBackupButton.Click([this](auto const&, auto const&)
        {
            RestoreBackupAsync();
        });

        TextBlock updateTitle;
        updateTitle.Text(L"应用更新");
        updateTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        m_settingsPanel.Children().Append(updateTitle);

        TextBlock updateDescription;
        updateDescription.Text(
            L"正式 MSIX 版本使用安装时关联的 .appinstaller 源。TieZ 不保存或替换该地址，下载、发布者签名校验和升级均由 Windows App Installer 完成。");
        updateDescription.TextWrapping(TextWrapping::Wrap);
        updateDescription.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        m_settingsPanel.Children().Append(updateDescription);

        StackPanel updateActions;
        updateActions.Orientation(Orientation::Horizontal);
        updateActions.Spacing(8);
        m_checkUpdateButton = Button();
        m_checkUpdateButton.Content(winrt::box_value(L"检查更新"));
        AutomationProperties::SetName(m_checkUpdateButton, L"检查 TieZ 更新");
        m_installUpdateButton = Button();
        m_installUpdateButton.Content(winrt::box_value(L"安装更新"));
        m_installUpdateButton.IsEnabled(false);
        AutomationProperties::SetName(m_installUpdateButton, L"使用 Windows App Installer 安装 TieZ 更新");
        m_updateProgress = ProgressRing();
        m_updateProgress.Width(22);
        m_updateProgress.Height(22);
        m_updateProgress.IsActive(false);
        m_updateProgress.Visibility(Visibility::Collapsed);
        updateActions.Children().Append(m_checkUpdateButton);
        updateActions.Children().Append(m_installUpdateButton);
        updateActions.Children().Append(m_updateProgress);
        m_settingsPanel.Children().Append(updateActions);

        m_updateStatus = TextBlock();
        m_updateStatus.Text(
            L"点击“检查更新”后，Windows 会查询当前安装包关联的发布源。未打包开发版本不会访问网络。");
        m_updateStatus.TextWrapping(TextWrapping::Wrap);
        m_updateStatus.IsTextSelectionEnabled(true);
        m_updateStatus.Foreground(Application::Current().Resources()
            .Lookup(winrt::box_value(L"TextFillColorSecondaryBrush")).as<Brush>());
        AutomationProperties::SetName(m_updateStatus, L"TieZ 更新状态");
        m_settingsPanel.Children().Append(m_updateStatus);

        m_checkUpdateButton.Click([this](auto const&, auto const&)
        {
            CheckForUpdatesAsync();
        });
        m_installUpdateButton.Click([this](auto const&, auto const&)
        {
            OpenAppInstallerAsync();
        });

        ScrollViewer scroller;
        scroller.MaxHeight(620);
        scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
        scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
        scroller.Content(m_settingsPanel);
        m_settingsDialog.Content(scroller);
        m_settingsDialog.Closed([this](auto const&, auto const&)
        {
            m_suspendLifecycle = false;
            SearchBox().Focus(FocusState::Programmatic);
        });

        m_colorModeCombo.SelectionChanged([this](auto const&, auto const&)
        {
            if (m_settingsLoading)
            {
                return;
            }
            auto const index = m_colorModeCombo.SelectedIndex();
            auto const mode = index == 1 ? "light" : index == 2 ? "dark" : "system";
            if (PersistSetting("app.color_mode", mode, L"界面主题"))
            {
                ApplyColorMode(mode);
            }
            else
            {
                LoadSettings();
            }
        });
        m_hotkeyApplyButton.Click([this](auto const&, auto const&)
        {
            if (!m_settingsLoading)
            {
                SaveToggleHotkey();
            }
        });
        m_compactModeToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_compactModeToggle.IsOn();
            if (PersistSetting(
                "app.compact_mode",
                enabled ? "true" : "false",
                L"紧凑列表"))
            {
                m_compactMode = enabled;
                RefreshItems();
            }
            else LoadSettings();
        });
        m_windowPinnedToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_windowPinnedToggle.IsOn();
            if (PersistSetting(
                "app.window_pinned",
                enabled ? "true" : "false",
                L"固定窗口"))
            {
                ApplyPinnedWindow(enabled);
            }
            else LoadSettings();
        });
        m_trayVisibleToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const visible = m_trayVisibleToggle.IsOn();
            if (PersistSetting(
                "app.hide_tray_icon",
                visible ? "false" : "true",
                L"系统托盘"))
            {
                SetTrayVisible(visible);
            }
            else LoadSettings();
        });
        m_autostartToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading || m_autostartBusy) return;
            ApplyAutostartAsync(m_autostartToggle.IsOn());
        });
        m_persistentToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_persistentToggle.IsOn();
            if (!PersistSetting(
                "app.persistent",
                enabled ? "true" : "false",
                L"持久保存历史"))
            {
                LoadSettings();
            }
        });
        m_persistentLimitEnabledToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_persistentLimitEnabledToggle.IsOn();
            if (PersistSetting(
                "app.persistent_limit_enabled",
                enabled ? "true" : "false",
                L"历史数量限制"))
            {
                m_persistentLimitNumber.IsEnabled(enabled && !m_settingsReadOnly);
            }
            else LoadSettings();
        });
        m_persistentLimitNumber.ValueChanged([this](auto const&, auto const& args)
        {
            if (m_settingsLoading || !std::isfinite(args.NewValue())) return;
            auto const limit = static_cast<std::int64_t>(std::llround(args.NewValue()));
            if (!PersistSetting(
                "app.persistent_limit",
                std::to_string(limit),
                L"历史数量上限"))
            {
                LoadSettings();
            }
        });
        m_deduplicateToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_deduplicateToggle.IsOn();
            if (!PersistSetting(
                "app.deduplicate",
                enabled ? "true" : "false",
                L"自动去重"))
            {
                LoadSettings();
            }
        });
        m_captureFilesToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_captureFilesToggle.IsOn();
            if (!PersistSetting(
                "app.capture_files",
                enabled ? "true" : "false",
                L"文件捕获"))
            {
                LoadSettings();
            }
        });
        m_captureRichTextToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_captureRichTextToggle.IsOn();
            if (!PersistSetting(
                "app.capture_rich_text",
                enabled ? "true" : "false",
                L"富文本捕获"))
            {
                LoadSettings();
            }
        });
        m_privacyProtectionToggle.Toggled([this](auto const&, auto const&)
        {
            if (m_settingsLoading) return;
            auto const enabled = m_privacyProtectionToggle.IsOn();
            if (!PersistSetting(
                "app.privacy_protection",
                enabled ? "true" : "false",
                L"隐私保护"))
            {
                LoadSettings();
            }
        });
    }

    void MainWindow::LoadSettings()
    {
        if (!m_core)
        {
            return;
        }

        auto const value = m_core->Settings();
        auto const root = JsonObject::Parse(tiez::probe::RustCoreBridge::Utf8ToHstring(value));
        auto const values = root.GetNamedObject(L"values");
        auto const getValue = [&values](wchar_t const* key, wchar_t const* fallback)
        {
            return values.HasKey(key) ? values.GetNamedString(key) : winrt::hstring{ fallback };
        };
        auto const getBool = [&getValue](wchar_t const* key, bool fallback)
        {
            auto const value = getValue(key, fallback ? L"true" : L"false");
            return value == L"true" || value == L"1";
        };

        auto const colorMode = getValue(L"app.color_mode", L"system");
        auto const savedHotkey = getValue(L"app.hotkey", L"Alt+C");
        auto hotkey = savedHotkey;
        auto hotkeyOverridden = false;
        if (auto const diagnosticHotkey = ReadEnvironmentText(L"TIEZ_WINUI_HOTKEY"))
        {
            hotkey = *diagnosticHotkey;
            hotkeyOverridden = true;
        }
        auto const compactMode = getBool(L"app.compact_mode", false);
        auto const persistent = getBool(L"app.persistent", false);
        auto const limitEnabled = getBool(L"app.persistent_limit_enabled", true);
        auto const deduplicate = getBool(L"app.deduplicate", true);
        auto const captureFiles = getBool(L"app.capture_files", false);
        auto const captureRichText = getBool(L"app.capture_rich_text", false);
        auto const privacyProtection = getBool(L"app.privacy_protection", true);
        auto const trayVisible = !getBool(L"app.hide_tray_icon", false);
        auto const pinned = getBool(L"app.window_pinned", false);
        auto const autostart = getBool(L"app.autostart", true);
        auto const adapter = root.GetNamedString(L"adapter", L"memory");
        double persistentLimit = 500;
        try
        {
            persistentLimit = std::stod(std::wstring{
                getValue(L"app.persistent_limit", L"500").c_str() });
        }
        catch (std::exception const&)
        {
            persistentLimit = 500;
        }

        auto const compactChanged = m_compactMode != compactMode;
        m_settingsReadOnly = root.GetNamedBoolean(L"read_only");
        m_productionData = adapter == L"sqlite" || adapter == L"sqlite-read-only";
        m_configuredHotkey = savedHotkey;
        auto const hotkeyApplied = ApplyToggleHotkey(hotkey);
        m_settingsLoading = true;
        m_compactMode = compactMode;
        m_autostartPreference = autostart;
        if (m_settingsPanel)
        {
            auto const settingsEnabled = !m_settingsReadOnly;
            m_settingsPanel.IsHitTestVisible(true);
            m_settingsPanel.Opacity(1.0);
            m_colorModeCombo.IsEnabled(settingsEnabled);
            m_compactModeToggle.IsEnabled(settingsEnabled);
            m_persistentToggle.IsEnabled(settingsEnabled);
            m_persistentLimitEnabledToggle.IsEnabled(settingsEnabled);
            m_deduplicateToggle.IsEnabled(settingsEnabled);
            m_captureFilesToggle.IsEnabled(settingsEnabled);
            m_captureRichTextToggle.IsEnabled(settingsEnabled);
            m_privacyProtectionToggle.IsEnabled(settingsEnabled);
            m_trayVisibleToggle.IsEnabled(settingsEnabled);
            m_windowPinnedToggle.IsEnabled(settingsEnabled);
            m_autostartToggle.IsEnabled(settingsEnabled && !m_autostartBusy);
            m_hotkeyEditor.IsEnabled(settingsEnabled && !hotkeyOverridden);
            m_hotkeyApplyButton.IsEnabled(settingsEnabled && !hotkeyOverridden);
            m_hotkeyEditor.Text(savedHotkey);
            if (hotkeyOverridden)
            {
                std::wstring message{ L"诊断模式正在临时使用 " };
                message.append(hotkey.c_str(), hotkey.size());
                message.append(L"；关闭环境变量后可编辑已保存值。");
                m_hotkeySettingsStatus.Text(winrt::hstring{ message });
            }
            else if (savedHotkey.empty())
            {
                m_hotkeySettingsStatus.Text(L"呼出快捷键已停用，仍可通过系统托盘打开 TieZ。");
            }
            else if (hotkeyApplied)
            {
                std::wstring message{ L"当前已启用：" };
                message.append(savedHotkey.c_str(), savedHotkey.size());
                m_hotkeySettingsStatus.Text(winrt::hstring{ message });
            }
            else if (m_hotkeyRegistered)
            {
                std::wstring message{ L"已保存值当前不可用；继续使用：" };
                message.append(m_registeredHotkey.c_str(), m_registeredHotkey.size());
                m_hotkeySettingsStatus.Text(winrt::hstring{ message });
            }
            else
            {
                m_hotkeySettingsStatus.Text(L"已保存值当前不可用，请使用系统托盘打开 TieZ。");
            }
            m_colorModeCombo.SelectedIndex(colorMode == L"light" ? 1 : colorMode == L"dark" ? 2 : 0);
            m_compactModeToggle.IsOn(compactMode);
            m_persistentToggle.IsOn(persistent);
            m_persistentLimitEnabledToggle.IsOn(limitEnabled);
            m_persistentLimitNumber.Value(persistentLimit);
            m_persistentLimitNumber.IsEnabled(limitEnabled && !m_settingsReadOnly);
            m_deduplicateToggle.IsOn(deduplicate);
            m_captureFilesToggle.IsOn(captureFiles);
            m_captureRichTextToggle.IsOn(captureRichText);
            m_privacyProtectionToggle.IsOn(privacyProtection);
            m_trayVisibleToggle.IsOn(trayVisible);
            m_windowPinnedToggle.IsOn(pinned);
            m_autostartToggle.IsOn(autostart);

            auto backupMessage = m_backupStatus.Text();
            if (backupMessage.empty())
            {
                if (!m_productionData)
                {
                    backupMessage = L"当前使用演示数据。连接生产数据库后才能导出或恢复备份。";
                }
                else if (m_settingsReadOnly)
                {
                    backupMessage = L"当前为只读生产数据：可以导出备份，不能安排恢复。";
                }
                else
                {
                    backupMessage = L"建议定期导出备份，并将文件保存在 TieZ 数据目录之外。";
                }
            }
            SetBackupBusy(m_backupBusy, backupMessage);
        }
        m_settingsLoading = false;
        LoadCloudSyncSettings();

        ApplyColorMode(winrt::to_string(colorMode));
        ApplyPinnedWindow(pinned);
        SetTrayVisible(trayVisible);
        if (compactChanged && m_core)
        {
            RefreshItems();
        }
    }

    void MainWindow::SetAutostartUi(
        bool enabled,
        bool canChange,
        winrt::hstring const& message)
    {
        if (!m_autostartToggle)
        {
            return;
        }

        auto const wasLoading = m_settingsLoading;
        m_settingsLoading = true;
        m_autostartToggle.IsOn(enabled);
        m_autostartToggle.IsEnabled(
            canChange && !m_settingsReadOnly && !m_autostartBusy);
        m_autostartStatus.Text(message);
        m_settingsLoading = wasLoading;
    }

    winrt::fire_and_forget MainWindow::RefreshAutostartStateAsync(
        bool reconcilePreference)
    {
        auto lifetime = get_strong();
        if (m_autostartBusy)
        {
            co_return;
        }

        m_autostartBusy = true;
        SetAutostartUi(
            m_autostartPreference,
            false,
            L"正在读取 Windows 登录启动状态……");
        try
        {
            if (HasPackageIdentity())
            {
                auto const task = co_await StartupTask::GetAsync(kAutostartTaskId);
                auto state = task.State();
                bool reconciled{};
                if (reconcilePreference && !m_settingsReadOnly)
                {
                    if (m_autostartPreference
                        && state == StartupTaskState::Disabled)
                    {
                        state = co_await task.RequestEnableAsync();
                        reconciled = true;
                    }
                    else if (!m_autostartPreference
                        && StartupTaskIsEnabled(state)
                        && StartupTaskCanChange(state))
                    {
                        task.Disable();
                        state = task.State();
                        reconciled = true;
                    }
                }

                // The packaged WinUI app owns login startup after installation.
                RemoveLegacyAutostartValues();
                m_autostartBusy = false;
                SetAutostartUi(
                    StartupTaskIsEnabled(state),
                    StartupTaskCanChange(state),
                    StartupTaskStatus(state, reconciled));
            }
            else
            {
                auto const enabled = IsNativeRunAutostartEnabled();
                m_autostartBusy = false;
                SetAutostartUi(
                    enabled,
                    true,
                    enabled
                        ? L"未打包版本已注册当前原生 TieZ；登录后只在托盘后台启动。"
                        : L"未打包版本不会随 Windows 登录启动。正式 MSIX 使用系统启动任务。");
            }
        }
        catch (winrt::hresult_error const& error)
        {
            m_autostartBusy = false;
            SetAutostartUi(
                m_autostartPreference,
                true,
                StatusMessage(L"读取开机启动状态失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_autostartBusy = false;
            SetAutostartUi(
                m_autostartPreference,
                true,
                StatusMessage(
                    L"读取开机启动状态失败：",
                    tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    winrt::fire_and_forget MainWindow::ApplyAutostartAsync(bool enabled)
    {
        auto lifetime = get_strong();
        if (m_autostartBusy)
        {
            co_return;
        }

        auto const previousPreference = m_autostartPreference;
        m_autostartBusy = true;
        SetAutostartUi(
            enabled,
            false,
            enabled ? L"正在注册 Windows 登录启动……" : L"正在关闭 Windows 登录启动……");
        try
        {
            bool actual{};
            bool canChange{ true };
            winrt::hstring message;
            if (HasPackageIdentity())
            {
                auto const task = co_await StartupTask::GetAsync(kAutostartTaskId);
                auto state = task.State();
                auto const previousEnabled = StartupTaskIsEnabled(state);
                if (enabled && state == StartupTaskState::Disabled)
                {
                    state = co_await task.RequestEnableAsync();
                }
                else if (!enabled
                    && StartupTaskIsEnabled(state)
                    && StartupTaskCanChange(state))
                {
                    task.Disable();
                    state = task.State();
                }

                actual = StartupTaskIsEnabled(state);
                canChange = StartupTaskCanChange(state);
                message = StartupTaskStatus(state, true);
                if (actual != enabled)
                {
                    m_autostartBusy = false;
                    SetAutostartUi(actual, canChange, message);
                    co_return;
                }

                if (!PersistSetting(
                    "app.autostart",
                    actual ? "true" : "false",
                    L"开机启动"))
                {
                    if (previousEnabled != actual)
                    {
                        if (previousEnabled && task.State() == StartupTaskState::Disabled)
                        {
                            (void)co_await task.RequestEnableAsync();
                        }
                        else if (!previousEnabled
                            && StartupTaskIsEnabled(task.State())
                            && StartupTaskCanChange(task.State()))
                        {
                            task.Disable();
                        }
                    }
                    m_autostartBusy = false;
                    RefreshAutostartStateAsync(false);
                    co_return;
                }
                RemoveLegacyAutostartValues();
            }
            else
            {
                auto const previousEnabled = IsNativeRunAutostartEnabled();
                SetNativeRunAutostart(enabled);
                actual = IsNativeRunAutostartEnabled();
                if (actual != enabled)
                {
                    throw std::runtime_error("Windows Run registration did not persist");
                }
                if (!PersistSetting(
                    "app.autostart",
                    actual ? "true" : "false",
                    L"开机启动"))
                {
                    SetNativeRunAutostart(previousEnabled);
                    m_autostartBusy = false;
                    RefreshAutostartStateAsync(false);
                    co_return;
                }
                message = actual
                    ? L"已注册当前原生 TieZ；下次登录后只在托盘后台启动。"
                    : L"已从 Windows 登录启动项移除 TieZ。";
            }

            m_autostartPreference = actual;
            m_autostartBusy = false;
            SetAutostartUi(actual, canChange, message);
        }
        catch (winrt::hresult_error const& error)
        {
            m_autostartPreference = previousPreference;
            m_autostartBusy = false;
            SetAutostartUi(
                previousPreference,
                true,
                StatusMessage(L"更新开机启动失败：", error.message()));
        }
        catch (std::exception const& error)
        {
            m_autostartPreference = previousPreference;
            m_autostartBusy = false;
            SetAutostartUi(
                previousPreference,
                true,
                StatusMessage(
                    L"更新开机启动失败：",
                    tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    bool MainWindow::PersistSetting(
        std::string_view key,
        std::string_view value,
        winrt::hstring const& label)
    {
        if (!m_core || m_settingsReadOnly)
        {
            SetStatus(m_settingsReadOnly
                ? L"当前数据库以只读方式打开，无法保存设置。"
                : L"Rust 核心尚未就绪，无法保存设置。");
            return false;
        }

        try
        {
            (void)m_core->UpdateSetting(key, value);
            std::wstring status{ label.c_str(), label.size() };
            status.append(L"已保存并立即生效。");
            SetStatus(winrt::hstring{ status });
            return true;
        }
        catch (std::exception const& error)
        {
            SetStatus(StatusMessage(
                L"保存设置失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
            return false;
        }
    }

    void MainWindow::ApplyColorMode(std::string_view mode)
    {
        auto const theme =
            mode == "light" ? ElementTheme::Light
            : mode == "dark" ? ElementTheme::Dark
            : ElementTheme::Default;
        RootGrid().RequestedTheme(theme);
        if (m_hoverPreviewRoot)
        {
            m_hoverPreviewRoot.RequestedTheme(theme);
        }
    }

    void MainWindow::ApplyPinnedWindow(bool pinned)
    {
        m_pinned = pinned;
        auto const wasLoading = m_settingsLoading;
        m_settingsLoading = true;
        PinWindowCheck().IsChecked(pinned);
        if (m_windowPinnedToggle)
        {
            m_windowPinnedToggle.IsOn(pinned);
        }
        m_settingsLoading = wasLoading;
        SetWindowPos(
            GetWindowHandle(),
            pinned ? HWND_TOPMOST : HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }

    void MainWindow::EnsureHoverPreviewWindow()
    {
        if (m_hoverPreviewWindow)
        {
            return;
        }

        m_hoverPreviewWindow = Window();
        m_hoverPreviewWindow.Title(L"TieZ 剪贴板预览");

        m_hoverPreviewRoot = Border();
        m_hoverPreviewRoot.Padding(ThicknessHelper::FromLengths(16, 14, 16, 14));
        m_hoverPreviewRoot.CornerRadius(CornerRadiusHelper::FromUniformRadius(12));
        m_hoverPreviewRoot.BorderThickness(ThicknessHelper::FromUniformLength(1));
        m_hoverPreviewRoot.Background(Application::Current().Resources()
            .Lookup(winrt::box_value(L"CardBackgroundFillColorDefaultBrush")).as<Brush>());
        m_hoverPreviewRoot.BorderBrush(Application::Current().Resources()
            .Lookup(winrt::box_value(L"CardStrokeColorDefaultBrush")).as<Brush>());
        m_hoverPreviewRoot.RequestedTheme(RootGrid().RequestedTheme());

        Grid layout;
        layout.RowSpacing(10);
        RowDefinition titleRow;
        titleRow.Height(GridLengthHelper::Auto());
        RowDefinition contentRow;
        contentRow.Height(GridLength{ 1, GridUnitType::Star });
        layout.RowDefinitions().Append(titleRow);
        layout.RowDefinitions().Append(contentRow);

        m_hoverPreviewTitle = TextBlock();
        m_hoverPreviewTitle.Style(Application::Current().Resources()
            .Lookup(winrt::box_value(L"SubtitleTextBlockStyle")).as<Style>());
        layout.Children().Append(m_hoverPreviewTitle);

        ScrollViewer scroller;
        scroller.HorizontalScrollBarVisibility(ScrollBarVisibility::Disabled);
        scroller.VerticalScrollBarVisibility(ScrollBarVisibility::Auto);
        Grid::SetRow(scroller, 1);
        StackPanel body;
        body.Spacing(10);
        m_hoverPreviewImage = Image();
        m_hoverPreviewImage.MaxHeight(190);
        m_hoverPreviewImage.Stretch(Stretch::Uniform);
        m_hoverPreviewImage.Visibility(Visibility::Collapsed);
        AutomationProperties::SetName(m_hoverPreviewImage, L"紧凑模式图片预览");
        m_hoverPreviewText = TextBlock();
        m_hoverPreviewText.FontFamily(FontFamily{ L"Consolas" });
        m_hoverPreviewText.TextWrapping(TextWrapping::Wrap);
        m_hoverPreviewText.IsTextSelectionEnabled(false);
        body.Children().Append(m_hoverPreviewImage);
        body.Children().Append(m_hoverPreviewText);
        scroller.Content(body);
        layout.Children().Append(scroller);
        m_hoverPreviewRoot.Child(layout);
        m_hoverPreviewWindow.Content(m_hoverPreviewRoot);

        Window preview = m_hoverPreviewWindow;
        winrt::check_hresult(preview.as<::IWindowNative>()->get_WindowHandle(&m_hoverPreviewHwnd));
        auto style = GetWindowLongPtrW(m_hoverPreviewHwnd, GWL_STYLE);
        style &= ~(WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU);
        style |= WS_POPUP;
        SetWindowLongPtrW(m_hoverPreviewHwnd, GWL_STYLE, style);
        auto extendedStyle = GetWindowLongPtrW(m_hoverPreviewHwnd, GWL_EXSTYLE);
        extendedStyle |= WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        SetWindowLongPtrW(m_hoverPreviewHwnd, GWL_EXSTYLE, extendedStyle);
        SetWindowSubclass(
            m_hoverPreviewHwnd,
            HoverPreviewSubclassProc,
            kHoverPreviewSubclassId,
            0);
        SetWindowPos(
            m_hoverPreviewHwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }

    void MainWindow::ShowHoverPreview(std::int64_t entryId)
    {
        if (!m_compactMode || !m_core)
        {
            return;
        }

        try
        {
            EnsureHoverPreviewWindow();
            auto const value = m_core->Content(entryId);
            auto const content = JsonObject::Parse(
                tiez::probe::RustCoreBridge::Utf8ToHstring(value));
            auto const contentType = content.GetNamedString(L"content_type");
            auto const available = content.GetNamedBoolean(L"available");
            auto const isSensitive = content.GetNamedBoolean(L"is_sensitive");

            std::wstringstream title;
            title << ContentTypeLabel(contentType).c_str() << L" · 记录 " << entryId;
            m_hoverPreviewTitle.Text(winrt::hstring{ title.str() });

            winrt::hstring displayContent;
            if (isSensitive)
            {
                displayContent = L"此记录受隐私保护，悬停预览已隐藏。";
            }
            else if (!available)
            {
                displayContent = L"此记录的完整内容当前不可用。";
            }
            else
            {
                displayContent = content.GetNamedString(L"content");
                if (displayContent.empty())
                {
                    auto const htmlContent = content.GetNamedValue(L"html_content");
                    if (htmlContent.ValueType() == JsonValueType::String)
                    {
                        displayContent = htmlContent.GetString();
                    }
                }
            }
            m_hoverPreviewText.Text(displayContent);
            ShowHoverPreviewImage(
                available && !isSensitive ? contentType : winrt::hstring{},
                displayContent);

            POINT cursor{};
            GetCursorPos(&cursor);
            MONITORINFO monitorInfo{};
            monitorInfo.cbSize = sizeof(monitorInfo);
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &monitorInfo);
            auto const scale = static_cast<double>(GetDpiForWindow(GetWindowHandle())) / 96.0;
            auto const width = static_cast<int>(420 * scale);
            auto const height = static_cast<int>(300 * scale);
            auto x = cursor.x + static_cast<int>(18 * scale);
            auto y = cursor.y + static_cast<int>(18 * scale);
            if (x + width > monitorInfo.rcWork.right)
            {
                x = cursor.x - width - static_cast<int>(18 * scale);
            }
            if (y + height > monitorInfo.rcWork.bottom)
            {
                y = monitorInfo.rcWork.bottom - height;
            }
            x = std::max(monitorInfo.rcWork.left, x);
            y = std::max(monitorInfo.rcWork.top, y);
            SetWindowPos(
                m_hoverPreviewHwnd,
                HWND_TOPMOST,
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        catch (std::exception const& error)
        {
            HideHoverPreview();
            SetStatus(StatusMessage(
                L"悬停预览失败：",
                tiez::probe::RustCoreBridge::Utf8ToHstring(error.what())));
        }
    }

    void MainWindow::HideHoverPreview()
    {
        if (m_hoverPreviewHwnd != nullptr && IsWindow(m_hoverPreviewHwnd))
        {
            ShowWindow(m_hoverPreviewHwnd, SW_HIDE);
        }
    }

    void MainWindow::ShowHoverPreviewImage(
        winrt::hstring const& contentType,
        winrt::hstring const& content)
    {
        if (!m_hoverPreviewImage)
        {
            return;
        }
        m_hoverPreviewImage.Source(nullptr);
        m_hoverPreviewImage.Visibility(Visibility::Collapsed);
        if (contentType != L"image")
        {
            return;
        }

        std::wstring path{ content };
        if (path.empty()
            || path.rfind(L"data:image/", 0) == 0
            || !std::filesystem::exists(std::filesystem::path{ content.c_str() }))
        {
            return;
        }
        std::replace(path.begin(), path.end(), L'\\', L'/');
        Microsoft::UI::Xaml::Media::Imaging::BitmapImage bitmap;
        bitmap.UriSource(Windows::Foundation::Uri{ L"file:///" + path });
        m_hoverPreviewImage.Source(bitmap);
        m_hoverPreviewImage.Visibility(Visibility::Visible);
    }

    void MainWindow::SetupImeGuards()
    {
        SearchBox().TextCompositionStarted([this](auto const&, auto const&)
        {
            m_imeComposing = true;
        });
        SearchBox().TextCompositionEnded([this](auto const&, auto const&)
        {
            m_imeComposing = false;
            m_ignoreNextEnter = true;
        });
        TagsTextBox().TextCompositionStarted([this](auto const&, auto const&)
        {
            m_imeComposing = true;
        });
        TagsTextBox().TextCompositionEnded([this](auto const&, auto const&)
        {
            m_imeComposing = false;
            m_ignoreNextEnter = true;
        });
    }

    void MainWindow::ShowDetailsImage(winrt::hstring const& contentType, winrt::hstring const& content)
    {
        DetailsImage().Source(nullptr);
        DetailsImage().Visibility(Visibility::Collapsed);
        if (contentType != L"image")
        {
            return;
        }

        std::wstring path{ content };
        if (path.empty() || path.rfind(L"data:image/", 0) == 0)
        {
            return;
        }
        std::replace(path.begin(), path.end(), L'\\', L'/');
        if (!std::filesystem::exists(std::filesystem::path{ content.c_str() }))
        {
            return;
        }

        try
        {
            Microsoft::UI::Xaml::Media::Imaging::BitmapImage bitmap;
            bitmap.UriSource(Windows::Foundation::Uri{ L"file:///" + path });
            DetailsImage().Source(bitmap);
            DetailsImage().Visibility(Visibility::Visible);
        }
        catch (winrt::hresult_error const&)
        {
        }
    }
}

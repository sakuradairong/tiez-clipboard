namespace TieZ.Native;

/// <summary>Accepted interface languages of the shared core configuration.</summary>
internal enum Language
{
    SimplifiedChinese,
    TraditionalChinese,
    English
}

/// <summary>
/// Owns the single native interface language. The main window publishes each accepted settings snapshot here,
/// so every live window and view reads the same value instead of re-reading configuration on its own.
/// </summary>
internal static class LanguageService
{
    private static Language current = Language.SimplifiedChinese;

    internal static Language Current => current;

    /// <summary>Raised on the UI thread only when the accepted language actually changed.</summary>
    internal static event Action? Changed;

    internal static Language FromSettings(string? value) => value switch
    {
        "en" => Language.English,
        "tw" => Language.TraditionalChinese,
        _ => Language.SimplifiedChinese
    };

    internal static string ToSettingsValue(Language language) => language switch
    {
        Language.English => "en",
        Language.TraditionalChinese => "tw",
        _ => "zh"
    };

    /// <summary>Applies one accepted configuration value; unchanged values notify nobody.</summary>
    internal static bool Apply(string? settingsValue)
    {
        var next = FromSettings(settingsValue);
        if (next == current) return false;
        current = next;
        Changed?.Invoke();
        return true;
    }
}
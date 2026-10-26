// Test-only ownership checks; never discover or delete an installed application's data.
internal static class FixturePaths
{
    private const string Prefix = "TieZ-focus-";
    internal static bool CanDelete(string directory, string temporaryRoot)
    {
        if (!Path.IsPathFullyQualified(directory) || !Path.IsPathFullyQualified(temporaryRoot)) return false;
        string path = Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
        string root = Path.TrimEndingDirectorySeparator(Path.GetFullPath(temporaryRoot));
        string name = Path.GetFileName(path);
        return string.Equals(Path.GetDirectoryName(path), root, StringComparison.OrdinalIgnoreCase)
            && name.StartsWith(Prefix, StringComparison.Ordinal)
            && Guid.TryParseExact(name[Prefix.Length..], "N", out _)
            && Directory.Exists(path) && (File.GetAttributes(path) & FileAttributes.ReparsePoint) == 0;
    }
    internal static void CheckRules()
    {
        string root = Path.GetTempPath();
        string owned = Path.Combine(root, Prefix + Guid.NewGuid().ToString("N"));
        string malformed = Path.Combine(root, Prefix + "not-a-guid-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(owned); Directory.CreateDirectory(malformed);
        int checks = 0;
        void Check(bool value) { checks++; if (!value) throw new InvalidOperationException("Fixture cleanup ownership regression"); }
        try
        {
            Check(CanDelete(owned, root));
            Check(CanDelete(owned, Path.TrimEndingDirectorySeparator(root)));
            Check(CanDelete(owned, root.Replace(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)));
            Check(CanDelete(owned, root.ToUpperInvariant()));
            Check(CanDelete(owned + Path.DirectorySeparatorChar, root));
            Check(!CanDelete(malformed, root));
            Check(!CanDelete(owned, Path.Combine(root, "foreign-sibling")));
            Check(!CanDelete(Path.GetFileName(owned), root));
            Check(!CanDelete(root, root));
            Console.WriteLine($"PASS: {checks} cleanup ownership checks; normalized roots, strict GUID, foreign/relative path rejection; no focus, clipboard or keyboard APIs used.");
        }
        finally
        {
            // These uniquely created, empty fixture directories need no recursive deletion.
            Directory.Delete(owned); Directory.Delete(malformed);
        }
    }
}

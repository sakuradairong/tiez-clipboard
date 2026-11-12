using System.Runtime.InteropServices;

namespace TieZ.Native;

internal static class NativePasteActivation
{
    internal static async Task WaitAsync(CancellationToken token)
    {
        long deadline = Environment.TickCount64 + 1500;
        while (Held())
        {
            token.ThrowIfCancellationRequested();
            if (Environment.TickCount64 >= deadline)
                throw new InvalidOperationException(QueueStrings.ReleaseKeys);
            await Task.Delay(15, token);
        }
    }
    private static bool Held()
    {
        // Never synthesize release events for keys physically held by the user.
        for (int key = 1; key < 255; key++)
            if ((GetAsyncKeyState(key) & 0x8000) != 0) return true;
        return false;
    }
    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
}

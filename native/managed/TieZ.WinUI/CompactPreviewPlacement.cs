using Windows.Graphics;

namespace TieZ.Native;

internal static class CompactPreviewPlacement
{
    // All arguments/results are physical pixels. WorkArea includes taskbar reservations.
    internal static RectInt32 Calculate(RectInt32 owner, RectInt32 anchor, RectInt32 work, double scale)
    {
        if (work.Width < 1 || work.Height < 1 || !double.IsFinite(scale) || scale is < 0.5 or > 8)
            throw new ArgumentOutOfRangeException(nameof(work));
        int margin = Math.Min((int)Math.Round(10 * scale), Math.Max(0, Math.Min(work.Width, work.Height) / 4));
        int gap = (int)Math.Round(12 * scale);
        int width = Math.Min((int)Math.Round(420 * scale), work.Width - 2 * margin);
        int height = Math.Min((int)Math.Round(360 * scale), work.Height - 2 * margin);
        int left = work.X + margin, top = work.Y + margin;
        int right = work.X + work.Width - margin, bottom = work.Y + work.Height - margin;
        int rightX = owner.X + owner.Width + gap;
        int leftX = owner.X - width - gap;
        int x;
        if (rightX >= left && rightX <= right - width) x = rightX;
        else if (leftX >= left && leftX <= right - width) x = leftX;
        else
        {
            int rightRoom = Math.Max(0, right - rightX);
            int leftRoom = Math.Max(0, owner.X - gap - left);
            if (Math.Max(rightRoom, leftRoom) >= 240 * scale)
            {
                width = Math.Min(width, Math.Max(rightRoom, leftRoom));
                x = rightRoom >= leftRoom ? rightX : owner.X - gap - width;
            }
            else x = Math.Clamp(anchor.X + anchor.Width + gap, left, right - width);
        }
        return new RectInt32(x, Math.Clamp(anchor.Y, top, bottom - height), width, height);
    }
}

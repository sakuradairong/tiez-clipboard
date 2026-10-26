using System.Text.Json;

namespace TieZ.Core;

// The exact catalog identity an icon is bound to: the `source_id`, `handler`, `kind` and
// `unavailable` fields of an accepted catalog row. A display name, path, launch target or pixel
// buffer is never accepted as an identity, and this record carries only the four catalog fields.
public sealed record ApplicationIconIdentity(string SourceId, string? Handler, string Kind, string? Unavailable);

// One owned, bounded raster. The buffer is copied into the result and exposed read-only, and no
// raster outlives the request that produced it.
public sealed class ApplicationIconRaster
{
    private readonly byte[] pixels;

    internal ApplicationIconRaster(ulong jobId, ulong catalogJobId, ApplicationIconIdentity identity,
        uint requestedEdge, int width, int height, byte[] pixels)
    {
        JobId = jobId;
        CatalogJobId = catalogJobId;
        Identity = identity;
        RequestedEdge = requestedEdge;
        Width = width;
        Height = height;
        this.pixels = pixels;
    }

    public ulong JobId { get; }
    public ulong CatalogJobId { get; }
    public ApplicationIconIdentity Identity { get; }
    public uint RequestedEdge { get; }
    public int Width { get; }
    public int Height { get; }
    // Top-down, straight-alpha RGBA of exactly Width * Height * 4 bytes.
    public ReadOnlyMemory<byte> Pixels => pixels;

    public override string ToString() => $"ApplicationIcon {Width}x{Height} ({pixels.Length} bytes, edge {RequestedEdge})";
}

public sealed partial class NativeCore
{
    // Platform raster contract: largest accepted/returned edge and largest raster payload.
    private const uint MaxIconEdge = 128;
    private const int MaxIconBytes = 65536;
    // Admitted requests: two workers plus a small bounded waiting set. A semaphore alone would
    // leave an unbounded number of waiters, so the total is counted at admission instead.
    private const int MaxIconActive = 2;
    private const int MaxIconOutstanding = 10;
    // A backend busy report can only clear after the previous worker terminated, so retry with a
    // bounded, cancellable yield rather than a spin or an unbounded restart.
    private const int IconBusyRetries = 8;
    private const int IconPollDelayMs = 40;
    private const int IconBusyDelayMs = 25;
    private const int MaxIconIdentityLength = 4096;

    private readonly SemaphoreSlim iconWorkGate = new(MaxIconActive, MaxIconActive);
    // Completes once every admitted icon request settled after shutdown. It never owns a waiter.
    private readonly TaskCompletionSource iconDrained = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private int admittedIcons;
    private int trackedIcons;

    // The owner initiates this call. Requests are bound to one accepted catalog generation and to
    // the exact identity of a row obtained from that catalog (including recommendation-only rows).
    // Invalid edge/identity and excess outstanding work fail before the first suspension; the
    // authoritative membership check remains in the C ABI.
    public Task<ApplicationIconRaster> GetApplicationIconAsync(ulong catalogJobId, InstalledApplication application,
        uint edge, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(application);
        VerifyAccess();
        cancellationToken.ThrowIfCancellationRequested();
        var identity = IconIdentityOf(catalogJobId, application, edge);
        if (Interlocked.Increment(ref admittedIcons) > MaxIconOutstanding)
        {
            Interlocked.Decrement(ref admittedIcons);
            throw new CoreException("icons_busy", "Too many application icon requests are already outstanding.");
        }
        Interlocked.Increment(ref trackedIcons);
        return RequestIconAsync(catalogJobId, identity, edge, cancellationToken);
    }

    private static ApplicationIconIdentity IconIdentityOf(ulong catalogJobId, InstalledApplication application, uint edge)
    {
        if (edge < 1 || edge > MaxIconEdge)
            throw new CoreException("icons_edge", $"Application icon edge must be 1..{MaxIconEdge} physical pixels.");
        if (catalogJobId == 0)
            throw new CoreException("icons_catalog", "No accepted application catalog generation was supplied.");
        if (application.Kind is "unavailable")
            throw new CoreException("icons_unavailable", "This catalog entry cannot provide an application icon.");
        if (application.Kind is not ("desktop" or "packaged") || application.Handler is null || application.Unavailable is not null)
            throw new CoreException("icons_identity", "Application identity is not a publishable catalog row.");
        // The limits below mirror the native bound (which measures UTF-8 bytes); native stays
        // authoritative for a string whose encoded length exceeds it.
        if (!IconBounded(application.SourceId) || !IconBounded(application.Handler))
            throw new CoreException("icons_identity", "Application identity exceeds the bounded identity contract.");
        return new(application.SourceId, application.Handler, application.Kind, null);
    }

    private static bool IconBounded(string value) =>
        value.Length <= MaxIconIdentityLength && !value.Any(char.IsControl);

    private async Task<ApplicationIconRaster> RequestIconAsync(ulong catalogJobId, ApplicationIconIdentity identity,
        uint edge, CancellationToken callerToken)
    {
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(callerToken, shutdown.Token);
        ulong ownedHandle = handle;
        ulong job = 0;
        bool acquired = false, delivered = false;
        try
        {
            await iconWorkGate.WaitAsync(cancellation.Token).ConfigureAwait(false);
            acquired = true;
            // A request cancelled while it was queued never reaches the native service.
            cancellation.Token.ThrowIfCancellationRequested();
            ObjectDisposedException.ThrowIf(handle == 0, this);
            job = await StartIconAsync(ownedHandle, catalogJobId, identity, edge, cancellation.Token,
                accepted => job = accepted).ConfigureAwait(false);
            while (true)
            {
                cancellation.Token.ThrowIfCancellationRequested();
                var polled = await PollIconAsync(ownedHandle, job, catalogJobId, edge, identity, cancellation.Token).ConfigureAwait(false);
                if (polled is null)
                {
                    await Task.Delay(IconPollDelayMs, cancellation.Token).ConfigureAwait(false);
                    continue;
                }
                // A successful poll and a concurrent cancel/refresh may overlap, so this client
                // rechecks its own token and lifetime before reporting the raster.
                cancellation.Token.ThrowIfCancellationRequested();
                delivered = true;
                return polled;
            }
        }
        finally
        {
            try
            {
                // The accepted job ID is retained even when the token is already cancelled, so an
                // accepted native job is always released here.
                if (job != 0 && !delivered)
                {
                    try { await SendIconCancelAsync(ownedHandle, job).ConfigureAwait(false); }
                    catch (CoreException failure) when (failure.Code is "invalid_handle" or "icons_job" or "icons_closed" or "core_closing") { }
                }
            }
            finally
            {
                if (acquired) iconWorkGate.Release();
                Interlocked.Decrement(ref admittedIcons);
                IconSettled();
            }
        }
    }

    private async Task<ulong> StartIconAsync(ulong ownedHandle, ulong catalogJobId, ApplicationIconIdentity identity,
        uint edge, CancellationToken cancellationToken, Action<ulong> accepted)
    {
        for (int attempt = 0; ; attempt++)
        {
            cancellationToken.ThrowIfCancellationRequested();
            JsonElement result;
            try
            {
                // Submitted exactly once: a start that may already be accepted must keep its job ID
                // observable so the caller can cancel it.
                result = await IconRequestAsync(new
                {
                    op = "apps_icon_start",
                    handle = ownedHandle,
                    catalog_job_id = catalogJobId,
                    identity,
                    edge,
                }, cancellationToken, submitExactlyOnce: true).ConfigureAwait(false);
            }
            catch (CoreException failure) when (failure.Code == "icons_busy" && attempt < IconBusyRetries)
            {
                await Task.Delay(IconBusyDelayMs, cancellationToken).ConfigureAwait(false);
                continue;
            }
            // Recorded before any further validation so a rejected response still frees its slot.
            ulong job = IconJobId(result);
            if (job != 0) accepted(job);
            ValidateIconStart(result, job, catalogJobId, edge);
            return job;
        }
    }

    private async Task<ApplicationIconRaster?> PollIconAsync(ulong ownedHandle, ulong job, ulong catalogJobId,
        uint edge, ApplicationIconIdentity identity, CancellationToken cancellationToken)
    {
        var result = await IconRequestAsync(new { op = "apps_icon_poll", handle = ownedHandle, job_id = job },
            cancellationToken).ConfigureAwait(false);
        return ParseIconPoll(result, job, catalogJobId, edge, identity);
    }

    private Task<JsonElement> SendIconCancelAsync(ulong ownedHandle, ulong job) =>
        IconRequestAsync(new { op = "apps_icon_cancel", handle = ownedHandle, job_id = job },
            CancellationToken.None, submitExactlyOnce: true);

    private async Task<JsonElement> IconRequestAsync(object request, CancellationToken cancellationToken,
        bool submitExactlyOnce = false)
    {
        byte[] bytes = SerializeRequest(request);
        await requestGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            return await Task.Run(() =>
            {
                if (!submitExactlyOnce) cancellationToken.ThrowIfCancellationRequested();
                return RequestBytes(bytes);
            }).ConfigureAwait(false);
        }
        finally { requestGate.Release(); }
    }

    private static void ValidateIconStart(JsonElement result, ulong job, ulong catalogJobId, uint edge)
    {
        if (job == 0) throw InvalidIcon("Application icon start returned no job ID.");
        if (IconUInt64(result, "catalog_job_id") != catalogJobId)
            throw InvalidIcon("Application icon start reported another catalog generation.");
        if (IconUInt64(result, "edge") != edge)
            throw InvalidIcon("Application icon start reported another edge.");
        if (IconState(result) != "running")
            throw InvalidIcon("Application icon start returned an unknown state.");
    }

    // Running is reported as null. A ready response is validated field by field against the request
    // before any pixel is adopted; nothing in it is taken from the caller.
    private static ApplicationIconRaster? ParseIconPoll(JsonElement result, ulong job, ulong catalogJobId,
        uint edge, ApplicationIconIdentity identity)
    {
        if (IconUInt64(result, "job_id") != job)
            throw InvalidIcon("Application icon job identity changed.");
        if (IconUInt64(result, "catalog_job_id") != catalogJobId)
            throw InvalidIcon("Application icon catalog generation changed.");
        if (IconUInt64(result, "edge") != edge)
            throw InvalidIcon("Application icon response reported another edge.");
        string state = IconState(result);
        if (state == "running") return null;
        if (state != "ready") throw InvalidIcon("Application icon request returned an unknown state.");
        if (ParseIconIdentity(result) != identity)
            throw InvalidIcon("Application icon response reported another identity.");
        int width = IconInt32(result, "width");
        int height = IconInt32(result, "height");
        if (width < 1 || height < 1 || width > edge || height > edge)
            throw InvalidIcon("Application icon raster exceeds the requested edge.");
        long expected = (long)width * height * 4;
        if (expected > MaxIconBytes)
            throw InvalidIcon("Application icon raster exceeds the bounded raster contract.");
        return new(job, catalogJobId, identity, edge, width, height, ParseIconPixels(result, (int)expected));
    }

    // The native contract publishes a JSON numeric array. A base64 string (System.Text.Json's
    // byte[] default) or any non-byte member is rejected rather than decoded.
    private static byte[] ParseIconPixels(JsonElement result, int length)
    {
        if (!TryIconProperty(result, "pixels", out var pixels) || pixels.ValueKind != JsonValueKind.Array)
            throw InvalidIcon("Application icon raster is not a JSON numeric array.");
        if (pixels.GetArrayLength() != length)
            throw InvalidIcon("Application icon raster byte count is inconsistent with its dimensions.");
        var bytes = new byte[length];
        int index = 0;
        foreach (var item in pixels.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Number || !item.TryGetInt32(out int value) || value is < 0 or > 255)
                throw InvalidIcon("Application icon raster contains a value outside a byte.");
            bytes[index++] = (byte)value;
        }
        return bytes;
    }

    private static ApplicationIconIdentity ParseIconIdentity(JsonElement result)
    {
        if (!TryIconProperty(result, "identity", out var identity) || identity.ValueKind != JsonValueKind.Object)
            throw InvalidIcon("Application icon response has no catalog identity.");
        string? sourceId = IconOptionalString(identity, "source_id");
        string? kind = IconOptionalString(identity, "kind");
        if (sourceId is null || string.IsNullOrEmpty(kind))
            throw InvalidIcon("Application icon response identity is incomplete.");
        return new(sourceId, IconOptionalString(identity, "handler"), kind, IconOptionalString(identity, "unavailable"));
    }

    private static ulong IconJobId(JsonElement result) =>
        TryIconProperty(result, "job_id", out var value) && value.ValueKind == JsonValueKind.Number
            && value.TryGetUInt64(out ulong job) ? job : 0;

    private static ulong IconUInt64(JsonElement result, string name)
    {
        if (!TryIconProperty(result, name, out var value) || value.ValueKind != JsonValueKind.Number
            || !value.TryGetUInt64(out ulong parsed))
            throw InvalidIcon($"Application icon response field '{name}' is not an unsigned integer.");
        return parsed;
    }

    private static int IconInt32(JsonElement result, string name)
    {
        if (!TryIconProperty(result, name, out var value) || value.ValueKind != JsonValueKind.Number
            || !value.TryGetInt32(out int parsed))
            throw InvalidIcon($"Application icon response field '{name}' is not an integer.");
        return parsed;
    }

    private static string IconState(JsonElement result) =>
        IconOptionalString(result, "state") is { Length: > 0 } state
            ? state
            : throw InvalidIcon("Application icon response has no state.");

    private static string? IconOptionalString(JsonElement element, string name)
    {
        if (!TryIconProperty(element, name, out var value)) return null;
        return value.ValueKind switch
        {
            JsonValueKind.Null => null,
            JsonValueKind.String => value.GetString(),
            _ => throw InvalidIcon($"Application icon response field '{name}' is not a string."),
        };
    }

    // JsonElement.TryGetProperty throws for null/array/scalar/undefined receivers.
    // Missing or malformed icon fields must keep the invalid_response contract instead.
    private static bool TryIconProperty(JsonElement element, string name, out JsonElement value)
    {
        value = default;
        return element.ValueKind == JsonValueKind.Object && element.TryGetProperty(name, out value);
    }

    private static CoreException InvalidIcon(string message) => new("invalid_response", message);

    // Managed drain: only requests this client admitted. The native close still joins the real
    // Shell workers; the handle is retained until every admitted task has settled, so a queued
    // request can never touch a released handle and no cancellation path needs the owner
    // dispatcher or the request gate.
    private void BeginIconDrain()
    {
        if (Volatile.Read(ref trackedIcons) == 0) iconDrained.TrySetResult();
    }

    private void WaitIconDrain()
    {
        BeginIconDrain();
        iconDrained.Task.GetAwaiter().GetResult();
    }

    private void IconSettled()
    {
        if (Interlocked.Decrement(ref trackedIcons) == 0 && shutdown.IsCancellationRequested) iconDrained.TrySetResult();
    }
}

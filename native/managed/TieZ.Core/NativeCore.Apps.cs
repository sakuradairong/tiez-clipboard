using System.Text.Json;

namespace TieZ.Core;

public sealed record InstalledApplication(string Name, string SourceId, string? Handler, string Kind, string? Unavailable);
public sealed record RecommendedApplication(InstalledApplication Application, string Reason);
public sealed record ApplicationRecommendations(IReadOnlyList<RecommendedApplication> Applications, string? AssociationError,
    int Unreadable, bool Limited, string? DefaultName);
public sealed record ApplicationCatalog(IReadOnlyList<InstalledApplication> Applications, int Scanned, int Unreadable,
    int Duplicates, bool Limited, IReadOnlyDictionary<string, ApplicationRecommendations> Recommendations);
public sealed record ApplicationCatalogResult(ulong JobId, bool Cached, ApplicationCatalog Snapshot);

public sealed partial class NativeCore
{
    private readonly SemaphoreSlim appsGate = new(1, 1);
    private sealed record AppsStarted(ulong JobId, bool Cached);
    private sealed record AppsPolled(ulong JobId, string State, bool Cached, ApplicationCatalog? Snapshot);

    // The owner starts this call. Polling and native cancellation never need its dispatcher.
    public async Task<ApplicationCatalogResult> GetApplicationCatalogAsync(bool refresh = false, CancellationToken cancellationToken = default)
    {
        VerifyAccess();
        ulong ownedHandle = handle;
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, shutdown.Token);
        await appsGate.WaitAsync(cancellation.Token).ConfigureAwait(false);
        ulong job = 0; bool completed = false;
        try
        {
            var started = await AppsRequestAsync<AppsStarted>(new { op = "apps_start", handle = ownedHandle, refresh }, cancellation.Token).ConfigureAwait(false);
            job = started.JobId;
            while (true)
            {
                cancellation.Token.ThrowIfCancellationRequested();
                var result = await AppsRequestAsync<AppsPolled>(new { op = "apps_poll", handle = ownedHandle, job_id = job }, cancellation.Token).ConfigureAwait(false);
                cancellation.Token.ThrowIfCancellationRequested();
                if (result.JobId != job) throw new CoreException("invalid_response", "Application scan identity changed.");
                if (result.State == "ready" && result.Snapshot is not null)
                {
                    completed = true;
                    return new(job, result.Cached, result.Snapshot);
                }
                if (result.State != "running") throw new CoreException("invalid_response", "Application scan returned an unknown state.");
                await Task.Delay(60, cancellation.Token).ConfigureAwait(false);
            }
        }
        finally
        {
            try
            {
                if (job != 0 && !completed)
                {
                    try { await AppsRequestAsync<JsonElement>(new { op = "apps_cancel", handle = ownedHandle, job_id = job }, CancellationToken.None).ConfigureAwait(false); }
                    catch (CoreException failure) when (failure.Code is "invalid_handle" or "apps_job" or "apps_closed" or "core_closing") { }
                }
            }
            finally { appsGate.Release(); }
        }
    }
    private async Task<T> AppsRequestAsync<T>(object request, CancellationToken cancellationToken)
    {
        byte[] bytes = SerializeRequest(request);
        await requestGate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            return await Task.Run(() =>
            {
                cancellationToken.ThrowIfCancellationRequested();
                // Preserve an accepted job ID so finally can cancel it even after caller cancellation.
                return RequestBytes(bytes).Deserialize<T>(WireJson)
                    ?? throw new CoreException("invalid_response", "Native application catalog returned an empty result.");
            }).ConfigureAwait(false);
        }
        finally { requestGate.Release(); }
    }
}

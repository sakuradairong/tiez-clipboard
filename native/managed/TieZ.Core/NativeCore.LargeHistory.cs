using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace TieZ.Core;

public sealed partial class NativeCore
{
    private const int MaxLargeHistoryBytes = 16 * 1024 * 1024;
    private const int MaxLargeHistoryAdmitted = 4;
    private readonly SemaphoreSlim largeHistoryGate = new(1, 1);
    private readonly TaskCompletionSource largeHistoryDrained = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private int trackedLargeHistory;
    // Owned real-DLL smoke can delay copied metadata, never replace native results.
    private Func<string, JsonElement, Task>? largeHistoryObserved = null;
    private sealed record LargeHistoryLimits(int MaxPayloadBytes, int MaxChunkBytes, ulong Revision);
    private sealed record LargeHistoryPayload(string Content, string? ContentType, string? HtmlContent, string[] Tags);

    private static bool NeedsHistoryStaging(string content, string? html = null, string? type = null, string[]? tags = null)
    {
        long characters = (long)content.Length + (html?.Length ?? 0) + (type?.Length ?? 0);
        if (characters > MaxLargeHistoryBytes) throw new CoreException("transfer_bounds", "History input exceeds the staging limit; the draft was not truncated.");
        foreach (string tag in tags ?? [])
        {
            characters += tag?.Length ?? 0;
            if (characters > MaxLargeHistoryBytes) throw new CoreException("transfer_bounds", "History input exceeds the staging limit; the draft was not truncated.");
        }
        // Six bytes per UTF-16 code unit bounds JSON escaping, including control characters.
        return characters > 10_000 || characters * 6 + (tags?.Length ?? 0) * 4L + 1024 > 65536;
    }
    private Task<HistoryMutation> LargeHistoryMutationAsync(LargeHistoryPayload payload, long? editId, CancellationToken token)
    {
        VerifyAccess(); token.ThrowIfCancellationRequested();
        if (Interlocked.Increment(ref trackedLargeHistory) > MaxLargeHistoryAdmitted)
        {
            Interlocked.Decrement(ref trackedLargeHistory);
            throw new CoreException("transfer_capacity", "Too many large history requests are outstanding.");
        }
        ulong ownedHandle = handle;
        return Task.Run(() => RunLargeHistoryMutationAsync(payload, editId, ownedHandle, token));
    }
    private sealed class HistoryPayloadBuffer(int maximum, CancellationToken token) : MemoryStream
    {
        private void EnsureWrite(int count)
        {
            token.ThrowIfCancellationRequested();
            long required = Position + count;
            if (required > maximum) throw new CoreException("transfer_bounds", "History payload exceeds the negotiated staging limit; the draft was not truncated.");
            if (required > Capacity) Capacity = (int)Math.Min(maximum, Math.Max(required, Math.Max(4096L, Capacity * 2L)));
        }
        public override void Write(byte[] buffer, int offset, int count) { EnsureWrite(count); base.Write(buffer, offset, count); }
        public override void Write(ReadOnlySpan<byte> buffer) { EnsureWrite(buffer.Length); base.Write(buffer); }
        public override void SetLength(long value)
        {
            if (value < 0 || value > maximum) throw new CoreException("transfer_bounds", "History payload exceeds its staging limit.");
            if (value > Capacity) Capacity = (int)value;
            base.SetLength(value);
        }
        protected override void Dispose(bool disposing)
        {
            if (disposing) CryptographicOperations.ZeroMemory(GetBuffer());
            base.Dispose(disposing);
        }
    }
    private static void WriteHistoryString(Utf8JsonWriter writer, string? text)
    {
        if (text is null) { writer.WriteNullValue(); writer.Flush(); return; }
        if (text.Length == 0) { writer.WriteStringValue(""); writer.Flush(); return; }
        int offset = 0;
        while (offset < text.Length)
        {
            int end = Math.Min(offset + 512, text.Length);
            if (end < text.Length && char.IsHighSurrogate(text[end - 1]) && char.IsLowSurrogate(text[end])) end--;
            writer.WriteStringValueSegment(text.AsSpan(offset, end - offset), end == text.Length);
            // Bound the JSON encoder's temporary buffer too, not just its output stream.
            writer.Flush(); offset = end;
        }
    }
    private static HistoryPayloadBuffer SerializeHistoryPayload(LargeHistoryPayload payload, int maximum, CancellationToken token)
    {
        var buffer = new HistoryPayloadBuffer(maximum, token);
        try
        {
            using (var writer = new Utf8JsonWriter(buffer, new JsonWriterOptions { Encoder = WireJson.Encoder }))
            {
                writer.WriteStartObject();
                if (payload.ContentType is not null)
                {
                    writer.WritePropertyName("content_type"); WriteHistoryString(writer, payload.ContentType);
                }
                writer.WritePropertyName("content"); WriteHistoryString(writer, payload.Content);
                if (payload.ContentType is not null)
                {
                    writer.WritePropertyName("html_content"); WriteHistoryString(writer, payload.HtmlContent);
                    writer.WriteStartArray("tags");
                    foreach (string tag in payload.Tags) WriteHistoryString(writer, tag);
                    writer.WriteEndArray();
                }
                writer.WriteEndObject(); writer.Flush();
            }
            token.ThrowIfCancellationRequested(); return buffer;
        }
        catch { buffer.Dispose(); throw; }
    }
    private static int HistoryChunkEnd(ReadOnlySpan<byte> bytes, int offset, int maximum)
    {
        int end = Math.Min(bytes.Length, offset + maximum);
        while (end < bytes.Length && end > offset && (bytes[end] & 0xC0) == 0x80) end--;
        if (end <= offset) throw new CoreException("invalid_response", "Native chunk limit cannot hold a UTF-8 code point.");
        return end;
    }
    private static CoreException InvalidHistoryTransfer() => new("invalid_response", "Native history transfer response is invalid; refresh authoritative history before retrying a submitted mutation.");
    private static void TransferObject(JsonElement result)
    {
        if (result.ValueKind != JsonValueKind.Object || result.GetRawText().Length > 4096) throw InvalidHistoryTransfer();
    }
    private static ulong TransferNumber(JsonElement result, string name)
    {
        if (result.ValueKind != JsonValueKind.Object || !result.TryGetProperty(name, out var value)
            || value.ValueKind != JsonValueKind.Number || !value.TryGetUInt64(out ulong number)) throw InvalidHistoryTransfer();
        return number;
    }
    private static bool TransferBoolean(JsonElement result, string name)
    {
        if (result.ValueKind != JsonValueKind.Object || !result.TryGetProperty(name, out var value)
            || value.ValueKind is not (JsonValueKind.True or JsonValueKind.False)) throw InvalidHistoryTransfer();
        return value.GetBoolean();
    }
    private static string? RecoverHistoryUploadToken(JsonElement result)
    {
        if (result.ValueKind != JsonValueKind.Object || !result.TryGetProperty("token", out var value)
            || value.ValueKind != JsonValueKind.String) return null;
        string token = value.GetString()!;
        return token.Length == 32 && token.All(char.IsAsciiHexDigit) ? token : null;
    }
    private static LargeHistoryLimits ParseHistoryUploadLimits(JsonElement result)
    {
        TransferObject(result);
        if (TransferNumber(result, "version") != 1) throw new CoreException("transfer_version", "Unsupported native history transfer protocol.");
        ulong payload = TransferNumber(result, "max_payload_bytes"), reserved = TransferNumber(result, "max_reserved_bytes"),
            chunk = TransferNumber(result, "max_chunk_bytes"), uploads = TransferNumber(result, "max_uploads"), idle = TransferNumber(result, "idle_seconds");
        if (payload is < 1 or > MaxLargeHistoryBytes || reserved < payload || reserved > 2 * MaxLargeHistoryBytes
            || chunk is < 4 or > 4096 || uploads is < 1 or > 2 || idle is < 1 or > 3600) throw InvalidHistoryTransfer();
        return new((int)payload, (int)chunk, TransferNumber(result, "history_revision"));
    }
    private static void ValidateHistoryUploadStatus(JsonElement result, string token, int total, int received, ulong revision)
    {
        TransferObject(result);
        if (RecoverHistoryUploadToken(result) != token || TransferNumber(result, "total_bytes") != (ulong)total
            || TransferNumber(result, "received_bytes") != (ulong)received || TransferNumber(result, "expected_revision") != revision)
            throw InvalidHistoryTransfer();
    }
    private static HistoryMutation ReadLargeHistoryReceipt(JsonElement result, long? editId, ulong revision)
    {
        TransferObject(result);
        ulong accepted = TransferNumber(result, "revision"); bool changed = TransferBoolean(result, "changed");
        if (changed ? accepted <= revision || accepted - revision != 1 : accepted != revision) throw InvalidHistoryTransfer();
        if (!result.TryGetProperty("id", out var idValue) || idValue.ValueKind != JsonValueKind.Number || !idValue.TryGetInt64(out long id)
            || (editId is { } expected ? id != expected : id <= 0)) throw InvalidHistoryTransfer();
        if (!result.TryGetProperty("replaced_id", out var replaced) || replaced.ValueKind != JsonValueKind.Null
            || !result.TryGetProperty("removed_ids", out var removed) || removed.ValueKind != JsonValueKind.Array || removed.GetArrayLength() != 0)
            throw InvalidHistoryTransfer();
        return new(accepted, changed, id, null, [], TransferBoolean(result, "persistent_sync_pending"));
    }
    private async Task<JsonElement> SendHistoryTransferAsync(object request, CancellationToken token, bool cleanup = false)
    {
        byte[] bytes = SerializeRequest(request); bool acquired = false;
        var observer = largeHistoryObserved; string? operation = null; JsonElement result;
        try
        {
            if (observer is not null)
            {
                using var document = JsonDocument.Parse(bytes);
                operation = document.RootElement.GetProperty("op").GetString();
            }
            await requestGate.WaitAsync(token).ConfigureAwait(false); acquired = true;
            ObjectDisposedException.ThrowIf(handle == 0, this);
            result = await Task.Run(() =>
            {
                if (!cleanup) shutdown.Token.ThrowIfCancellationRequested();
                token.ThrowIfCancellationRequested();
                // Once submitted, copy/free and preserve the actual result after cancellation.
                return RequestBytes(bytes);
            }).ConfigureAwait(false);
        }
        finally { if (acquired) requestGate.Release(); CryptographicOperations.ZeroMemory(bytes); }
        if (observer is not null) await observer(operation!, result).ConfigureAwait(false);
        return result;
    }
    private async Task<HistoryMutation> RunLargeHistoryMutationAsync(LargeHistoryPayload payload, long? editId, ulong ownedHandle, CancellationToken callerToken)
    {
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(callerToken, shutdown.Token);
        string? resource = null; bool acquired = false, completed = false; Exception? primary = null;
        HistoryPayloadBuffer? buffer = null;
        try
        {
            await largeHistoryGate.WaitAsync(cancellation.Token).ConfigureAwait(false); acquired = true;
            var limits = ParseHistoryUploadLimits(await SendHistoryTransferAsync(new { op = "history_upload_limits", handle = ownedHandle }, cancellation.Token).ConfigureAwait(false));
            buffer = SerializeHistoryPayload(payload, limits.MaxPayloadBytes, cancellation.Token);
            int length = checked((int)buffer.Length);
            string checksum = Convert.ToHexString(SHA256.HashData(buffer.GetBuffer().AsSpan(0, length)));
            cancellation.Token.ThrowIfCancellationRequested();
            object target = editId is { } id ? new { kind = "edit", id, allow_sensitive = true } : new { kind = "add" };
            var begun = await SendHistoryTransferAsync(new { op = "history_upload_begin", handle = ownedHandle,
                version = 1, target, expected_revision = limits.Revision, total_bytes = length, sha256 = checksum }, cancellation.Token).ConfigureAwait(false);
            // Retain the accepted token before any recheck, including cancellation during begin.
            resource = RecoverHistoryUploadToken(begun);
            if (resource is null) throw InvalidHistoryTransfer();
            ValidateHistoryUploadStatus(begun, resource, length, 0, limits.Revision);
            int offset = 0;
            while (offset < length)
            {
                cancellation.Token.ThrowIfCancellationRequested();
                int end = HistoryChunkEnd(buffer.GetBuffer().AsSpan(0, length), offset, limits.MaxChunkBytes);
                string data = Encoding.UTF8.GetString(buffer.GetBuffer(), offset, end - offset);
                var acknowledged = await SendHistoryTransferAsync(new { op = "history_upload_append", handle = ownedHandle,
                    token = resource, offset, data }, cancellation.Token).ConfigureAwait(false);
                ValidateHistoryUploadStatus(acknowledged, resource, length, end, limits.Revision);
                offset = end;
            }
            cancellation.Token.ThrowIfCancellationRequested();
            var accepted = await SendHistoryTransferAsync(new { op = "history_upload_commit", handle = ownedHandle, token = resource }, cancellation.Token).ConfigureAwait(false);
            // No token recheck after submission: preserve the real committed/no-op result.
            var mutation = ReadLargeHistoryReceipt(accepted, editId, limits.Revision);
            completed = true;
            return mutation;
        }
        catch (Exception failure) { primary = failure; throw; }
        finally
        {
            try
            {
                if (resource is not null && !completed)
                {
                    try
                    {
                        var aborted = await SendHistoryTransferAsync(new { op = "history_upload_abort", handle = ownedHandle, token = resource }, CancellationToken.None, cleanup: true).ConfigureAwait(false);
                        TransferObject(aborted); _ = TransferBoolean(aborted, "aborted");
                    }
                    catch (CoreException failure) when (failure.Code is "invalid_handle" or "core_closing" or "transfer_missing") { }
                    catch (Exception cleanup)
                    {
                        if (primary is null) throw;
                        throw new AggregateException("History transfer and staging cleanup failed; refresh authoritative history before retrying.", primary, cleanup);
                    }
                }
            }
            finally
            {
                buffer?.Dispose();
                if (acquired) largeHistoryGate.Release();
                if (Interlocked.Decrement(ref trackedLargeHistory) == 0 && shutdown.IsCancellationRequested) largeHistoryDrained.TrySetResult();
            }
        }
    }
    private void BeginLargeHistoryDrain()
    {
        if (Volatile.Read(ref trackedLargeHistory) == 0) largeHistoryDrained.TrySetResult();
    }
    private void WaitLargeHistoryDrain()
    {
        BeginLargeHistoryDrain(); largeHistoryDrained.Task.GetAwaiter().GetResult();
    }
}

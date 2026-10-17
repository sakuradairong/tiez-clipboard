using TieZ.Core;

internal static class TagSelectionSmoke
{
    private static T Read<T>(Task<T> task) => task.GetAwaiter().GetResult();
    private static void Check(bool value,string message) { if(!value)throw new InvalidOperationException(message); }
    private static void Expect(string code,Action action)
    {
        try{action();}catch(CoreException error)when(error.Code==code){return;}
        throw new InvalidOperationException($"Expected {code}");
    }
    internal static void Run(NativeCore core)
    {
        const string raw="  目录 ' %_  "; const string body="  selection\r\n "; const string html="<b>selection</b>";
        ulong settingsRevision=core.Snapshot.Revision;
        var added=Read(core.AddManualAsync(body,"rich_text",html));
        var snapshot=Read(core.GetTagSelectionAsync(added.Id));
        Check(snapshot.Id==added.Id&&snapshot.Revision==added.Revision&&snapshot.Tags.Count==0,"Tag selection snapshot DTO");
        var catalog=Read(core.SetTagColorAsync(raw,"#123456",snapshot.Revision));
        Expect("history_conflict",()=>Read(core.SetExactTagsAsync(added.Id,new[]{raw},snapshot.Revision)));
        snapshot=Read(core.GetTagSelectionAsync(added.Id));
        Check(snapshot.Revision==catalog.Revision,"Selection read used a stale catalog version");
        var tagged=Read(core.SetExactTagsAsync(added.Id,new[]{raw,"PASSWORD",raw},snapshot.Revision));
        var sensitive=Read(core.GetTagSelectionAsync(added.Id));
        Check(tagged.Changed&&tagged.Revision==snapshot.Revision+1&&sensitive.Tags.SequenceEqual(new[]{raw,"PASSWORD"}),"Exact tag names/deduplication");
        Expect("sensitive_confirmation",()=>Read(core.GetPreviewEntryAsync(added.Id,sensitive.Revision,false)));
        Check(!Read(core.SetExactTagsAsync(added.Id,new[]{"PASSWORD",raw},sensitive.Revision)).Changed,"Same tag set rewrote ordering or advanced revision");
        Check(Read(core.GetTagSelectionAsync(added.Id)).Tags.SequenceEqual(sensitive.Tags),"Semantic no-op changed original order");
        using var canceled=new CancellationTokenSource();canceled.Cancel();
        try{Read(core.SetExactTagsAsync(added.Id,Array.Empty<string>(),sensitive.Revision,canceled.Token));throw new InvalidOperationException("Canceled selection saved");}catch(OperationCanceledException){}
        Expect("invalid_tag",()=>Read(core.SetExactTagsAsync(added.Id,new[]{" "},sensitive.Revision)));
        Check(Read(core.GetTagSelectionAsync(added.Id)).Revision==sensitive.Revision,"Canceled/invalid tag selection wrote state");
        var ordinary=Read(core.SetExactTagsAsync(added.Id,new[]{raw,"new picker tag"},sensitive.Revision));
        var entry=Read(core.GetPreviewEntryAsync(added.Id,ordinary.Revision,false));
        Check(entry.Content==body&&entry.HtmlContent==html&&entry.Tags.Contains(raw),"Exact tag transition lost rich content/CRLF/legacy name");
        var tagPage=Read(core.GetTagsAsync(new TagQuery(Search:raw)));
        Check(tagPage.Items.Single(tag=>tag.Name==raw).Color=="#123456","Exact selection changed catalog spelling/color");
        Check(core.Snapshot.Revision==settingsRevision,"Tag selection changed settings revision");
        var deleted=Read(core.DeleteEntryAsync(added.Id));
        Expect("not_found",()=>Read(core.GetTagSelectionAsync(added.Id)));
        Expect("not_found",()=>Read(core.SetExactTagsAsync(added.Id,Array.Empty<string>(),deleted.Revision)));
        Console.WriteLine("PASS: real DLL tag-selection metadata, exact legacy names, sensitive transitions, no-op/stale/cancel/invalid guards and original rich CRLF");
    }
}

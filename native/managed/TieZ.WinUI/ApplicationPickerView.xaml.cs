using System.Collections.ObjectModel;
using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public sealed class ApplicationChoice : INotifyPropertyChanged
{
    private readonly string contentType="",kind="",identity="",unavailable="";
    internal InstalledApplication? Application { get; }
    internal string? Value { get; }
    internal string Group { get; }
    public string Name { get; private set; } = "";
    public string GroupLabel { get; private set; } = "";
    public string Details { get; private set; } = "";
    // Searchable text: display name, launch target/path and the opaque catalog identity.
    internal string SearchText { get; private set; } = "";
    public string AccessibilityLabel=>string.Format(AppPickerStrings.Current.AccessibilityFormat,Name,GroupLabel,Details);
    public bool CanSelect { get; }
    public event PropertyChangedEventHandler? PropertyChanged;
    private ApplicationChoice(string contentType)
    {this.contentType=contentType;Group="default";Value="";CanSelect=true;Relocalize();}
    internal ApplicationChoice(InstalledApplication app,string reason)
    {
        Application=app;Name=app.Name;Value=app.Handler;
        CanSelect=app.Handler is {Length:>0}&&app.Unavailable is null&&app.Kind is "desktop" or "packaged";
        Group=CanSelect?reason:"unavailable";
        kind=app.Kind;identity=app.Handler??app.SourceId;unavailable=app.Unavailable??"";
        SearchText=$"{app.Name}\n{app.Handler}\n{app.SourceId}\n{app.Kind}";
        Relocalize();
    }
    internal static ApplicationChoice DefaultRule(string contentType)=>new(contentType);
    // Row labels are re-derived from data already in memory, never from a new catalog read.
    internal void Relocalize()
    {
        var text=AppPickerStrings.Current;
        if(Application is null)
        {
            bool code=contentType=="code";
            Name=code?text.FollowTextApp:text.DefaultApp;
            GroupLabel=text.DefaultRowGroup;
            Details=code?text.DefaultRowDetailsCode:text.DefaultRowDetailsOther;
            SearchText=$"{Name}\n{Details}\n{GroupLabel}";
        }
        else
        {
            string type=kind=="desktop"?text.KindDesktop:kind=="packaged"?text.KindPackaged:text.KindUnavailable;
            GroupLabel=Group switch
            {
                "association"=>string.Format(text.GroupAssociationFormat,type),
                "keyword"=>string.Format(text.GroupKeywordFormat,type),
                "other"=>string.Format(text.GroupOtherFormat,type),
                _=>text.GroupUnavailable
            };
            string reason=unavailable switch
            {
                "missing_target"=>text.UnavailableMissingTarget,
                "invalid_identity"=>text.UnavailableInvalidIdentity,
                "not_native_executable"=>text.UnavailableNotNative,
                _=>text.UnavailableGeneric
            };
            Details=CanSelect?identity:reason+text.DetailSeparator+identity;
        }
        Notify(nameof(Name));Notify(nameof(GroupLabel));Notify(nameof(Details));Notify(nameof(AccessibilityLabel));
    }
    private void Notify(string property)=>PropertyChanged?.Invoke(this,new PropertyChangedEventArgs(property));
}

// A local selection draft over a shared core snapshot. No settings update occurs here.
public sealed partial class ApplicationPickerView : UserControl, IDisposable
{
    private readonly ObservableCollection<ApplicationChoice> visible=[];
    private readonly List<ApplicationChoice> all=[];
    private Func<bool,CancellationToken,Task<ApplicationCatalogResult>>? reader;
    private CancellationTokenSource? lifetime,loadingCancellation;
    private ApplicationCatalogResult? accepted;
    private string contentType="",current="";
    private Func<AppPickerStrings,string>? feedbackText;
    private bool disposed,loading,stale,failed,binding;
    private int generation;
    internal event Action? StateChanged;
    internal bool CanChoose=>!disposed&&!loading&&!stale&&!failed&&accepted is not null&&lifetime is {IsCancellationRequested:false}&&Choices.SelectedItem is ApplicationChoice {CanSelect:true};
    internal bool IsLoading=>loading;
    internal bool IsStale=>stale;
    internal bool HasError=>Feedback.IsOpen;
    internal bool HasRetainedData=>HasIconState||reader is not null||accepted is not null||all.Count>0||visible.Count>0||current.Length>0||SearchText.Text.Length>0||CurrentText.Text.Length>0||SelectionText.Text.Length>0;
    // Narrow read-only state for the owned acceptance smoke; no behavior is added here.
    internal bool IsDisposed=>disposed;
    internal bool IsFailed=>failed;
    internal string FeedbackMessage=>Feedback.Message;
    internal string DefaultTextValue=>DefaultText.Text;
    internal string? SelectedValue=>CanChoose?(Choices.SelectedItem as ApplicationChoice)?.Value:null;
    internal IReadOnlyList<ApplicationChoice> VisibleChoices=>visible.ToArray();
    internal int RealizedCount=>Enumerable.Range(0,visible.Count).Count(i=>Choices.ContainerFromIndex(i) is not null);
    internal string SearchDraft {get=>SearchText.Text;set=>SearchText.Text=value;}
    internal int Filter {get=>Category.SelectedIndex;set=>Category.SelectedIndex=value;}
    public ApplicationPickerView()
    {
        InitializeComponent();Choices.ItemsSource=visible;InitializeIcons();
        SearchText.TextChanging+=(_,_)=>ApplyFilter();Category.SelectionChanged+=(_,_)=>ApplyFilter();
        Choices.SelectionChanged+=(_,_)=>{if(!binding&&!disposed){ShowSelection();StateChanged?.Invoke();}};
        RefreshButton.Click+=async(_,_)=>await LoadAsync(true);
        ApplyLanguage();
    }
    internal ApplicationPickerView(string contentType,string current,Func<bool,CancellationToken,Task<ApplicationCatalogResult>> reader,CancellationToken cancellationToken):this()
    {
        this.contentType=contentType;this.current=current;this.reader=reader;
        lifetime=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        RenderCurrent();
    }
    private static string Identity(InstalledApplication app)=>app.Handler is { } handler?$"{app.Kind}:{(app.Kind=="desktop"?handler.ToLowerInvariant():handler)}":$"unavailable:{app.SourceId}:{app.Name}";
    private List<ApplicationChoice> Build(ApplicationCatalogResult result)
    {
        if(!result.Snapshot.Recommendations.TryGetValue(contentType,out var recommendations))throw new CoreException("invalid_response","Application recommendations missing");
        var choices=new List<ApplicationChoice> {ApplicationChoice.DefaultRule(contentType)};
        var seen=new HashSet<string>(StringComparer.Ordinal);
        foreach(var item in recommendations.Applications)
            if(seen.Add(Identity(item.Application)))choices.Add(new(item.Application,item.Reason));
        foreach(var item in result.Snapshot.Applications.OrderBy(app=>app.Name,StringComparer.CurrentCultureIgnoreCase))
            if(seen.Add(Identity(item)))choices.Add(new(item,"other"));
        return choices.OrderBy(choice=>choice.Group switch {"default"=>0,"association"=>1,"keyword"=>2,"other"=>3,_=>4}).ToList();
    }
    internal async Task LoadAsync(bool refresh)
    {
        if(disposed||stale||loading||reader is null||lifetime is null||lifetime.IsCancellationRequested)return;
        var cancellation=CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);loadingCancellation=cancellation;
        ClearIcons();int version=++generation;loading=true;failed=false;Feedback.IsOpen=false;UpdateControls();
        try
        {
            var result=await reader(refresh,cancellation.Token).WaitAsync(cancellation.Token);
            if(disposed||stale||version!=generation||cancellation.IsCancellationRequested)return;
            var choices=Build(result);string? selected=(Choices.SelectedItem as ApplicationChoice)?.Value??current;
            accepted=result;all.Clear();all.AddRange(choices);
            RenderDefaultText(result);
            ApplyFilter(selected);
        }
        catch(OperationCanceledException) { }
        catch(Exception failure)
        {
            if(!disposed&&!stale&&version==generation)
            {
                failed=true;ShowFeedback(InfoBarSeverity.Error,failure is CoreException {Code:"apps_busy"}
                    ?text=>text.Busy:text=>text.LoadFailed);
            }
        }
        finally
        {
            if(ReferenceEquals(loadingCancellation,cancellation))loadingCancellation=null;cancellation.Dispose();
            if(!disposed&&version==generation){loading=false;UpdateControls();}
        }
    }
    private void ApplyFilter()=>ApplyFilter((Choices.SelectedItem as ApplicationChoice)?.Value);
    private void ApplyFilter(string? selected)
    {
        if(disposed)return;
        string query=SearchText.Text;int filter=Category.SelectedIndex;
        binding=true;
        try
        {
            ClearIcons();visible.Clear();
            foreach(var choice in all)
                if((filter<=0||filter==1&&choice.Group is "association" or "keyword"||filter==2&&choice.Group=="other"||filter==3&&choice.Group=="unavailable")
                    &&(query.Length==0||choice.SearchText.Contains(query,StringComparison.OrdinalIgnoreCase)))visible.Add(choice);
            Choices.SelectedItem=selected is null?null:visible.FirstOrDefault(choice=>choice.CanSelect&&choice.Value==selected);
        }
        finally{binding=false;}
        ShowSelection();UpdateControls();
    }
    private void ShowSelection()
    {
        var text=AppPickerStrings.Current;
        SelectionText.Text=Choices.SelectedItem is ApplicationChoice choice
            ?string.Format(choice.CanSelect?text.SelectionSelectedFormat:text.SelectionUnavailableFormat,choice.Name,choice.Details)
            :text.SelectionNone;
    }
    private void ShowFeedback(InfoBarSeverity severity,Func<AppPickerStrings,string> message)
    {
        feedbackText=message;Feedback.Severity=severity;Feedback.Message=message(AppPickerStrings.Current);Feedback.IsOpen=true;
    }
    private static string DefaultLabel(string contentType)=>contentType=="code"?AppPickerStrings.Current.FollowTextApp:AppPickerStrings.Current.DefaultApp;
    private void RenderCurrent()
    {
        if(contentType.Length>0)CurrentText.Text=string.Format(AppPickerStrings.Current.CurrentDraftFormat,current.Length==0?DefaultLabel(contentType):current);
    }
    private void RenderDefaultText(ApplicationCatalogResult result)
    {
        if(!result.Snapshot.Recommendations.TryGetValue(contentType,out var info))return;
        var text=AppPickerStrings.Current;
        DefaultText.Text=contentType=="code"?text.DefaultSummaryCode:info.DefaultName is {Length:>0} name?string.Format(text.DefaultSummaryFormat,name):text.DefaultSummaryUnknown;
    }
    // Re-derives every visible string from the tables; accepted catalog rows, search text and selection stay in place.
    internal void ApplyLanguage()
    {
        if(disposed)return;
        var text=AppPickerStrings.Current;
        SearchText.PlaceholderText=text.SearchPlaceholder;
        AutomationProperties.SetName(SearchText,text.SearchName);
        RefreshButton.Content=text.Refresh;
        AutomationProperties.SetName(RefreshButton,text.Refresh);
        AutomationProperties.SetName(Category,text.CategoryName);
        CategoryAll.Content=text.CategoryAll;CategoryRecommended.Content=text.CategoryRecommended;
        CategoryOther.Content=text.CategoryOther;CategoryUnavailable.Content=text.CategoryUnavailable;
        AutomationProperties.SetName(Choices,text.ChoicesName);
        Footer.Text=text.Footer;
        foreach(var choice in all)choice.Relocalize();
        foreach(var icon in IconViews)icon.ApplyLanguage();
        if(Feedback.IsOpen&&feedbackText is { } message)Feedback.Message=message(text);
        if(all.Count>0)ShowSelection();
        if(accepted is not null||loading||failed)UpdateControls();
        RenderCurrent();
    }
    private void UpdateControls()
    {
        if(disposed)return;
        Progress.Visibility=loading?Visibility.Visible:Visibility.Collapsed;Choices.IsEnabled=!loading&&!stale;RefreshButton.IsEnabled=!loading&&!stale&&lifetime is {IsCancellationRequested:false};
        var text=AppPickerStrings.Current;
        if(loading)Summary.Text=text.LoadingSummary;
        else if(accepted is { } result)
        {
            var info=result.Snapshot.Recommendations[contentType];int unavailable=all.Count(choice=>!choice.CanSelect);
            bool incomplete=result.Snapshot.Limited||info.Limited||result.Snapshot.Unreadable>0||info.Unreadable>0||info.AssociationError is not null;
            Summary.Text=string.Format(text.SummaryFormat,visible.Count,unavailable)
                +(result.Cached?text.SummaryCached:"")+(incomplete?text.SummaryIncomplete:"");
        }
        else Summary.Text=failed?text.SummaryFailed:"";
        QueueIconRefresh();StateChanged?.Invoke();
    }
    internal void Invalidate()
    {
        if(disposed)return;stale=true;ClearIcons();generation++;loadingCancellation?.Cancel();loading=false;Choices.SelectedItem=null;
        ShowFeedback(InfoBarSeverity.Warning,text=>text.Invalidated);UpdateControls();
    }
    public void Dispose()
    {
        if(disposed)return;disposed=true;ReleaseIcons();generation++;loadingCancellation?.Cancel();lifetime?.Cancel();lifetime?.Dispose();lifetime=null;
        reader=null;accepted=null;Choices.SelectedItem=null;Choices.ItemsSource=null;all.Clear();visible.Clear();
        SearchText.Text="";CurrentText.Text=SelectionText.Text=Summary.Text=DefaultText.Text="";Feedback.IsOpen=false;Feedback.Message="";
        contentType=current="";feedbackText=null;StateChanged=null;
    }
    internal void Select(string value)=>Choices.SelectedItem=visible.FirstOrDefault(choice=>choice.Value==value);
    internal void SelectUnavailable()=>Choices.SelectedItem=visible.FirstOrDefault(choice=>!choice.CanSelect);
    internal void InvokeRefresh()=>((Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(RefreshButton).GetPattern(Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke)).Invoke();
}

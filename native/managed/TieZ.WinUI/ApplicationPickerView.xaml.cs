using System.Collections.ObjectModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TieZ.Core;

namespace TieZ.Native;

public sealed class ApplicationChoice
{
    internal string? Value { get; }
    internal string Group { get; }
    public string Name { get; }
    public string GroupLabel { get; }
    public string Details { get; }
    // Searchable text: display name, launch target/path and the opaque catalog identity.
    internal string SearchText { get; }
    public string AccessibilityLabel => $"{Name}，{GroupLabel}，{Details}";
    public bool CanSelect { get; }
    internal ApplicationChoice(string name,string details,string defaultLabel)
    {Name=name;Details=details;GroupLabel=defaultLabel;Value="";Group="default";CanSelect=true;SearchText=$"{name}\n{details}\n{defaultLabel}";}
    internal ApplicationChoice(InstalledApplication app,string reason)
    {
        Name=app.Name;Value=app.Handler;CanSelect=app.Handler is {Length:>0}&&app.Unavailable is null&&app.Kind is "desktop" or "packaged";
        Group=CanSelect?reason:"unavailable";
        string type=app.Kind=="desktop"?"桌面程序":app.Kind=="packaged"?"打包应用":"无法使用";
        GroupLabel=Group switch {"association"=>$"系统关联推荐 · {type}","keyword"=>$"名称推荐 · {type}","other"=>$"其他应用 · {type}",_=>"无法用作打开程序"};
        string identity=app.Handler??app.SourceId;
        string unavailable=app.Unavailable switch {"missing_target"=>"没有可读取的启动目标","invalid_identity"=>"应用标识无法使用","not_native_executable"=>"目标缺失或不是支持的原生程序",_=>"没有支持的打开方式"};
        Details=CanSelect?identity:$"{unavailable} · {identity}";
        SearchText=$"{app.Name}\n{app.Handler}\n{app.SourceId}\n{app.Kind}";
    }
}

// A local selection draft over a shared core snapshot. No settings update occurs here.
public sealed partial class ApplicationPickerView : UserControl, IDisposable
{
    private readonly ObservableCollection<ApplicationChoice> visible=[];
    private readonly List<ApplicationChoice> all=[];
    private Func<bool,CancellationToken,Task<ApplicationCatalogResult>>? reader;
    private CancellationTokenSource? lifetime,loadingCancellation;
    private ApplicationCatalogResult? accepted;
    private string contentType="",current="",defaultLabel="";
    private bool disposed,loading,stale,failed,binding;
    private int generation;
    internal event Action? StateChanged;
    internal bool CanChoose=>!disposed&&!loading&&!stale&&!failed&&accepted is not null&&lifetime is {IsCancellationRequested:false}&&Choices.SelectedItem is ApplicationChoice {CanSelect:true};
    internal bool IsLoading=>loading;
    internal bool IsStale=>stale;
    internal bool HasError=>Feedback.IsOpen;
    internal bool HasRetainedData=>reader is not null||accepted is not null||all.Count>0||visible.Count>0||current.Length>0||SearchText.Text.Length>0||CurrentText.Text.Length>0||SelectionText.Text.Length>0;
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
        InitializeComponent();Choices.ItemsSource=visible;
        SearchText.TextChanging+=(_,_)=>ApplyFilter();Category.SelectionChanged+=(_,_)=>ApplyFilter();
        Choices.SelectionChanged+=(_,_)=>{if(!binding&&!disposed){ShowSelection();StateChanged?.Invoke();}};
        RefreshButton.Click+=async(_,_)=>await LoadAsync(true);
    }
    internal ApplicationPickerView(string contentType,string current,string defaultLabel,Func<bool,CancellationToken,Task<ApplicationCatalogResult>> reader,CancellationToken cancellationToken):this()
    {
        this.contentType=contentType;this.current=current;this.defaultLabel=defaultLabel;this.reader=reader;
        lifetime=CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        CurrentText.Text=$"当前草稿：{(current.Length==0?defaultLabel:current)}";
    }
    private static string Identity(InstalledApplication app)=>app.Handler is { } handler?$"{app.Kind}:{(app.Kind=="desktop"?handler.ToLowerInvariant():handler)}":$"unavailable:{app.SourceId}:{app.Name}";
    private List<ApplicationChoice> Build(ApplicationCatalogResult result)
    {
        if(!result.Snapshot.Recommendations.TryGetValue(contentType,out var recommendations))throw new CoreException("invalid_response","Application recommendations missing");
        var choices=new List<ApplicationChoice> {new(defaultLabel,contentType=="code"?"保存后跟随文本打开应用":"打开时使用 Windows 当前关联","默认规则")};
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
        int version=++generation;loading=true;failed=false;Feedback.IsOpen=false;UpdateControls();
        try
        {
            var result=await reader(refresh,cancellation.Token).WaitAsync(cancellation.Token);
            if(disposed||stale||version!=generation||cancellation.IsCancellationRequested)return;
            var choices=Build(result);string? selected=(Choices.SelectedItem as ApplicationChoice)?.Value??current;
            accepted=result;all.Clear();all.AddRange(choices);
            var info=result.Snapshot.Recommendations[contentType];
            DefaultText.Text=contentType=="code"?"默认跟随文本应用":info.DefaultName is {Length:>0} name?$"Windows 默认：{name}":"Windows 默认应用名称不可读取";
            ApplyFilter(selected);
        }
        catch(OperationCanceledException) { }
        catch(Exception failure)
        {
            if(!disposed&&!stale&&version==generation)
            {
                failed=true;Feedback.Severity=InfoBarSeverity.Error;Feedback.Message=failure is CoreException {Code:"apps_busy"}
                    ?"上一次读取仍在停止，请稍后刷新。":"无法读取应用目录，请刷新重试，或取消后填写程序路径。";Feedback.IsOpen=true;
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
            visible.Clear();
            foreach(var choice in all)
                if((filter<=0||filter==1&&choice.Group is "association" or "keyword"||filter==2&&choice.Group=="other"||filter==3&&choice.Group=="unavailable")
                    &&(query.Length==0||choice.SearchText.Contains(query,StringComparison.OrdinalIgnoreCase)))visible.Add(choice);
            Choices.SelectedItem=selected is null?null:visible.FirstOrDefault(choice=>choice.CanSelect&&choice.Value==selected);
        }
        finally{binding=false;}
        ShowSelection();UpdateControls();
    }
    private void ShowSelection()=>SelectionText.Text=Choices.SelectedItem is ApplicationChoice choice?$"{(choice.CanSelect?"已选":"不可选择")}：{choice.Name} · {choice.Details}":"请选择一项；当前草稿不会自动更改。";
    private void UpdateControls()
    {
        if(disposed)return;
        Progress.Visibility=loading?Visibility.Visible:Visibility.Collapsed;Choices.IsEnabled=!loading&&!stale;RefreshButton.IsEnabled=!loading&&!stale&&lifetime is {IsCancellationRequested:false};
        if(loading)Summary.Text="正在读取应用目录…";
        else if(accepted is { } result)
        {
            var info=result.Snapshot.Recommendations[contentType];int unavailable=all.Count(choice=>!choice.CanSelect);
            string incomplete=result.Snapshot.Limited||info.Limited||result.Snapshot.Unreadable>0||info.Unreadable>0||info.AssociationError is not null?" 部分目录或推荐未能完整读取。":"";
            Summary.Text=$"显示 {visible.Count} 项；目录中 {unavailable} 项无法用作打开程序。{(result.Cached?" 使用上次成功读取的列表；可刷新。":"")}{incomplete}";
        }
        else Summary.Text=failed?"应用目录尚未读取成功。":"";
        StateChanged?.Invoke();
    }
    internal void Invalidate()
    {
        if(disposed)return;stale=true;generation++;loadingCancellation?.Cancel();loading=false;Choices.SelectedItem=null;
        Feedback.Severity=InfoBarSeverity.Warning;Feedback.Message="配置或应用草稿已变化，请取消后重新选择。";Feedback.IsOpen=true;UpdateControls();
    }
    public void Dispose()
    {
        if(disposed)return;disposed=true;generation++;loadingCancellation?.Cancel();lifetime?.Cancel();lifetime?.Dispose();lifetime=null;
        reader=null;accepted=null;Choices.SelectedItem=null;Choices.ItemsSource=null;all.Clear();visible.Clear();
        SearchText.Text="";CurrentText.Text=SelectionText.Text=Summary.Text=DefaultText.Text="";Feedback.IsOpen=false;Feedback.Message="";
        contentType=current=defaultLabel="";StateChanged=null;
    }
    internal void Select(string value)=>Choices.SelectedItem=visible.FirstOrDefault(choice=>choice.Value==value);
    internal void SelectUnavailable()=>Choices.SelectedItem=visible.FirstOrDefault(choice=>!choice.CanSelect);
    internal void InvokeRefresh()=>((Microsoft.UI.Xaml.Automation.Provider.IInvokeProvider)new Microsoft.UI.Xaml.Automation.Peers.ButtonAutomationPeer(RefreshButton).GetPattern(Microsoft.UI.Xaml.Automation.Peers.PatternInterface.Invoke)).Invoke();
}

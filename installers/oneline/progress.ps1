# Shared bootstrap display, embedded deterministically into both entrypoints.
# No worker writes live output. The timer only reads local transfer state.
$AxiomOwnsProcess = $false
foreach ($AxiomHostArgument in [Environment]::GetCommandLineArgs() | Select-Object -Skip 1) {
    if ($AxiomHostArgument -match '^-noe(x(i(t)?)?)?$') { break }
    if ($AxiomHostArgument -match '^-(c|co|com|comm|comma|comman|command|f|fi|fil|file)$') {
        $AxiomOwnsProcess = $true
        break
    }
}
if (-not ('@@PROGRESS_CLASS@@' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Runtime.InteropServices;
public sealed class @@PROGRESS_CLASS@@ : IDisposable {
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int n);
    [DllImport("kernel32.dll")] static extern bool GetConsoleMode(IntPtr h, out uint m);
    [DllImport("kernel32.dll")] static extern bool SetConsoleMode(IntPtr h, uint m);
    [StructLayout(LayoutKind.Sequential)] struct CursorInfo { public uint size; public int visible; }
    [DllImport("kernel32.dll")] static extern bool GetConsoleCursorInfo(IntPtr h, out CursorInfo info);
    [DllImport("kernel32.dll")] static extern bool SetConsoleCursorInfo(IntPtr h, ref CursorInfo info);
    readonly object gate = new object();
    readonly string action;
    readonly IntPtr handle;
    uint original;
    bool live, off, reserved, disposed, vt;
    bool cursorSaved, measured;
    CursorInfo cursor;
    string fallback = "";
    string label = "", detail = "", path = "";
    long total, received;
    int frame, generation, measuring, viewportWidth, viewportHeight;
    Timer timer, meter;
    public @@PROGRESS_CLASS@@(string action) {
        this.action = action;
        string mode = Environment.GetEnvironmentVariable("AXIOM_PROGRESS") ?? "";
        off = mode == "off";
        handle = GetStdHandle(-12);
        live = !off && mode != "plain" && Environment.GetEnvironmentVariable("CI") == null
            && Environment.GetEnvironmentVariable("TERM") != "dumb"
            && !Console.IsOutputRedirected && !Console.IsErrorRedirected
            && GetConsoleMode(handle, out original) && SetConsoleMode(handle, original | 4);
        vt=live;
        if(live) {
            cursorSaved=GetConsoleCursorInfo(handle,out cursor);
            timer = new Timer(delegate { lock(gate) { if(!disposed) Draw(); } }, null, 125, 125);
            meter = new Timer(delegate { Measure(); },null,125,125);
        }
    }
    // Bootstrap labels are printable ASCII. Escaping everything else makes width unambiguous.
    static string Safe(string value, int width) {
        StringBuilder b = new StringBuilder();
        foreach(char c in value) {
            string s = c >= 32 && c <= 126 ? c.ToString() : "?";
            if(b.Length + s.Length > width) break;
            b.Append(s);
        }
        return b.ToString();
    }
    public void Begin(string label, string detail) {
        lock(gate) {
            this.label=label; this.detail=detail; path=""; total=0; received=0; measured=false; generation++;
            if(off || disposed) return;
            if(!live) Console.Error.WriteLine("axiom " + action + ": " + Safe(detail, 2048));
            else Draw();
        }
    }
    public void Update(string path, long total) { lock(gate) { this.path=path; this.total=total; } }
    void Measure() {
        // Exactly one observer can be blocked by a mount; the UI timer never stats files.
        if(Interlocked.CompareExchange(ref measuring,1,0)!=0) return;
        try {
            string observed; int stage;
            lock(gate) { if(disposed) return; observed=path; stage=generation; }
            if(observed.Length==0) return;
            long bytes=0; bool known=false;
            try { bytes=new FileInfo(observed).Length; known=true; }
            catch(IOException) {} catch(UnauthorizedAccessException) {} catch(System.Security.SecurityException) {}
            catch(ArgumentException) {} catch(NotSupportedException) {}
            lock(gate) { if(!disposed && generation==stage && path==observed) { received=bytes; measured=known; } }
        } finally { Interlocked.Exchange(ref measuring,0); }
    }
    void Draw() {
        if(!live || label.Length == 0) return;
        try {
            int width = Console.WindowWidth - 1;
            int height = Console.WindowHeight;
            if(width < 31 || height < 3) {
                Clear();
                if(fallback != detail) { Console.Error.WriteLine("axiom "+action+": "+Safe(detail,2048)); fallback=detail; }
                return;
            }
            string position=Position(width,height);
            if(!reserved) { Console.Error.Write("\x1b[?25l\r\n\r\n\x1b[2A\x1b" + "7"); reserved=true; }
            int percent=measured && total>0 ? (int)Math.Min(99, Math.Floor((decimal)received * 100 / total)) : -1;
            int count=Math.Max(3,Math.Min(40,width/3));
            string bar="";
            for(int i=0;i<count;i++) bar += (percent>=0 ? i<percent*count/100 : i==frame%count) ? "=" : "-";
            string status=percent<0 ? "indeterminate" : percent+"%";
            string top=Safe(label,Math.Max(0,width-count-status.Length-5))+"  ["+bar+"] "+status;
            string second=Safe("|/-\\"[frame%4]+" axiom "+action+": "+detail,width);
            Console.Error.Write(position+"\x1b"+"8\r\x1b[J\x1b[2K"+top+"\r\n\x1b[2K"+second+"\x1b"+"8");
            frame=(frame+1)%120000;
        } catch(Exception) {
            live=false;
            try { Clear(); } catch(Exception) {}
            if(cursorSaved) { cursor.visible=1; SetConsoleCursorInfo(handle,ref cursor); }
        }
    }
    string Position(int width,int height) {
        bool changed=reserved && (viewportWidth!=width || viewportHeight!=height);
        viewportWidth=width; viewportHeight=height;
        return changed ? "\r\x1b"+"7" : "";
    }
    void Clear() {
        if(reserved) { string position=Position(Console.WindowWidth-1,Console.WindowHeight); Console.Error.Write(position+"\x1b"+"8\r\x1b[J\x1b[?25h"); reserved=false; }
    }
    public void Suspend() { lock(gate) { Clear(); label=""; path=""; generation++; } }
    public void Resume(string label,string detail) { Begin(label,detail); }
    public void Complete() { Suspend(); }
    public void Fail() { Suspend(); }
    public void Dispose() {
        lock(gate) {
            if(disposed) return;
            disposed=true;
            if(timer!=null) timer.Dispose();
            if(meter!=null) meter.Dispose();
            try { Clear(); } catch(IOException) {}
            finally { if(vt) SetConsoleMode(handle,original); if(cursorSaved) { cursor.visible=1; SetConsoleCursorInfo(handle,ref cursor); } }
        }
    }
}
'@ -ErrorAction Stop
}
$ui = New-Object @@PROGRESS_CLASS@@ $AxiomAction

function Receive-AxiomArchive([string]$Uri, [string]$Destination) {
    $client = New-Object Net.WebClient
    $client.Headers['User-Agent'] = 'axiom-bootstrapper'
    try {
        $task = $client.DownloadFileTaskAsync([Uri]$Uri, $Destination)
        while (-not $task.IsCompleted) {
            $total = [long]0
            if ($client.ResponseHeaders) { [long]::TryParse($client.ResponseHeaders['Content-Length'], [ref]$total) | Out-Null }
            $ui.Update($Destination, $total)
            Start-Sleep -Milliseconds 125
        }
        $task.GetAwaiter().GetResult()
    }
    finally { $client.Dispose() }
}

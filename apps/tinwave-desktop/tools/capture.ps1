# Dev-only (not packaged): screenshot of the Tinwave main window for UI tickets (design-system.md section 8, evidence).
#
#   powershell -File tools/capture.ps1 -Scheme dark -Out shell-dark.png
#   powershell -File tools/capture.ps1 -Scheme light -Keys "{TAB},{DOWN}" -Out kb-light.png
#
# - The capture is cropped to DWMWA_EXTENDED_FRAME_BOUNDS, i.e. the visible window. GetWindowRect also covers Windows 11's
#   invisible resize border, which PrintWindow paints black (a strip on the left, right and bottom edge); the 1 px outline DWM
#   draws itself is trimmed as well.
# - Keys are sent with the window in the foreground: an inactive window does not draw its focus ring under PrintWindow.
#   The foreground is verified before every key and before the grab (retried 5 times); if it cannot be taken the script
#   saves nothing and exits 1 with a message. The very first key after activation is occasionally still lost (the window
#   is active, Qt has not routed focus yet), so look at a keyboard capture before using it as evidence.
#   Keep the keyboard free while the script runs. Keys use SendKeys syntax, comma separated, one step each.
# - The colour scheme is the app's own (`--color-scheme=`); the OS theme, accent and scaling are never changed. The image
#   has the physical size of the window at the current scaling (e.g. 150 %).
param(
    [string]$Exe = '',   # default: the debug build of this checkout
    [ValidateSet('light', 'dark', 'system')][string]$Scheme = 'system',
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Style = '',
    [string]$Keys = '',
    [string]$MainQml = '',
    [string]$QtBin = 'C:\Qt\6.12.0\msvc2022_64\bin',
    [int]$SettleMs = 2000
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class TwCapture {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[TwCapture]::SetProcessDPIAware() | Out-Null

if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\..\..\target\debug\tinwave-desktop.exe' }
$Exe = (Resolve-Path $Exe).Path
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
if ($Style) { $env:QT_QUICK_CONTROLS_STYLE = $Style } else { Remove-Item Env:QT_QUICK_CONTROLS_STYLE -ErrorAction SilentlyContinue }
if ($MainQml) { $env:TINWAVE_MAIN_QML = $MainQml } else { Remove-Item Env:TINWAVE_MAIN_QML -ErrorAction SilentlyContinue }
$env:PATH = "$QtBin;$env:PATH"

$proc = Start-Process -FilePath $Exe -ArgumentList "--color-scheme=$Scheme" -PassThru
try {
    for ($i = 0; $i -lt 80 -and $proc.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 250; $proc.Refresh() }
    if ($proc.MainWindowHandle -eq 0) { throw 'the app showed no window' }
    $h = $proc.MainWindowHandle
    Start-Sleep -Milliseconds $SettleMs

    # Foreground: Windows only lets a process that owns the foreground (or shares its input queue) take it, so attach to the
    # current foreground window's input queue for the call. The Alt tap lifts the foreground lock for this process too.
    # The grab is sometimes flaky, so it is verified, retried a few times, and then fails loudly: a capture of an inactive
    # window has no focus ring and must never be saved as evidence.
    function Get-Foreground($handle, $attempts) {
        for ($try = 1; $try -le $attempts; $try++) {
            [TwCapture]::ShowWindow($handle, 9) | Out-Null
            [TwCapture]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
            [TwCapture]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
            $fg = [TwCapture]::GetForegroundWindow()
            $fgPid = [uint32]0
            $fgThread = [TwCapture]::GetWindowThreadProcessId($fg, [ref]$fgPid)
            $me = [TwCapture]::GetCurrentThreadId()
            if ($fgThread -ne 0 -and $fgThread -ne $me) { [TwCapture]::AttachThreadInput($me, $fgThread, $true) | Out-Null }
            [TwCapture]::BringWindowToTop($handle) | Out-Null
            [TwCapture]::SetForegroundWindow($handle) | Out-Null
            if ($fgThread -ne 0 -and $fgThread -ne $me) { [TwCapture]::AttachThreadInput($me, $fgThread, $false) | Out-Null }
            Start-Sleep -Milliseconds 300
            if ([TwCapture]::GetForegroundWindow() -eq $handle) { Start-Sleep -Milliseconds 500; return }   # let Qt finish activating
            Write-Warning "the window is not in the foreground (attempt $try of $attempts), retrying"
            Start-Sleep -Milliseconds 500
        }
        throw "capture aborted: Tinwave could not be brought to the foreground after $attempts attempts, so the capture would show an inactive window (no focus ring). Nothing was saved; close what is stealing focus and run again."
    }

    Get-Foreground $h 5
    if ($Keys) {
        foreach ($k in $Keys.Split(',')) {
            Get-Foreground $h 5   # checked before every key, not only once at the start
            [System.Windows.Forms.SendKeys]::SendWait($k.Trim()); Start-Sleep -Milliseconds 400
        }
        Start-Sleep -Milliseconds 500
    }
    Get-Foreground $h 5   # and once more right before the grab

    $win = New-Object TwCapture+RECT; [TwCapture]::GetWindowRect($h, [ref]$win) | Out-Null
    $vis = New-Object TwCapture+RECT
    if ([TwCapture]::DwmGetWindowAttribute($h, 9, [ref]$vis, 16) -ne 0) { throw 'DWMWA_EXTENDED_FRAME_BOUNDS failed' }
    $bmp = New-Object System.Drawing.Bitmap ($win.R - $win.L), ($win.B - $win.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [TwCapture]::PrintWindow($h, $dc, 2) | Out-Null   # 2 = PW_RENDERFULLCONTENT
    $g.ReleaseHdc($dc); $g.Dispose()
    $crop = New-Object System.Drawing.Rectangle ($vis.L - $win.L), ($vis.T - $win.T), ($vis.R - $vis.L), ($vis.B - $vis.T)

    # DWM draws the window's 1 px outline itself (2 px at 150 %), outside what PrintWindow renders, so the frame bounds still
    # include it as pure black on the left, right and bottom edge. Trim those edge lines (only while the whole line is black).
    function Test-BlackLine($bitmap, $x0, $y0, $dx, $dy, $count) {
        for ($i = 0; $i -lt $count; $i++) {
            $c = $bitmap.GetPixel($x0 + $dx * $i, $y0 + $dy * $i)
            if ($c.R -ne 0 -or $c.G -ne 0 -or $c.B -ne 0) { return $false }
        }
        return $true
    }
    # (Left/right: only the lower half is tested; the title bar paints its own pixels over the outline.)
    $halfY = $crop.Y + [int]($crop.Height / 2); $halfH = [int]($crop.Height / 2)
    while ($crop.Width -gt 2 -and (Test-BlackLine $bmp $crop.X $halfY 0 1 $halfH)) { $crop.X += 1; $crop.Width -= 1 }
    while ($crop.Width -gt 2 -and (Test-BlackLine $bmp ($crop.Right - 1) $halfY 0 1 $halfH)) { $crop.Width -= 1 }
    while ($crop.Height -gt 2 -and (Test-BlackLine $bmp $crop.X ($crop.Bottom - 1) 1 0 $crop.Width)) { $crop.Height -= 1 }
    $bmp.Clone($crop, $bmp.PixelFormat).Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    "saved $Out $($crop.Width)x$($crop.Height)"
}
catch {
    [Console]::Error.WriteLine("capture.ps1: $($_.Exception.Message)")
    $failed = $true
}
finally {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
}
if ($failed) { exit 1 }

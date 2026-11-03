# Dev-only (not packaged): screenshot of the Tinwave main window for UI tickets (design-system.md section 8, evidence).
#
#   powershell -File tools/capture.ps1 -Scheme dark -Out shell-dark.png
#   powershell -File tools/capture.ps1 -Scheme light -Keys "{TAB},{DOWN}" -Out kb-light.png
#   powershell -File tools/capture.ps1 -Scheme dark -Mode desktop -Keys "@click:30;56,@wait:500" -Out menu-dark.png
#   powershell -File tools/capture.ps1 -Scheme light -Keys "{TAB},{TAB},@down:DOWN" -Out held-down.png
#
# Modes (-Mode)
#   window   (default) PrintWindow of the app window, cropped to DWMWA_EXTENDED_FRAME_BOUNDS, i.e. the visible window.
#            GetWindowRect also covers Windows 11's invisible resize border, which PrintWindow paints black (a strip on
#            the left, right and bottom edge); the 1 px outline DWM draws itself is trimmed as well. PrintWindow never shows
#            native popup menus, and the Mica backdrop only shows if the system composes it into the PrintWindow image (it
#            came out solid in T-0036); use desktop mode for either.
#   desktop  A screen grab of exactly the area of the DWM frame bounds (Graphics.CopyFromScreen): what the user sees,
#            including Mica and native menus. Needs the window fully on a screen and nothing in front of it; the
#            window is verified to be the foreground window right before the grab. The rounded window corners show the
#            desktop behind them, and no edge trimming is done (the outline is real). The mouse pointer is not part of
#            the image.
#
# Steps (-Keys): comma separated, run in order, one step each. A step is one of
#   SendKeys text       "{TAB}", "{DOWN}", "^a", "{TAB 3}" ... (System.Windows.Forms.SendKeys syntax; unchanged).
#   @click:x;y          Left click at a window-relative LOGICAL point. x;y are IMAGE coordinates divided by the scale: pixel
#                       positions in the saved image (window and desktop mode alike) / (DPI / 96), origin at the top-left of
#                       the frame bounds, INCLUDING the native title bar. They are not QML scene coordinates (those start
#                       below the title bar). Example at 150 %: the menu button at image pixel (45,84) is @click:30;56. A
#                       point outside the window fails the run. The pointer is parked outside the window again after the click.
#   @down:KEY           Key down only (the key stays held; no focus ring is drawn for it unless the app does).
#   @up:KEY             Key up. A key still held when the run ends or fails is always released.
#   @wait:ms            Pause 1..30000 ms.
#   KEY is a name (TAB ENTER ESC SPACE BACK DELETE INSERT HOME END PGUP PGDN LEFT UP RIGHT DOWN SHIFT CTRL ALT,
#   F1..F12, A..Z, 0..9) or a hex virtual-key code (0x28). The extended-key flag is set for the navigation keys, as the
#   dedicated keys on a keyboard do (not the numpad ones). A second @down of a held key sends another key-down event
#   (what a held physical key produces). @up of a key that is not held is refused. A SendKeys step cannot start with
#   "@" (use "{@}"). The steps are checked before the app is started; a bad step exits 1 immediately.
#   Every step is trimmed, so a lone space cannot be a SendKeys step: use "@down:SPACE,@up:SPACE" for the space bar.
#
# Behaviour
# - Keys and clicks are sent with the window in the foreground: an inactive window does not draw its focus ring under
#   PrintWindow. The foreground is verified before every step and before the grab (retried 5 times; the Alt tap that
#   lifts the foreground lock is skipped when the window is already in front); if it cannot be taken the script saves
#   nothing and exits 1 with a message. Keep the keyboard and mouse free while the script runs.
# - Readiness check: before the first step and after every key / click step the script waits (max 3 s each) until the window
#   is the foreground and active window of its thread with keyboard focus (or owns an open menu), the app answers its message queue
#   (Process.Responding), and two PrintWindow frames 150 ms apart are identical, i.e. animations have finished. This makes
#   the first key after activation (Qt had not routed focus yet) arrive; if the check fails the script saves nothing and
#   exits 1. A frame that never stops changing (a running animation) only gives a warning. A key that is delivered
#   but ignored by the app cannot be detected: look at a keyboard capture before using it as evidence.
# - The mouse pointer is parked in a screen corner outside the window (no hover fill in the capture). If every corner of
#   every screen lies inside the window, a warning says so (hover fills may then show). The pointer's original position is
#   restored at the end of the run, also after a failure.
# - On any failure nothing is saved, the app is stopped, held keys are released, and the exit code is 1.
# - The colour scheme is the app's own (`--color-scheme=`); the OS theme, accent and scaling are never changed. The image
#   has the physical size of the window at the current scaling (e.g. 150 %).
param(
    [string]$Exe = '',   # default: the debug build of this checkout
    [ValidateSet('light', 'dark', 'system')][string]$Scheme = 'system',
    [Parameter(Mandatory = $true)][string]$Out,
    [ValidateSet('window', 'desktop')][string]$Mode = 'window',
    [string]$Style = '',
    [string]$Keys = '',
    [string]$MainQml = '',
    [string]$QtBin = 'C:\Qt\6.12.0\msvc2022_64\bin',
    [int]$SettleMs = 2000
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System; using System.Drawing; using System.Drawing.Imaging; using System.Runtime.InteropServices;
public static class TwCapture {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GUITHREADINFO info);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO {
    public uint cbSize, flags; public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret; public RECT rcCaret;
  }
  // FNV-1a over every pixel byte: equal hashes = identical frames (used to wait for animations to finish).
  public static ulong Hash(Bitmap b) {
    BitmapData d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    try {
      ulong h = 14695981039346656037UL;
      byte[] row = new byte[d.Width * 4];
      for (int y = 0; y < d.Height; y++) {
        Marshal.Copy(IntPtr.Add(d.Scan0, y * d.Stride), row, 0, row.Length);
        for (int i = 0; i < row.Length; i++) { h ^= row[i]; h *= 1099511628211UL; }
      }
      return h;
    } finally { b.UnlockBits(d); }
  }
}
"@
if (-not [TwCapture]::SetProcessDpiAwarenessContext([IntPtr]-4)) { [TwCapture]::SetProcessDPIAware() | Out-Null }   # -4 = per-monitor v2

# ---- Steps -------------------------------------------------------------------------------------------------------------
$extendedKeys = @(0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2D, 0x2E)
$keyNames = @{
    TAB = 0x09; ENTER = 0x0D; ESC = 0x1B; SPACE = 0x20; BACK = 0x08; DELETE = 0x2E; INSERT = 0x2D; HOME = 0x24; END = 0x23
    PGUP = 0x21; PGDN = 0x22; LEFT = 0x25; UP = 0x26; RIGHT = 0x27; DOWN = 0x28; SHIFT = 0x10; CTRL = 0x11; ALT = 0x12
}
for ($f = 1; $f -le 12; $f++) { $keyNames["F$f"] = 0x6F + $f }

function ConvertTo-VirtualKey([string]$name) {
    $n = $name.Trim().ToUpperInvariant()
    if ($keyNames.ContainsKey($n)) { return [int]$keyNames[$n] }
    if ($n -match '^[A-Z0-9]$') { return [int][char]$n }
    if ($n -match '^0X[0-9A-F]{1,2}$') { return [Convert]::ToInt32($n.Substring(2), 16) }
    throw "unknown key '$name' (a name such as TAB/DOWN/F5/A, or a hex virtual-key code such as 0x28)"
}

# Parses every step up front, so a typo fails before the app is started. Tracks held keys to refuse an @up without @down.
function ConvertTo-Steps([string]$text) {
    $steps = @(); $held = @{}
    foreach ($raw in $text.Split(',')) {
        $s = $raw.Trim()
        if (-not $s) { continue }
        if (-not $s.StartsWith('@')) { $steps += , @{ Kind = 'keys'; Text = $s }; continue }
        if ($s -notmatch '^@(click|down|up|wait):(.+)$') { throw "bad step '$s' (expected @click:x;y, @down:KEY, @up:KEY or @wait:ms)" }
        $arg = $Matches[2].Trim()
        switch ($Matches[1]) {
            'click' {
                if ($arg -notmatch '^(\d+(\.\d+)?);(\d+(\.\d+)?)$') { throw "bad step '$s' (expected @click:x;y with non-negative logical pixels)" }
                $steps += , @{ Kind = 'click'; X = [double]$Matches[1]; Y = [double]$Matches[3]; Text = $s }
            }
            'wait' {
                if ($arg -notmatch '^\d+$' -or [int]$arg -lt 1 -or [int]$arg -gt 30000) { throw "bad step '$s' (expected @wait:1..30000)" }
                $steps += , @{ Kind = 'wait'; Ms = [int]$arg; Text = $s }
            }
            'down' { $vk = ConvertTo-VirtualKey $arg; $held[$vk] = $true; $steps += , @{ Kind = 'down'; Vk = $vk; Text = $s } }
            'up' {
                $vk = ConvertTo-VirtualKey $arg
                if (-not $held.ContainsKey($vk)) { throw "bad step '$s': that key is not held (an @down of it must come first)" }
                $held.Remove($vk); $steps += , @{ Kind = 'up'; Vk = $vk; Text = $s }
            }
        }
    }
    return , $steps
}

# ---- Run ---------------------------------------------------------------------------------------------------------------
$proc = $null
$origCursor = New-Object TwCapture+POINT; [TwCapture]::GetCursorPos([ref]$origCursor) | Out-Null
$heldKeys = @{}   # vk -> $true while a synthetic key-down has no key-up yet; released in `finally`
$cursorWarned = $false
$failed = $false
try {
    $steps = ConvertTo-Steps $Keys

    if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\..\..\target\debug\tinwave-desktop.exe' }
    $Exe = (Resolve-Path $Exe).Path
    $Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
    if ($Style) { $env:QT_QUICK_CONTROLS_STYLE = $Style } else { Remove-Item Env:QT_QUICK_CONTROLS_STYLE -ErrorAction SilentlyContinue }
    if ($MainQml) { $env:TINWAVE_MAIN_QML = $MainQml } else { Remove-Item Env:TINWAVE_MAIN_QML -ErrorAction SilentlyContinue }
    $env:PATH = "$QtBin;$env:PATH"

    $proc = Start-Process -FilePath $Exe -ArgumentList "--color-scheme=$Scheme" -PassThru
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
            if ([TwCapture]::GetForegroundWindow() -eq $handle) { return }   # already in front: no Alt tap, no extra key events
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

    # Frame hash of the window as PrintWindow paints it (only used to detect that the picture has stopped changing).
    function Get-FrameHash {
        $r = New-Object TwCapture+RECT; [TwCapture]::GetWindowRect($h, [ref]$r) | Out-Null
        $b = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
        try {
            $g = [System.Drawing.Graphics]::FromImage($b); $dc = $g.GetHdc()
            [TwCapture]::PrintWindow($h, $dc, 2) | Out-Null
            $g.ReleaseHdc($dc); $g.Dispose()
            return [TwCapture]::Hash($b)
        } finally { $b.Dispose() }
    }

    # Readiness: foreground + active + keyboard focus (or an open menu owned by the window) + app responding + picture stable.
    function Wait-Ready([string]$what) {
        Get-Foreground $h 5
        $tpid = [uint32]0; $thread = [TwCapture]::GetWindowThreadProcessId($h, [ref]$tpid)
        $deadline = [DateTime]::UtcNow.AddSeconds(3)
        while ($true) {
            $gti = New-Object TwCapture+GUITHREADINFO; $gti.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($gti)
            [TwCapture]::GetGUIThreadInfo($thread, [ref]$gti) | Out-Null
            $focusRoot = if ($gti.hwndFocus -ne [IntPtr]::Zero) { [TwCapture]::GetAncestor($gti.hwndFocus, 2) } else { [IntPtr]::Zero }
            $ready = ([TwCapture]::GetForegroundWindow() -eq $h) -and ($gti.hwndActive -eq $h) -and (($focusRoot -eq $h) -or ($gti.hwndMenuOwner -eq $h))
            if ($ready) { break }
            if ([DateTime]::UtcNow -gt $deadline) {
                throw "capture aborted: Tinwave is not ready after $what (foreground/active/keyboard focus missing for 3 s). Nothing was saved."
            }
            Start-Sleep -Milliseconds 100
        }
        $proc.Refresh()
        $respDeadline = [DateTime]::UtcNow.AddSeconds(3)
        while (-not $proc.Responding) {
            if ([DateTime]::UtcNow -gt $respDeadline) { throw "capture aborted: Tinwave does not respond after $what (3 s). Nothing was saved." }
            Start-Sleep -Milliseconds 100
            $proc.Refresh()
        }
        $prev = Get-FrameHash
        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Milliseconds 150
            $cur = Get-FrameHash
            if ($cur -eq $prev) { return }
            $prev = $cur
        }
        Write-Warning "the window content was still changing 3 s after $what (a running animation?); the capture may show a mid-animation frame"
    }

    # Park the mouse pointer in a screen corner outside the window: a pointer over the sidebar paints a hover fill on a row
    # that looks like a second selection in keyboard evidence. Warns (once) when no corner is free.
    function Move-CursorAway {
        $rc = New-Object TwCapture+RECT; [TwCapture]::GetWindowRect($h, [ref]$rc) | Out-Null
        foreach ($scr in [System.Windows.Forms.Screen]::AllScreens) {
            $b = $scr.Bounds
            foreach ($c in @(@(($b.Right - 1), $b.Top), @($b.Left, $b.Top), @($b.Left, ($b.Bottom - 1)), @(($b.Right - 1), ($b.Bottom - 1)))) {
                if ($c[0] -ge $rc.L -and $c[0] -lt $rc.R -and $c[1] -ge $rc.T -and $c[1] -lt $rc.B) { continue }
                [TwCapture]::SetCursorPos($c[0], $c[1]) | Out-Null
                $now = New-Object TwCapture+POINT; [TwCapture]::GetCursorPos([ref]$now) | Out-Null
                if ($now.X -eq $c[0] -and $now.Y -eq $c[1]) { return }
            }
        }
        if (-not $script:cursorWarned) {
            $script:cursorWarned = $true
            Write-Warning 'the mouse pointer cannot be parked outside the window (the window covers every screen corner): hover fills may show in the capture'
        }
    }

    function Get-Frame {
        $v = New-Object TwCapture+RECT
        if ([TwCapture]::DwmGetWindowAttribute($h, 9, [ref]$v, 16) -ne 0) { throw 'DWMWA_EXTENDED_FRAME_BOUNDS failed' }
        return $v
    }

    function Send-KeyEvent([int]$vk, [bool]$up) {
        $flags = 0
        if ($extendedKeys -contains $vk) { $flags = $flags -bor 1 }   # KEYEVENTF_EXTENDEDKEY
        if ($up) { $flags = $flags -bor 2 }                            # KEYEVENTF_KEYUP
        [TwCapture]::keybd_event([byte]$vk, 0, $flags, [UIntPtr]::Zero)
    }

    Move-CursorAway
    Get-Foreground $h 5
    if ($steps.Count -gt 0) { Wait-Ready 'activation' }
    foreach ($step in $steps) {
        if ($step.Kind -eq 'wait') { Start-Sleep -Milliseconds $step.Ms; continue }
        Get-Foreground $h 5   # checked before every step, not only once at the start
        switch ($step.Kind) {
            'keys' { [System.Windows.Forms.SendKeys]::SendWait($step.Text) }
            'down' { Send-KeyEvent $step.Vk $false; $heldKeys[$step.Vk] = $true }
            'up'   { Send-KeyEvent $step.Vk $true; $heldKeys.Remove($step.Vk) }
            'click' {
                $dpi = [TwCapture]::GetDpiForWindow($h); if ($dpi -eq 0) { throw 'GetDpiForWindow failed' }
                $scale = $dpi / 96.0
                $v = Get-Frame
                $w = ($v.R - $v.L) / $scale; $ht = ($v.B - $v.T) / $scale
                if ($step.X -ge $w -or $step.Y -ge $ht) {
                    throw ("step '{0}' lies outside the window ({1:0} x {2:0} logical pixels at {3:0} % scaling)" -f $step.Text, $w, $ht, ($scale * 100))
                }
                $px = $v.L + [int][Math]::Round($step.X * $scale); $py = $v.T + [int][Math]::Round($step.Y * $scale)
                [TwCapture]::SetCursorPos($px, $py) | Out-Null
                Start-Sleep -Milliseconds 150
                [TwCapture]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)   # MOUSEEVENTF_LEFTDOWN
                Start-Sleep -Milliseconds 60
                [TwCapture]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)   # MOUSEEVENTF_LEFTUP
                Start-Sleep -Milliseconds 150
                Move-CursorAway
            }
        }
        Start-Sleep -Milliseconds 400
        Wait-Ready ("step '{0}'" -f $step.Text)
    }
    if ($steps.Count -gt 0) { Start-Sleep -Milliseconds 500 }
    Get-Foreground $h 5   # and once more right before the grab

    $win = New-Object TwCapture+RECT; [TwCapture]::GetWindowRect($h, [ref]$win) | Out-Null
    $vis = Get-Frame

    if ($Mode -eq 'desktop') {
        $virt = [System.Windows.Forms.SystemInformation]::VirtualScreen
        if ($vis.L -lt $virt.Left -or $vis.T -lt $virt.Top -or $vis.R -gt $virt.Right -or $vis.B -gt $virt.Bottom) {
            throw 'capture aborted: the window is not fully on screen, a desktop grab would be incomplete. Nothing was saved.'
        }
        Start-Sleep -Milliseconds 300   # let the compositor settle (Mica, menu fade)
        $crop = New-Object System.Drawing.Rectangle $vis.L, $vis.T, ($vis.R - $vis.L), ($vis.B - $vis.T)
        $bmp = New-Object System.Drawing.Bitmap $crop.Width, $crop.Height
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($crop.X, $crop.Y, 0, 0, $bmp.Size)
        $g.Dispose()
        $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        "saved $Out $($crop.Width)x$($crop.Height) (desktop)"
    }
    else {
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
}
catch {
    [Console]::Error.WriteLine("capture.ps1: $($_.Exception.Message)")
    $failed = $true
}
finally {
    # Nothing may outlive the run: held keys are released (a stuck key would stay down in the whole session), the app is
    # stopped and the pointer goes back where it was.
    foreach ($vk in @($heldKeys.Keys)) {
        $flags = 2; if ($extendedKeys -contains $vk) { $flags = 3 }
        [TwCapture]::keybd_event([byte]$vk, 0, $flags, [UIntPtr]::Zero)
    }
    if ($proc) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        $proc.WaitForExit(5000) | Out-Null   # bounded: nothing may outlive the script
    }
    [TwCapture]::SetCursorPos($origCursor.X, $origCursor.Y) | Out-Null
}
if ($failed) { exit 1 }

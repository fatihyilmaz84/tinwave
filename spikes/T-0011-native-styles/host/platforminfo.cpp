#include "platforminfo.h"

#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QPixmap>
#include <QQuickWindow>
#include <QScreen>
#include <QStyleHints>
#include <QTimer>
#include <QSGRendererInterface>
#include <QtQml/QQmlEngine>
#include <cstdio>

#ifdef Q_OS_WIN
#  include <windows.h>
#  include <dwmapi.h>
#  include <psapi.h>
#  ifndef DWMWA_SYSTEMBACKDROP_TYPE
#    define DWMWA_SYSTEMBACKDROP_TYPE 38
#  endif
#  ifndef SPI_GETCLIENTAREAANIMATION
#    define SPI_GETCLIENTAREAANIMATION 0x1042
#  endif
#endif

static PlatformInfo *g_instance = nullptr;
#ifdef Q_OS_WIN
static HWND g_mainHwnd = nullptr;
#endif

PlatformInfo::PlatformInfo(QObject *parent) : QObject(parent)
{
    g_instance = this;
    qApp->installNativeEventFilter(this);
    refresh();
}

PlatformInfo *PlatformInfo::instance() { return g_instance; }

#ifdef Q_OS_WIN
static QString regDword(HKEY root, const wchar_t *sub, const wchar_t *name)
{
    DWORD v = 0, size = sizeof v;
    if (RegGetValueW(root, sub, name, RRF_RT_REG_DWORD, nullptr, &v, &size) != ERROR_SUCCESS)
        return {};
    return QString::number(v);
}
#endif

void PlatformInfo::refresh()
{
#ifdef Q_OS_WIN
    // Reduced motion: "Animation effects" off  <=>  SPI_GETCLIENTAREAANIMATION == FALSE.
    BOOL anim = TRUE;
    if (SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &anim, 0))
        m_reducedMotion = !anim;
    // High contrast: SPI_GETHIGHCONTRAST dwFlags & HCF_HIGHCONTRASTON.
    HIGHCONTRASTW hc{};
    hc.cbSize = sizeof hc;
    wchar_t scheme[256] = {};
    hc.lpszDefaultScheme = scheme;
    if (SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof hc, &hc, 0)) {
        m_highContrast = (hc.dwFlags & HCF_HIGHCONTRASTON) != 0;
        m_hcScheme = QString::fromWCharArray(hc.lpszDefaultScheme);
    }
    // "Transparency effects" toggle. Mica is drawn solid by DWM when it is off.
    const QString t = regDword(HKEY_CURRENT_USER,
        L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", L"EnableTransparency");
    m_transparency = t.isEmpty() ? true : t != QLatin1String("0");
    // Build number (RtlGetVersion is not subject to the manifest-based version lie).
    using RtlGetVersionFn = LONG(WINAPI *)(PRTL_OSVERSIONINFOW);
    if (auto fn = reinterpret_cast<RtlGetVersionFn>(GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "RtlGetVersion"))) {
        RTL_OSVERSIONINFOW vi{};
        vi.dwOSVersionInfoSize = sizeof vi;
        if (fn(&vi) == 0)
            m_build = int(vi.dwBuildNumber);
    }
    // Cross-check of the OS accent (registry, ABGR) against what Qt reports in palette.accent.
    DWORD abgr = 0, sz = sizeof abgr;
    if (RegGetValueW(HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\DWM", L"AccentColor",
                     RRF_RT_REG_DWORD, nullptr, &abgr, &sz) == ERROR_SUCCESS) {
        m_dwmAccent = QStringLiteral("#%1%2%3")
                          .arg(abgr & 0xFF, 2, 16, QLatin1Char('0'))
                          .arg((abgr >> 8) & 0xFF, 2, 16, QLatin1Char('0'))
                          .arg((abgr >> 16) & 0xFF, 2, 16, QLatin1Char('0'))
                          .toUpper();
    }
#endif
}

QString PlatformInfo::qtHints() const
{
    QString s;
    const auto *h = QGuiApplication::styleHints();
    s += QStringLiteral("colorScheme=%1").arg(int(h->colorScheme()));
#if QT_VERSION >= QT_VERSION_CHECK(6, 10, 0)
    s += QStringLiteral(" contrastPreference=%1").arg(int(h->accessibility()->contrastPreference()));
#endif
#if QT_VERSION >= QT_VERSION_CHECK(6, 12, 0)
    s += QStringLiteral(" motionPreference=%1").arg(int(h->accessibility()->motionPreference()));
#endif
    return s;
}

bool PlatformInfo::nativeEventFilter(const QByteArray &eventType, void *message, qintptr *)
{
#ifdef Q_OS_WIN
    if (eventType == "windows_generic_MSG") {
        const MSG *m = static_cast<const MSG *>(message);
        switch (m->message) {
        case WM_SETTINGCHANGE:
        case WM_THEMECHANGED:
        case WM_SYSCOLORCHANGE:
        case 0x0320 /* WM_DWMCOLORIZATIONCOLORCHANGED */: {
            const bool wasRm = m_reducedMotion, wasHc = m_highContrast;
            refresh();
            ++m_changeCount;
            std::printf("EVENT msg=0x%04x wParam=0x%llx reducedMotion %d->%d highContrast %d->%d\n",
                        m->message, (unsigned long long)m->wParam, wasRm, m_reducedMotion, wasHc, m_highContrast);
            std::fflush(stdout);
            emit changed();
            break;
        }
        default:
            break;
        }
    }
#else
    Q_UNUSED(eventType)
    Q_UNUSED(message)
#endif
    return false;
}

bool PlatformInfo::enableDarkMenus(int mode)
{
#ifdef Q_OS_WIN
    // UNDOCUMENTED uxtheme ordinals (135 = SetPreferredAppMode, 136 = FlushMenuThemes), per-process only.
    // Spike evidence for "does the native Win32 menu follow dark mode"; NOT a recommendation to ship.
    HMODULE ux = LoadLibraryW(L"uxtheme.dll");
    if (!ux)
        return false;
    using SetPreferredAppModeFn = int(WINAPI *)(int);
    using FlushMenuThemesFn = void(WINAPI *)();
    auto setMode = reinterpret_cast<SetPreferredAppModeFn>(GetProcAddress(ux, MAKEINTRESOURCEA(135)));
    auto flush = reinterpret_cast<FlushMenuThemesFn>(GetProcAddress(ux, MAKEINTRESOURCEA(136)));
    if (!setMode)
        return false;
    setMode(mode);   // 0 default, 1 AllowDark (follow system), 2 ForceDark, 3 ForceLight
    if (flush)
        flush();
    return true;
#else
    Q_UNUSED(mode)
    return false;
#endif
}

QString PlatformInfo::applyMica(QQuickWindow *w, bool on)
{
#ifdef Q_OS_WIN
    if (!w)
        return QStringLiteral("error:no window");
    if (!micaSupported())
        return QStringLiteral("unsupported:build %1 < 22621").arg(m_build);
    HWND hwnd = reinterpret_cast<HWND>(w->winId());
    g_mainHwnd = hwnd;
    std::printf("MICA window alpha=%d color=%s api=%d exStyle=0x%lx\n", w->format().alphaBufferSize(),
                qPrintable(w->color().name(QColor::HexArgb)), int(w->rendererInterface()->graphicsApi()),
                (unsigned long)GetWindowLongW(hwnd, GWL_EXSTYLE));
    const int backdrop = on ? 2 /* DWMSBT_MAINWINDOW = Mica */ : 1 /* DWMSBT_NONE */;
    HRESULT hr = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop, sizeof backdrop);
    if (FAILED(hr))
        return QStringLiteral("error:DwmSetWindowAttribute 0x%1").arg(quint32(hr), 8, 16, QLatin1Char('0'));
    const MARGINS m = on ? MARGINS{-1, -1, -1, -1} : MARGINS{0, 0, 0, 0};
    hr = DwmExtendFrameIntoClientArea(hwnd, &m);
    if (FAILED(hr))
        return QStringLiteral("error:DwmExtendFrameIntoClientArea 0x%1").arg(quint32(hr), 8, 16, QLatin1Char('0'));
    return QStringLiteral("ok");
#else
    Q_UNUSED(w)
    Q_UNUSED(on)
    return QStringLiteral("unsupported:not windows");
#endif
}

void PlatformInfo::simulateSettingChange(QQuickWindow *w, const QString &what)
{
#ifdef Q_OS_WIN
    // Delivered to OUR window only (never HWND_BROADCAST); the setting itself is untouched.
    HWND hwnd = reinterpret_cast<HWND>(w->winId());
    WPARAM wp = 0;
    LPARAM lp = 0;
    if (what == QLatin1String("motion"))
        wp = SPI_SETCLIENTAREAANIMATION;
    else if (what == QLatin1String("contrast"))
        wp = SPI_SETHIGHCONTRAST;
    else
        lp = reinterpret_cast<LPARAM>(L"ImmersiveColorSet");
    const LRESULT r = SendMessageW(hwnd, WM_SETTINGCHANGE, wp, lp);
    std::printf("SIMULATE %s hwnd=%p result=%lld\n", qPrintable(what), (void *)hwnd, (long long)r);
#else
    Q_UNUSED(w)
    Q_UNUSED(what)
#endif
}

double PlatformInfo::processAgeMs()
{
#ifdef Q_OS_WIN
    FILETIME c, e, k, u, now;
    GetProcessTimes(GetCurrentProcess(), &c, &e, &k, &u);
    GetSystemTimePreciseAsFileTime(&now);
    ULARGE_INTEGER a, b;
    a.LowPart = c.dwLowDateTime;
    a.HighPart = c.dwHighDateTime;
    b.LowPart = now.dwLowDateTime;
    b.HighPart = now.dwHighDateTime;
    return double(b.QuadPart - a.QuadPart) / 10000.0;
#else
    return 0;
#endif
}

QString PlatformInfo::rssLine()
{
#ifdef Q_OS_WIN
    PROCESS_MEMORY_COUNTERS_EX2 pm{};
    pm.cb = sizeof pm;
    if (!K32GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS *>(&pm), sizeof pm))
        return QStringLiteral("rss_err");
    FILETIME c, e, k, u;
    GetProcessTimes(GetCurrentProcess(), &c, &e, &k, &u);
    ULARGE_INTEGER ku, uu;
    ku.LowPart = k.dwLowDateTime;
    ku.HighPart = k.dwHighDateTime;
    uu.LowPart = u.dwLowDateTime;
    uu.HighPart = u.dwHighDateTime;
    return QStringLiteral("ws_mb=%1 private_ws_mb=%2 commit_mb=%3 cpu_ms=%4")
        .arg(pm.WorkingSetSize / 1048576.0, 0, 'f', 1)
        .arg(pm.PrivateWorkingSetSize / 1048576.0, 0, 'f', 1)
        .arg(pm.PrivateUsage / 1048576.0, 0, 'f', 1)
        .arg((ku.QuadPart + uu.QuadPart) / 10000.0, 0, 'f', 0);
#else
    return {};
#endif
}

QString PlatformInfo::metrics(const QString &tag)
{
    const QString line = QStringLiteral("METRIC %1 t_ms=%2 %3").arg(tag).arg(processAgeMs(), 0, 'f', 0).arg(rssLine());
    std::printf("%s\n", qPrintable(line));
    std::fflush(stdout);
    return line;
}

void PlatformInfo::log(const QString &line)
{
    std::printf("%s\n", qPrintable(line));
    std::fflush(stdout);
}

void PlatformInfo::quitLater(int ms)
{
    QTimer::singleShot(ms, qApp, &QCoreApplication::quit);
}

#ifdef Q_OS_WIN
struct EnumCtx
{
    QList<HWND> found;
};

static BOOL CALLBACK enumProc(HWND h, LPARAM lp)
{
    DWORD pid = 0;
    GetWindowThreadProcessId(h, &pid);
    if (pid != GetCurrentProcessId() || !IsWindowVisible(h) || h == g_mainHwnd)
        return TRUE;
    wchar_t cls[128] = {};
    GetClassNameW(h, cls, 128);
    if (wcsstr(cls, L"QWindowIcon"))   // Qt top-level windows with an icon class == our main window; never crop it
        return TRUE;
    reinterpret_cast<EnumCtx *>(lp)->found.append(h);
    return TRUE;
}

static QImage cropScreen(const RECT &r)
{
    // Grabs the primary screen into memory only; only the crop of OUR window is ever written to disk.
    QScreen *s = QGuiApplication::primaryScreen();
    const QPixmap all = s->grabWindow(0);
    return all.copy(r.left, r.top, r.right - r.left, r.bottom - r.top).toImage();
}
#endif

void PlatformInfo::scheduleProbe(const QString &label, int delayMs, bool dismiss, const QString &how)
{
#ifdef Q_OS_WIN
    QTimer::singleShot(delayMs, qApp, [this, label, dismiss, how] {
        EnumCtx ctx;
        EnumWindows(enumProc, reinterpret_cast<LPARAM>(&ctx));
        std::printf("PROBE %s: %lld extra visible top-level window(s) in this process\n", qPrintable(label),
                    (long long)ctx.found.size());
        int i = 0;
        const QString shotDir = m_options.value(QStringLiteral("probeShotDir")).toString();
        for (HWND h : std::as_const(ctx.found)) {
            wchar_t cls[128] = {}, title[256] = {};
            GetClassNameW(h, cls, 128);
            GetWindowTextW(h, title, 256);
            RECT r{};
            if (FAILED(DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &r, sizeof r)))
                GetWindowRect(h, &r);
            std::printf("PROBE %s: class='%s' title='%s' rect=%ldx%ld@%ld,%ld\n", qPrintable(label),
                        qPrintable(QString::fromWCharArray(cls)), qPrintable(QString::fromWCharArray(title)),
                        r.right - r.left, r.bottom - r.top, r.left, r.top);
            if (!shotDir.isEmpty()) {
                const QString path = shotDir + QStringLiteral("/probe-%1-%2.png").arg(label).arg(i);
                cropScreen(r).save(path);
                std::printf("PROBE %s: saved %s\n", qPrintable(label), qPrintable(path));
            }
            ++i;
        }
        std::fflush(stdout);
        if (dismiss) {
            if (how == QLatin1String("menu"))
                EndMenu();
            for (HWND h : std::as_const(ctx.found))
                PostMessageW(h, WM_CLOSE, 0, 0);
        }
    });
#else
    Q_UNUSED(label)
    Q_UNUSED(delayMs)
    Q_UNUSED(dismiss)
    Q_UNUSED(how)
#endif
}

void PlatformInfo::saveShot(QQuickWindow *w, const QString &path, const QString &whichGrab)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
#ifdef Q_OS_WIN
    if (whichGrab == QLatin1String("screen")) {
        // Whole top-level frame incl. title bar and Mica, cropped to OUR window's bounds.
        HWND hwnd = reinterpret_cast<HWND>(w->winId());
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        for (int i = 0; i < 15; ++i) {   // let DWM compose the topmost window before reading the desktop
            QCoreApplication::processEvents();
            Sleep(20);
        }
        RECT r{};
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &r, sizeof r);
        cropScreen(r).save(path);
        return;
    }
#endif
    w->grabWindow().save(path);
}

// T-0011 spike host: starts the QML prototype with a chosen Qt Quick Controls style and prints
// startup / memory metrics. Closes itself; never changes a Windows setting.
//
//   t0011-host --style FluentWinUI3 --page controls --measure
//   t0011-host --style Basic --scheme dark --accent "#E3008C" --shot out.png
//
// Options:
//   --style <name>      QQuickStyle name (Basic, Fusion, Windows, FluentWinUI3, Native, ...)
//   --page <name>       controls (default) | minimal | selftest
//   --scheme <s>        system (default) | light | dark   -> applied via Theme.setSchemeOverride (setColorScheme)
//   --accent <#rrggbb>  in-app accent override (stands in for a different OS accent)
//   --mica              enable Mica behind the sidebar (Windows 11 22H2+)
//   --shot <file.png>   save a screenshot of the window content after it has settled, then exit
//   --shot-grab <how>   quick (default, item grab) | screen (desktop crop of our window incl. Mica/title bar)
//   --probe <what>      menu | file | folder | message : open that native popup, report what Windows made, close it
//   --probe-shots <dir> also save crops of the probe windows
//   --idle <ms>         idle time before the second RSS sample (default 1500)
//   --motion-test       exercise the settings-changed plumbing against our own window
//   --live <seconds>    keep the window open that long (default watchdog is raised), for the PO runbook
//   --scheme-cycle      flip light/dark/system via Theme.setSchemeOverride and log what Theme + native palettes saw
//   --timeout <ms>      hard watchdog (default 20000)
#include "platforminfo.h"

#include <QCommandLineParser>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QtQml/qqml.h>
#include <QTimer>
#include <cstdio>
#include <memory>

int main(int argc, char *argv[])
{
    const double tMain = PlatformInfo::processAgeMs();

    // Parse before QGuiApplication so alpha-buffer default can be set first.
    QVariantMap opt;
    QString style;
    int timeoutMs = 20000;
    for (int i = 1; i < argc; ++i) {
        const QString a = QString::fromLocal8Bit(argv[i]);
        auto next = [&]() { return i + 1 < argc ? QString::fromLocal8Bit(argv[++i]) : QString(); };
        if (a == "--style") style = next();
        else if (a == "--page") opt["page"] = next();
        else if (a == "--scheme") opt["scheme"] = next();
        else if (a == "--accent") opt["accent"] = next();
        else if (a == "--mica") opt["mica"] = true;
        else if (a == "--shot") opt["shot"] = next();
        else if (a == "--shot-grab") opt["shotGrab"] = next();
        else if (a == "--probe") opt["probe"] = next();
        else if (a == "--probe-shots") opt["probeShotDir"] = next();
        else if (a == "--idle") opt["idle"] = next().toInt();
        else if (a == "--motion-test") opt["motionTest"] = true;
        else if (a == "--scheme-cycle") opt["schemeCycle"] = true;
        else if (a == "--no-accent-fix") opt["noAccentFix"] = true;
        else if (a == "--dlg-folder") opt["dlgFolder"] = next();   // file:/// URL the dialogs open in
        else if (a == "--dark-menus") opt["darkMenus"] = next().toInt();   // uxtheme SetPreferredAppMode experiment
        else if (a == "--live") opt["live"] = next().toInt();   // keep the window open N seconds (PO runbook)
        else if (a == "--no-native") opt["noNative"] = true;
        else if (a == "--no-dialogs") opt["noDialogs"] = true;
        else if (a == "--timeout") timeoutMs = next().toInt();
    }
    if (opt.contains("live") && timeoutMs < opt.value("live").toInt() * 1000 + 10000)
        timeoutMs = opt.value("live").toInt() * 1000 + 10000;   // watchdog must outlive --live
    if (opt.value("mica").toBool()) {
        // Needed for Mica under a Qt Quick (D3D11) window: without WS_EX_NOREDIRECTIONBITMAP the transparent
        // pixels show the HWND's white GDI redirection surface instead of the DWM backdrop (measured, see ADR-draft).
        qputenv("QT_QPA_DISABLE_REDIRECTION_SURFACE", "1");
        QQuickWindow::setDefaultAlphaBuffer(true);
    }

    QGuiApplication app(argc, argv);
    app.setApplicationName("t0011-host");
    if (!style.isEmpty())
        QQuickStyle::setStyle(style);

    if (opt.contains("darkMenus"))
        std::printf("DARKMENUS mode=%d applied=%d\n", opt.value("darkMenus").toInt(), PlatformInfo::enableDarkMenus(opt.value("darkMenus").toInt()));
    PlatformInfo info;
    opt["style"] = QQuickStyle::name();
    info.setOptions(opt);
    qmlRegisterSingletonInstance("Tinwave.Spike", 1, 0, "PlatformInfo", &info);

    const QString page = opt.value("page", "controls").toString();
    std::printf("RUN style=%s page=%s scheme=%s accent=%s mica=%d qt=%s\n", qPrintable(QQuickStyle::name()),
                qPrintable(page), qPrintable(opt.value("scheme", "system").toString()),
                qPrintable(opt.value("accent", "-").toString()), opt.value("mica").toBool() ? 1 : 0, qVersion());
    std::printf("METRIC main_entered t_ms=%.0f %s\n", tMain, qPrintable(PlatformInfo::rssLine()));

    QTimer watchdog;
    watchdog.setSingleShot(true);
    QObject::connect(&watchdog, &QTimer::timeout, &app, [] {
        std::printf("WATCHDOG timeout, exiting\n");
        std::fflush(stdout);
        QCoreApplication::exit(3);
    });
    watchdog.start(timeoutMs);

    QQmlApplicationEngine engine;
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(2); },
                     Qt::QueuedConnection);
    const char *root = page == "selftest" ? "SelfTest" : page == "minimal" ? "MinimalWindow" : "Main";
    engine.loadFromModule("Tinwave.Spike", root);
    std::printf("METRIC engine_loaded t_ms=%.0f %s\n", PlatformInfo::processAgeMs(), qPrintable(PlatformInfo::rssLine()));
    if (engine.rootObjects().isEmpty())
        return 2;

    auto *win = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    if (win) {
        const int idleMs = opt.value("idle", 1500).toInt();
        // frameSwapped can be emitted from the render thread several times before the GUI thread runs the
        // first queued call; guard with a flag (an earlier version double-deleted a heap object here).
        auto seen = std::make_shared<bool>(false);
        QObject::connect(win, &QQuickWindow::frameSwapped, win, [&, win, seen, idleMs] {
            if (*seen)
                return;
            *seen = true;
            std::printf("METRIC first_frame t_ms=%.0f %s\n", PlatformInfo::processAgeMs(), qPrintable(PlatformInfo::rssLine()));
            std::fflush(stdout);
            QTimer::singleShot(idleMs, win, [&, win] {
                std::printf("METRIC idle t_ms=%.0f %s\n", PlatformInfo::processAgeMs(), qPrintable(PlatformInfo::rssLine()));
                std::printf("INFO dpr=%.2f size=%dx%d\n", win->devicePixelRatio(), win->width(), win->height());
                std::fflush(stdout);
                if (opt.contains("shot")) {
                    info.saveShot(win, opt.value("shot").toString(), opt.value("shotGrab", "quick").toString());
                    std::printf("SHOT %s\n", qPrintable(opt.value("shot").toString()));
                }
                if (opt.contains("live"))
                    QTimer::singleShot(opt.value("live").toInt() * 1000, &app, &QCoreApplication::quit);
                else if (!opt.contains("probe") && !opt.contains("motionTest") && !opt.contains("schemeCycle"))
                    QTimer::singleShot(100, &app, &QCoreApplication::quit);
            });
        });
    }
    const int rc = app.exec();
    std::printf("EXIT rc=%d t_ms=%.0f\n", rc, PlatformInfo::processAgeMs());
    return rc;
}

// T-0041 reproducer main: stress QML + crash capture (vectored handler -> minidump + symbolized stack).
#include "providers.h"

#include <QtCore/QDir>
#include <QtCore/QTimer>
#include <QtGui/QGuiApplication>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlContext>

#include "crash.h"

static int envInt(const char* name, int def)
{
    const int v = qEnvironmentVariableIntValue(name);
    return v ? v : def;
}

int main(int argc, char** argv)
{
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    AddVectoredExceptionHandler(1, onException);
    QGuiApplication app(argc, argv);
    {
        QQmlApplicationEngine engine;
        engine.addImageProvider("art", make_provider(qEnvironmentVariable("TW_PROVIDER", "queued"), envInt("TW_THREADS", 2)));
        QVariantMap cfg;
        cfg["tick"] = envInt("TW_TICK", 4);
        cfg["exitMs"] = envInt("TW_EXIT_MS", 600);
        cfg["drain"] = envInt("TW_DRAIN_MS", 0);
        cfg["mode"] = qEnvironmentVariable("TW_MODE", "mix");
        engine.rootContext()->setContextProperty("cfg", cfg);
        QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { fprintf(stderr, "QML load failed\n"); exit(2); });
        engine.load(QUrl::fromLocalFile(QCoreApplication::applicationDirPath() + "/stress.qml"));
        app.exec();
        if (qEnvironmentVariableIntValue("TW_NOQUIESCE") == 0) quiesce_providers();
        fprintf(stderr, "TW EXEC_RETURNED\n"); fflush(stderr);
    }   // engine (and with it the provider) torn down here, with requests possibly still in flight
    fprintf(stderr, "TW ENGINE_DESTROYED\n"); fflush(stderr);
    printf("TW DONE\n"); fflush(stdout);
    return 0;
}

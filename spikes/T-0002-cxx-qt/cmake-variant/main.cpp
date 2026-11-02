#include <QtGui/QGuiApplication>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlEngine>
#include <QtQuickControls2/QQuickStyle>

extern "C" void tw_init_metrics();
QQmlImageProviderBase* tw_make_image_provider(); // from the Rust crate's cpp_file

int main(int argc, char* argv[])
{
    tw_init_metrics();
    QGuiApplication app(argc, argv);
    QQuickStyle::setStyle(qEnvironmentVariable("T2_STYLE", "FluentWinUI3"));
    QQmlApplicationEngine engine;
    engine.addImageProvider("art", tw_make_image_provider());
    const bool empty = qEnvironmentVariable("T2_MODE") == "empty";
    const QUrl url(empty ? "qrc:/qt/qml/Tw/Spike/qml/Empty.qml" : "qrc:/qt/qml/Tw/Spike/qml/Main.qml");
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app,
                     [] { QCoreApplication::exit(-1); }, Qt::QueuedConnection);
    engine.load(url);
    return app.exec();
}

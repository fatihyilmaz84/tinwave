// T-0011 spike: the platform facts a Theme needs that Qt 6.8 does not expose, plus the Windows
// "WindowEffects" prototype (Mica). Read-only: nothing here changes a Windows setting.
#pragma once
#include <QAbstractNativeEventFilter>
#include <QObject>
#include <QVariantMap>
#include <QQuickWindow>
#include <QtGui/qaccessibilityhints.h>
#include <QtQml/qqmlregistration.h>

class PlatformInfo : public QObject, public QAbstractNativeEventFilter
{
    Q_OBJECT
    Q_PROPERTY(bool reducedMotion READ reducedMotion NOTIFY changed)
    Q_PROPERTY(bool highContrast READ highContrast NOTIFY changed)
    Q_PROPERTY(QString highContrastScheme READ highContrastScheme NOTIFY changed)
    Q_PROPERTY(bool transparencyEffects READ transparencyEffects NOTIFY changed)
    Q_PROPERTY(int windowsBuild READ windowsBuild CONSTANT)
    Q_PROPERTY(bool micaSupported READ micaSupported CONSTANT)
    Q_PROPERTY(QString dwmAccent READ dwmAccent NOTIFY changed)
    Q_PROPERTY(QString qtHints READ qtHints NOTIFY changed)
    Q_PROPERTY(QVariantMap options READ options CONSTANT)
    Q_PROPERTY(int changeCount READ changeCount NOTIFY changed)

public:
    explicit PlatformInfo(QObject *parent = nullptr);
    static PlatformInfo *instance();

    bool reducedMotion() const { return m_reducedMotion; }
    bool highContrast() const { return m_highContrast; }
    QString highContrastScheme() const { return m_hcScheme; }
    bool transparencyEffects() const { return m_transparency; }
    int windowsBuild() const { return m_build; }
    bool micaSupported() const { return m_build >= 22621; }
    QString dwmAccent() const { return m_dwmAccent; }
    QString qtHints() const;
    QVariantMap options() const { return m_options; }
    void setOptions(const QVariantMap &o) { m_options = o; }
    int changeCount() const { return m_changeCount; }

    // WindowEffects prototype. Returns "ok", "unsupported:<why>" or "error:<hr>".
    Q_INVOKABLE QString applyMica(QQuickWindow *w, bool on);
    static bool enableDarkMenus(int mode);
    // Test hooks (own window only).
    Q_INVOKABLE void simulateSettingChange(QQuickWindow *w, const QString &what);
    Q_INVOKABLE void scheduleProbe(const QString &label, int delayMs, bool dismiss, const QString &how);
    Q_INVOKABLE QString metrics(const QString &tag);   // prints + returns working set / private bytes
    Q_INVOKABLE void log(const QString &line);
    Q_INVOKABLE void quitLater(int ms);
    Q_INVOKABLE void saveShot(QQuickWindow *w, const QString &path, const QString &whichGrab);

    bool nativeEventFilter(const QByteArray &eventType, void *message, qintptr *result) override;

    void refresh();
    static double processAgeMs();   // ms since this process was created
    static QString rssLine();

signals:
    void changed();

private:
    QVariantMap m_options;
    bool m_reducedMotion = false;
    bool m_highContrast = false;
    QString m_hcScheme;
    bool m_transparency = true;
    int m_build = 0;
    QString m_dwmAccent;
    int m_changeCount = 0;
};

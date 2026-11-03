#pragma once
// Interface between the Rust side (src/app/artwork_bridge.rs, through cxx) and the Qt Quick image provider.
// Rust never sees the Qt Quick class hierarchy: it gets an owning handle to the shared state, and one factory that
// returns the provider object (ownership goes to the QML engine).
#include <QtQml/QQmlImageProviderBase>

#include <cstdint>
#include <memory>

struct ProviderShared;

// Owning handle to a provider's shared state. Independent of the engine: it stays valid after the engine took the
// provider and after the engine was destroyed, so the UI owner can quiesce at any point.
struct ProviderHandle
{
    explicit ProviderHandle(std::shared_ptr<ProviderShared> shared);
    ~ProviderHandle();
    ProviderHandle(const ProviderHandle&) = delete;
    ProviderHandle& operator=(const ProviderHandle&) = delete;

    std::shared_ptr<ProviderShared> shared;
};

// Creates the shared state. `rustCtx` is an Arc::into_raw pointer owned by the new state (released with
// tw_provider_ctx_release when the last user is gone). `poolThreads` is clamped to at least 1. `maxEdge` is the
// largest thumbnail edge in pixels a request may ask for; Rust owns the value (ProviderCore::max_edge) and larger
// requests are refused as Image.Error before anything is allocated. There is no default on this side.
std::unique_ptr<ProviderHandle> tw_provider_handle_new(const uint8_t* rustCtx, int32_t poolThreads, int32_t maxEdge);

// Creates the QQuickAsyncImageProvider sharing the handle's state. The QML engine owns the result.
QQmlImageProviderBase* tw_provider_make(const ProviderHandle& handle);

// GUI thread only: set closing and wait until no worker thread is running. Call after QGuiApplication::exec()
// returned and BEFORE the QML engine is destroyed (ADR 0008 rule 5). Idempotent.
void tw_provider_quiesce(const ProviderHandle& handle);

// Counters, see ProviderShared. `which`: 0 responses, 1 finished, 2 duplicates, 3 cancels, 4 tasksRun, 5 tasksSkipped.
uint64_t tw_provider_counter(const ProviderHandle& handle, uint32_t which);

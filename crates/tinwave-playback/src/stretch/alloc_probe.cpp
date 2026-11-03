// TEST BINARIES ONLY (build.rs passes this object to the linker through `rustc-link-arg-tests`;
// it is never part of the library or of the application).
//
// Replaces the global C++ `operator new` / `operator delete` so a test can count the heap
// operations the C++ side makes on the calling thread. The Rust counting allocator
// (tinwave-test-alloc) cannot see them: C++ allocations go to the C runtime, not to Rust's
// `#[global_allocator]`. Counting mirrors `count_allocations`: one per allocation and one per
// free, calling thread only.
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <new>

namespace {
thread_local std::uint64_t t_ops = 0;

void *take(std::size_t n) {
    ++t_ops;
    void *p = std::malloc(n != 0 ? n : 1);
    if (p == nullptr) throw std::bad_alloc();
    return p;
}

void give(void *p) noexcept {
    // Freeing nullptr is a no-op and not a heap operation.
    if (p != nullptr) {
        ++t_ops;
        std::free(p);
    }
}
}  // namespace

extern "C" std::uint64_t tw_probe_cpp_heap_ops() { return t_ops; }

void *operator new(std::size_t n) { return take(n); }
void *operator new[](std::size_t n) { return take(n); }
void *operator new(std::size_t n, const std::nothrow_t &) noexcept {
    try {
        return take(n);
    } catch (...) {
        return nullptr;
    }
}
void *operator new[](std::size_t n, const std::nothrow_t &) noexcept {
    try {
        return take(n);
    } catch (...) {
        return nullptr;
    }
}
void operator delete(void *p) noexcept { give(p); }
void operator delete[](void *p) noexcept { give(p); }
void operator delete(void *p, std::size_t) noexcept { give(p); }
void operator delete[](void *p, std::size_t) noexcept { give(p); }
void operator delete(void *p, const std::nothrow_t &) noexcept { give(p); }
void operator delete[](void *p, const std::nothrow_t &) noexcept { give(p); }

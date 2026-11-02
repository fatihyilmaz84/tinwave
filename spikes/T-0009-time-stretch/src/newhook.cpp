// Counts C++ heap allocations (operator new) so the spike can show whether a stretcher allocates while processing.
#include <cstdlib>
#include <cstddef>
#include <new>
extern "C" {
unsigned long long tw_cpp_allocs = 0;
unsigned long long tw_cpp_allocs_get() { return tw_cpp_allocs; }
}
void* operator new(std::size_t n) {
    ++tw_cpp_allocs;
    void* p = std::malloc(n ? n : 1);
    if (!p) throw std::bad_alloc();
    return p;
}
void* operator new[](std::size_t n) { return operator new(n); }
void* operator new(std::size_t n, const std::nothrow_t&) noexcept { ++tw_cpp_allocs; return std::malloc(n ? n : 1); }
void operator delete(void* p) noexcept { std::free(p); }
void operator delete[](void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }
void operator delete[](void* p, std::size_t) noexcept { std::free(p); }

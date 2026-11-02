// Vectored exception handler: symbolized stack + minidump on fatal exceptions (shared by repro.exe and race.exe).
#pragma once
#include <windows.h>
#include <dbghelp.h>
#include <cstdio>

static inline LONG WINAPI onException(EXCEPTION_POINTERS* ep)
{
    const DWORD code = ep->ExceptionRecord->ExceptionCode;
    if (code != EXCEPTION_ACCESS_VIOLATION && code != 0xC0000409 && code != 0xC0000374 && code != EXCEPTION_STACK_OVERFLOW)
        return EXCEPTION_CONTINUE_SEARCH;
    static LONG once = 0;
    if (InterlockedExchange(&once, 1)) return EXCEPTION_CONTINUE_SEARCH;
    HANDLE proc = GetCurrentProcess();
    char logPath[MAX_PATH];
    const char* dir = getenv("TW_CRASH_DIR");
    snprintf(logPath, sizeof logPath, "%s\\crash-%lu.txt", dir ? dir : ".", GetCurrentProcessId());
    FILE* f = fopen(logPath, "w");
    if (!f) f = stderr;
    fprintf(f, "TW CRASH code=0x%08lx addr=%p thread=%lu", code, ep->ExceptionRecord->ExceptionAddress, GetCurrentThreadId());
    if (code == EXCEPTION_ACCESS_VIOLATION && ep->ExceptionRecord->NumberParameters >= 2)
        fprintf(f, " %s addr=%p", ep->ExceptionRecord->ExceptionInformation[0] ? "write" : "read",
                (void*)ep->ExceptionRecord->ExceptionInformation[1]);
    fprintf(f, "\n");
    SymSetOptions(SYMOPT_UNDNAME | SYMOPT_LOAD_LINES | SYMOPT_DEFERRED_LOADS);
    SymInitialize(proc, nullptr, TRUE);
    CONTEXT ctx = *ep->ContextRecord;
    STACKFRAME64 sf = {};
    sf.AddrPC.Offset = ctx.Rip; sf.AddrFrame.Offset = ctx.Rbp; sf.AddrStack.Offset = ctx.Rsp;
    sf.AddrPC.Mode = sf.AddrFrame.Mode = sf.AddrStack.Mode = AddrModeFlat;
    for (int i = 0; i < 40; ++i) {
        if (!StackWalk64(IMAGE_FILE_MACHINE_AMD64, proc, GetCurrentThread(), &sf, &ctx, nullptr, SymFunctionTableAccess64, SymGetModuleBase64, nullptr)) break;
        if (!sf.AddrPC.Offset) break;
        char buf[sizeof(SYMBOL_INFO) + 256];
        SYMBOL_INFO* s = (SYMBOL_INFO*)buf; s->SizeOfStruct = sizeof(SYMBOL_INFO); s->MaxNameLen = 255;
        DWORD64 disp = 0;
        IMAGEHLP_MODULE64 mi = {}; mi.SizeOfStruct = sizeof mi;
        DWORD64 base = SymGetModuleBase64(proc, sf.AddrPC.Offset);
        SymGetModuleInfo64(proc, sf.AddrPC.Offset, &mi);
        const char* name = SymFromAddr(proc, sf.AddrPC.Offset, &disp, s) ? s->Name : "?";
        IMAGEHLP_LINE64 ln = {}; ln.SizeOfStruct = sizeof ln; DWORD ld = 0;
        char lnbuf[300] = "";
        if (SymGetLineFromAddr64(proc, sf.AddrPC.Offset, &ld, &ln)) snprintf(lnbuf, sizeof lnbuf, "  %s:%lu", ln.FileName, ln.LineNumber);
        fprintf(f, "  #%02d %s+0x%llx  [%s rva=0x%llx]%s\n", i, name, (unsigned long long)disp, mi.ModuleName, (unsigned long long)(sf.AddrPC.Offset - base), lnbuf);
    }
    fflush(f);
    char dumpPath[MAX_PATH];
    snprintf(dumpPath, sizeof dumpPath, "%s\\crash-%lu.dmp", dir ? dir : ".", GetCurrentProcessId());
    HANDLE h = CreateFileA(dumpPath, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h != INVALID_HANDLE_VALUE) {
        MINIDUMP_EXCEPTION_INFORMATION mei = {GetCurrentThreadId(), ep, FALSE};
        MiniDumpWriteDump(proc, GetCurrentProcessId(), h, (MINIDUMP_TYPE)(MiniDumpWithDataSegs | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules), &mei, nullptr, nullptr);
        CloseHandle(h);
    }
    if (f != stderr) fclose(f);
    fprintf(stderr, "TW CRASH (see %s)\n", logPath);
    TerminateProcess(proc, 0xC0000005);   // skip WER: no system-wide side effects
    return EXCEPTION_CONTINUE_SEARCH;
}


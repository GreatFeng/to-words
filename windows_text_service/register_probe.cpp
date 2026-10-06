// Development-only registrar: reports the HRESULT that regsvr32 hides.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <iomanip>
#include <iostream>

int wmain(int argc, wchar_t** argv) {
    if (argc != 3 || (wcscmp(argv[1], L"register") && wcscmp(argv[1], L"unregister"))) {
        std::wcerr << L"Usage: tsf_register_probe.exe register|unregister <DLL path>\n";
        return 2;
    }
    HMODULE module = LoadLibraryW(argv[2]);
    if (!module) {
        std::wcerr << L"LoadLibrary failed: " << GetLastError() << L'\n';
        return 3;
    }
    const char* name = wcscmp(argv[1], L"register") == 0
        ? "DllRegisterServer" : "DllUnregisterServer";
    using Entry = HRESULT(__stdcall*)();
    auto entry = reinterpret_cast<Entry>(GetProcAddress(module, name));
    if (!entry) {
        std::wcerr << L"Missing DLL export.\n";
        FreeLibrary(module);
        return 4;
    }
    HRESULT result = entry();
    std::wcout << L"HRESULT=0x" << std::hex << std::setw(8) << std::setfill(L'0')
               << static_cast<unsigned long>(result) << L'\n';
    FreeLibrary(module);
    return SUCCEEDED(result) ? 0 : 5;
}

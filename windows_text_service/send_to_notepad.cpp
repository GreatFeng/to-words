// Sends a test phrase to the active Notepad TSF service; never sends key events.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <iostream>
#include <string>

#pragma comment(lib, "user32.lib")

namespace {
constexpr wchar_t kWindowClass[] = L"ToWordsNotepadTsfPrototype";
constexpr ULONG_PTR kMessageTag = 0x54535754;
struct Search {
    DWORD process_id;
    HWND window = nullptr;
    unsigned other_bridges = 0;
};
BOOL CALLBACK find_bridge(HWND window, LPARAM parameter) {
    auto* search = reinterpret_cast<Search*>(parameter);
    wchar_t class_name[128]{};
    if (GetClassNameW(window, class_name, 128) &&
        wcscmp(class_name, kWindowClass) == 0) {
        DWORD process_id = 0;
        GetWindowThreadProcessId(window, &process_id);
        if (process_id == search->process_id) {
            search->window = window;
            return FALSE;
        }
        ++search->other_bridges;
    }
    return TRUE;
}
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 2 || !argv[1][0]) {
        std::wcerr << L"Usage: to_words_tsf_send.exe <text>\n";
        return 2;
    }
    HWND foreground = GetForegroundWindow();
    Search search{};
    if (!foreground || !GetWindowThreadProcessId(foreground, &search.process_id)) {
        std::wcerr << L"No foreground text window.\n";
        return 3;
    }
    EnumWindows(find_bridge, reinterpret_cast<LPARAM>(&search));
    if (!search.window) {
        if (search.other_bridges) {
            std::wcerr << L"TSF bridge is active in another process, not foreground.\n";
            return 7;
        }
        std::wcerr << L"No active to_words Notepad TSF bridge on this desktop.\n";
        return 4;
    }
    const std::wstring text(argv[1]);
    COPYDATASTRUCT data{};
    data.dwData = kMessageTag;
    data.cbData = static_cast<DWORD>((text.size() + 1) * sizeof(wchar_t));
    data.lpData = const_cast<wchar_t*>(text.c_str());
    DWORD_PTR result = 0;
    if (!SendMessageTimeoutW(search.window, WM_COPYDATA, 0,
                             reinterpret_cast<LPARAM>(&data),
                             SMTO_ABORTIFHUNG | SMTO_BLOCK, 1000, &result) || !result) {
        std::wcerr << L"TSF edit session was not accepted.\n";
        return 5;
    }
    return 0;
}

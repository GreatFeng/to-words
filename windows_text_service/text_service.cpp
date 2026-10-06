// Notepad-only TSF proof of concept. Never enable this DLL inside a game.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <msctf.h>
#include <atomic>
#include <cstdio>
#include <cwchar>
#include <new>
#include <string>
#include <utility>

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "advapi32.lib")
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "uuid.lib")

namespace {
constexpr CLSID kService = {0x45f1de8a, 0xb258, 0x4b62, {0xa3, 0x61, 0x20, 0x47, 0x89, 0x16, 0xe9, 0x4c}};
constexpr GUID kProfile = {0xef286879, 0xf1c5, 0x4bac, {0x95, 0x0c, 0x7a, 0x64, 0x81, 0xcf, 0x02, 0x96}};
constexpr wchar_t kWindowClass[] = L"ToWordsNotepadTsfPrototype";
constexpr ULONG_PTR kMessageTag = 0x54535754;
constexpr LANGID kLanguage = MAKELANGID(LANG_CHINESE, SUBLANG_CHINESE_SIMPLIFIED);
HINSTANCE g_instance = nullptr;
std::atomic_ulong g_objects{0};
std::atomic_ulong g_locks{0};

void log_status(const char* stage, HRESULT result) {
    // Prototype-only diagnostics: process, stage, and HRESULT; never user text.
    wchar_t path[MAX_PATH]{};
    wchar_t process[MAX_PATH]{};
    DWORD chars = GetTempPathW(MAX_PATH, path);
    if (!chars || chars >= MAX_PATH ||
        !GetModuleFileNameW(nullptr, process, MAX_PATH)) return;
    if (wcscat_s(path, L"to_words_tsf_notepad.log")) return;
    HANDLE file = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                              nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return;
    const wchar_t* name = wcsrchr(process, L'\\');
    char line[256]{};
    int bytes = sprintf_s(line, "pid=%lu stage=%s hr=0x%08lx process=%ls\r\n",
                          GetCurrentProcessId(), stage,
                          static_cast<unsigned long>(result), name ? name + 1 : process);
    if (bytes > 0) {
        DWORD written = 0;
        WriteFile(file, line, static_cast<DWORD>(bytes), &written, nullptr);
    }
    CloseHandle(file);
}

void log_activation(const char* stage) { log_status(stage, S_OK); }

bool is_notepad() {
    wchar_t path[MAX_PATH]{};
    if (!GetModuleFileNameW(nullptr, path, MAX_PATH)) return false;
    const wchar_t* name = wcsrchr(path, L'\\');
    return _wcsicmp(name ? name + 1 : path, L"notepad.exe") == 0;
}

class EditSession final : public ITfEditSession {
public:
    EditSession(ITfContext* context, std::wstring text) : context_(context), text_(std::move(text)) {
        context_->AddRef();
        ++g_objects;
    }
    ~EditSession() { context_->Release(); --g_objects; }

    STDMETHODIMP QueryInterface(REFIID iid, void** out) override {
        if (!out) return E_POINTER;
        *out = nullptr;
        if (iid != IID_IUnknown && iid != IID_ITfEditSession) return E_NOINTERFACE;
        *out = static_cast<ITfEditSession*>(this);
        AddRef();
        return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
    STDMETHODIMP_(ULONG) Release() override {
        ULONG remaining = --refs_;
        if (!remaining) delete this;
        return remaining;
    }
    STDMETHODIMP DoEditSession(TfEditCookie cookie) override {
        TF_SELECTION selection{};
        ULONG fetched = 0;
        HRESULT hr = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1,
                                            &selection, &fetched);
        log_status("get_selection", hr);
        if (FAILED(hr)) return hr;
        if (fetched != 1 || !selection.range) return TF_E_NOSELECTION;
        hr = selection.range->SetText(cookie, 0, text_.c_str(),
                                      static_cast<LONG>(text_.size()));
        log_status("set_text", hr);
        if (SUCCEEDED(hr)) {
            HRESULT move = selection.range->Collapse(cookie, TF_ANCHOR_END);
            log_status("collapse", move);
            if (SUCCEEDED(move)) {
                HRESULT select = context_->SetSelection(cookie, 1, &selection);
                log_status("set_selection", select);
            }
        }
        selection.range->Release();
        return hr;
    }
private:
    std::atomic_ulong refs_{1};
    ITfContext* context_;
    std::wstring text_;
};

class TextService final : public ITfTextInputProcessor {
public:
    TextService() { ++g_objects; }
    ~TextService() { Deactivate(); --g_objects; }

    STDMETHODIMP QueryInterface(REFIID iid, void** out) override {
        if (!out) return E_POINTER;
        *out = nullptr;
        if (iid != IID_IUnknown && iid != IID_ITfTextInputProcessor) return E_NOINTERFACE;
        *out = static_cast<ITfTextInputProcessor*>(this);
        AddRef();
        return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
    STDMETHODIMP_(ULONG) Release() override {
        ULONG remaining = --refs_;
        if (!remaining) delete this;
        return remaining;
    }
    STDMETHODIMP Activate(ITfThreadMgr* manager, TfClientId client) override {
        log_activation("activate_called");
        if (!is_notepad()) return S_OK;  // Hard boundary: no IPC/window in any other process.
        if (!manager) return E_INVALIDARG;
        manager_ = manager;
        manager_->AddRef();
        client_ = client;
        WNDCLASSW cls{};
        cls.lpfnWndProc = window_proc;
        cls.hInstance = g_instance;
        cls.lpszClassName = kWindowClass;
        if (!RegisterClassW(&cls) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
            DWORD error = GetLastError();
            Deactivate();
            return HRESULT_FROM_WIN32(error);
        }
        window_ = CreateWindowExW(0, kWindowClass, kWindowClass, WS_POPUP,
                                  0, 0, 0, 0, nullptr, nullptr, g_instance, this);
        if (!window_) {
            log_activation("window_failed");
            DWORD error = GetLastError();
            Deactivate();
            return HRESULT_FROM_WIN32(error);
        }
        log_activation("bridge_created");
        return S_OK;
    }
    STDMETHODIMP Deactivate() override {
        if (window_) {
            DestroyWindow(window_);
            window_ = nullptr;
        }
        if (manager_) {
            manager_->Release();
            manager_ = nullptr;
        }
        client_ = TF_CLIENTID_NULL;
        return S_OK;
    }

private:
    static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wp, LPARAM lp) {
        if (message == WM_NCCREATE) {
            auto* create = reinterpret_cast<CREATESTRUCTW*>(lp);
            SetWindowLongPtrW(window, GWLP_USERDATA,
                              reinterpret_cast<LONG_PTR>(create->lpCreateParams));
            return TRUE;
        }
        auto* self = reinterpret_cast<TextService*>(GetWindowLongPtrW(window, GWLP_USERDATA));
        if (message == WM_COPYDATA && self) {
            const auto* data = reinterpret_cast<const COPYDATASTRUCT*>(lp);
            if (!data || data->dwData != kMessageTag || !data->lpData ||
                data->cbData < sizeof(wchar_t) || data->cbData > 32768 ||
                data->cbData % sizeof(wchar_t)) return FALSE;
            const auto* chars = static_cast<const wchar_t*>(data->lpData);
            size_t count = data->cbData / sizeof(wchar_t);
            if (chars[count - 1] != L'\0') return FALSE;
            return SUCCEEDED(self->insert(std::wstring(chars, count - 1)));
        }
        return DefWindowProcW(window, message, wp, lp);
    }

    HRESULT insert(std::wstring text) {
        if (!manager_ || text.empty() || !is_notepad()) return E_FAIL;
        ITfDocumentMgr* document = nullptr;
        HRESULT hr = manager_->GetFocus(&document);
        if (FAILED(hr) || !document) return FAILED(hr) ? hr : E_FAIL;
        ITfContext* context = nullptr;
        hr = document->GetTop(&context);
        document->Release();
        if (FAILED(hr) || !context) return FAILED(hr) ? hr : E_FAIL;
        auto* session = new (std::nothrow) EditSession(context, std::move(text));
        if (!session) {
            context->Release();
            return E_OUTOFMEMORY;
        }
        HRESULT session_result = E_FAIL;
        hr = context->RequestEditSession(client_, session,
                                         TF_ES_ASYNC | TF_ES_READWRITE, &session_result);
        log_status("request_result", hr);
        log_status("session_result", session_result);
        session->Release();
        context->Release();
        return FAILED(hr) ? hr : session_result;
    }

    std::atomic_ulong refs_{1};
    ITfThreadMgr* manager_ = nullptr;
    TfClientId client_ = TF_CLIENTID_NULL;
    HWND window_ = nullptr;
};

class Factory final : public IClassFactory {
public:
    Factory() { ++g_objects; }
    ~Factory() { --g_objects; }
    STDMETHODIMP QueryInterface(REFIID iid, void** out) override {
        if (!out) return E_POINTER;
        *out = nullptr;
        if (iid != IID_IUnknown && iid != IID_IClassFactory) return E_NOINTERFACE;
        *out = static_cast<IClassFactory*>(this);
        AddRef();
        return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
    STDMETHODIMP_(ULONG) Release() override {
        ULONG remaining = --refs_;
        if (!remaining) delete this;
        return remaining;
    }
    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID iid, void** out) override {
        if (outer) return CLASS_E_NOAGGREGATION;
        auto* service = new (std::nothrow) TextService();
        if (!service) return E_OUTOFMEMORY;
        HRESULT hr = service->QueryInterface(iid, out);
        service->Release();
        return hr;
    }
    STDMETHODIMP LockServer(BOOL lock) override {
        if (lock) ++g_locks; else --g_locks;
        return S_OK;
    }
private:
    std::atomic_ulong refs_{1};
};

HRESULT set_string(HKEY key, const wchar_t* name, const wchar_t* value) {
    return HRESULT_FROM_WIN32(RegSetValueExW(key, name, 0, REG_SZ,
        reinterpret_cast<const BYTE*>(value),
        static_cast<DWORD>((wcslen(value) + 1) * sizeof(wchar_t))));
}

HRESULT register_com() {
    wchar_t dll[MAX_PATH]{};
    if (!GetModuleFileNameW(g_instance, dll, MAX_PATH)) return HRESULT_FROM_WIN32(GetLastError());
    wchar_t clsid[64]{};
    StringFromGUID2(kService, clsid, 64);
    std::wstring key_path = std::wstring(L"Software\\Classes\\CLSID\\") + clsid;
    HKEY key = nullptr;
    LONG error = RegCreateKeyExW(HKEY_CURRENT_USER, key_path.c_str(), 0, nullptr, 0,
                                  KEY_WRITE, nullptr, &key, nullptr);
    if (error != ERROR_SUCCESS) return HRESULT_FROM_WIN32(error);
    HRESULT hr = set_string(key, nullptr, L"to_words Notepad-only TSF prototype");
    RegCloseKey(key);
    if (FAILED(hr)) return hr;
    key_path += L"\\InprocServer32";
    error = RegCreateKeyExW(HKEY_CURRENT_USER, key_path.c_str(), 0, nullptr, 0,
                            KEY_WRITE, nullptr, &key, nullptr);
    if (error != ERROR_SUCCESS) return HRESULT_FROM_WIN32(error);
    hr = set_string(key, nullptr, dll);
    if (SUCCEEDED(hr)) hr = set_string(key, L"ThreadingModel", L"Apartment");
    RegCloseKey(key);
    return hr;
}

void unregister_com() {
    wchar_t clsid[64]{};
    StringFromGUID2(kService, clsid, 64);
    std::wstring path = std::wstring(L"Software\\Classes\\CLSID\\") + clsid;
    RegDeleteTreeW(HKEY_CURRENT_USER, path.c_str());
}
}  // namespace

BOOL APIENTRY DllMain(HINSTANCE instance, DWORD reason, void*) {
    if (reason == DLL_PROCESS_ATTACH) {
        g_instance = instance;
        DisableThreadLibraryCalls(instance);
    }
    return TRUE;
}

extern "C" HRESULT __stdcall DllGetClassObject(REFCLSID clsid, REFIID iid, void** out) {
    if (clsid != kService) return CLASS_E_CLASSNOTAVAILABLE;
    auto* factory = new (std::nothrow) Factory();
    if (!factory) return E_OUTOFMEMORY;
    HRESULT hr = factory->QueryInterface(iid, out);
    factory->Release();
    return hr;
}

extern "C" HRESULT __stdcall DllCanUnloadNow() {
    return g_objects == 0 && g_locks == 0 ? S_OK : S_FALSE;
}

extern "C" HRESULT __stdcall DllRegisterServer() {
    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) return init;
    HRESULT hr = register_com();
    std::fwprintf(stderr, L"register_com: 0x%08lx\n", static_cast<unsigned long>(hr));
    if (FAILED(hr)) {
        if (SUCCEEDED(init)) CoUninitialize();
        return hr;
    }
    ITfInputProcessorProfiles* profiles = nullptr;
    hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                          IID_ITfInputProcessorProfiles, reinterpret_cast<void**>(&profiles));
    std::fwprintf(stderr, L"profiles instance: 0x%08lx\n", static_cast<unsigned long>(hr));
    if (FAILED(hr)) {
        unregister_com();
        if (SUCCEEDED(init)) CoUninitialize();
        return hr;
    }
    hr = profiles->Register(kService);
    std::fwprintf(stderr, L"profiles register: 0x%08lx\n", static_cast<unsigned long>(hr));
    constexpr wchar_t label[] = L"to_words (Notepad test)";
    bool modern_profile_registered = false;
    if (FAILED(hr)) {
        // Windows may refuse the legacy TSF registration API for a user-scoped TIP.
        ITfInputProcessorProfileMgr* modern = nullptr;
        HRESULT query = profiles->QueryInterface(IID_ITfInputProcessorProfileMgr,
                                                reinterpret_cast<void**>(&modern));
        if (SUCCEEDED(query)) {
            hr = modern->RegisterProfile(kService, kLanguage, kProfile, label,
                                         static_cast<ULONG>(wcslen(label)), nullptr, 0, 0,
                                         nullptr, 0, FALSE, 0);
            std::fwprintf(stderr, L"modern register profile: 0x%08lx\n",
                          static_cast<unsigned long>(hr));
            modern_profile_registered = SUCCEEDED(hr);
            modern->Release();
        }
    }
    if (SUCCEEDED(hr) && !modern_profile_registered) {
        hr = profiles->AddLanguageProfile(kService, kLanguage, kProfile, label,
                                          static_cast<ULONG>(wcslen(label)), nullptr, 0, 0);
        std::fwprintf(stderr, L"add profile: 0x%08lx\n", static_cast<unsigned long>(hr));
    }
    if (SUCCEEDED(hr)) {
        hr = profiles->EnableLanguageProfile(kService, kLanguage, kProfile, TRUE);
        std::fwprintf(stderr, L"enable profile: 0x%08lx\n", static_cast<unsigned long>(hr));
    }
    profiles->Release();
    if (FAILED(hr)) {
        unregister_com();
        if (SUCCEEDED(init)) CoUninitialize();
        return hr;
    }
    ITfCategoryMgr* categories = nullptr;
    hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                          IID_ITfCategoryMgr, reinterpret_cast<void**>(&categories));
    std::fwprintf(stderr, L"category instance: 0x%08lx\n", static_cast<unsigned long>(hr));
    if (FAILED(hr)) {
        unregister_com();
        if (SUCCEEDED(init)) CoUninitialize();
        return hr;
    }
    hr = categories->RegisterCategory(kService, GUID_TFCAT_TIP_KEYBOARD, kService);
    std::fwprintf(stderr, L"register category: 0x%08lx\n", static_cast<unsigned long>(hr));
    categories->Release();
    if (FAILED(hr)) unregister_com();
    if (SUCCEEDED(init)) CoUninitialize();
    return hr;
}

extern "C" HRESULT __stdcall DllUnregisterServer() {
    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) return init;
    ITfCategoryMgr* categories = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                   IID_ITfCategoryMgr, reinterpret_cast<void**>(&categories)))) {
        categories->UnregisterCategory(kService, GUID_TFCAT_TIP_KEYBOARD, kService);
        categories->Release();
    }
    ITfInputProcessorProfiles* profiles = nullptr;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                   IID_ITfInputProcessorProfiles, reinterpret_cast<void**>(&profiles)))) {
        ITfInputProcessorProfileMgr* modern = nullptr;
        if (SUCCEEDED(profiles->QueryInterface(IID_ITfInputProcessorProfileMgr,
                                              reinterpret_cast<void**>(&modern)))) {
            modern->UnregisterProfile(kService, kLanguage, kProfile, 0);
            modern->Release();
        }
        profiles->RemoveLanguageProfile(kService, kLanguage, kProfile);
        profiles->Unregister(kService);
        profiles->Release();
    }
    unregister_com();
    if (SUCCEEDED(init)) CoUninitialize();
    return S_OK;
}

// Read-only TSF profile diagnostic for the Notepad-only prototype.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <msctf.h>
#include <cstdio>

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "uuid.lib")

namespace {
constexpr CLSID kService = {0x45f1de8a, 0xb258, 0x4b62,
                            {0xa3, 0x61, 0x20, 0x47, 0x89, 0x16, 0xe9, 0x4c}};
constexpr GUID kProfile = {0xef286879, 0xf1c5, 0x4bac,
                           {0x95, 0x0c, 0x7a, 0x64, 0x81, 0xcf, 0x02, 0x96}};
constexpr LANGID kLanguage = MAKELANGID(LANG_CHINESE, SUBLANG_CHINESE_SIMPLIFIED);
}

int wmain() {
    HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(hr)) {
        std::printf("com_init=0x%08lx\n", static_cast<unsigned long>(hr));
        return 1;
    }
    ITfInputProcessorProfiles* profiles = nullptr;
    hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                          IID_ITfInputProcessorProfiles,
                          reinterpret_cast<void**>(&profiles));
    std::printf("profiles=0x%08lx\n", static_cast<unsigned long>(hr));
    if (SUCCEEDED(hr)) {
        BOOL enabled = FALSE;
        hr = profiles->IsEnabledLanguageProfile(kService, kLanguage, kProfile, &enabled);
        std::printf("enabled_hr=0x%08lx enabled=%d\n", static_cast<unsigned long>(hr), enabled);

        LANGID active_language = 0;
        GUID active_guid{};
        hr = profiles->GetActiveLanguageProfile(kService, &active_language, &active_guid);
        std::printf("active_language_hr=0x%08lx lang=0x%04x matches=%d\n",
                    static_cast<unsigned long>(hr), active_language, active_guid == kProfile);

        ITfInputProcessorProfileMgr* manager = nullptr;
        hr = profiles->QueryInterface(IID_ITfInputProcessorProfileMgr,
                                      reinterpret_cast<void**>(&manager));
        std::printf("manager=0x%08lx\n", static_cast<unsigned long>(hr));
        if (SUCCEEDED(hr)) {
            TF_INPUTPROCESSORPROFILE active{};
            hr = manager->GetActiveProfile(GUID_TFCAT_TIP_KEYBOARD, &active);
            std::printf("keyboard_hr=0x%08lx clsid_matches=%d profile_matches=%d "
                        "lang=0x%04x flags=0x%08lx\n", static_cast<unsigned long>(hr),
                        active.clsid == kService, active.guidProfile == kProfile,
                        active.langid, static_cast<unsigned long>(active.dwFlags));
            manager->Release();
        }
        profiles->Release();
    }
    ITfTextInputProcessor* service = nullptr;
    hr = CoCreateInstance(kService, nullptr, CLSCTX_INPROC_SERVER,
                          IID_ITfTextInputProcessor,
                          reinterpret_cast<void**>(&service));
    std::printf("com_service=0x%08lx\n", static_cast<unsigned long>(hr));
    if (service) service->Release();
    CoUninitialize();
    return 0;
}

// Build with the header from the same Santec SDK as the installed SLMFunc.dll.
// Deducing the phase type from ReadWL avoids choosing a calling convention
// from unofficial wrappers. Both calls must agree on that type at compile time.
#include <windows.h>
#include <cstring>
#include <tuple>
#include <type_traits>
#include "SLMFunc.h"

template <typename> struct FnArgs;
template <typename R, typename... Args>
struct FnArgs<R (*)(Args...)> {
    template <size_t I> using Arg = std::tuple_element_t<I, std::tuple<Args...>>;
};

using ReadWL = decltype(&SLM_Ctrl_ReadWL);
using WriteWL = decltype(&SLM_Ctrl_WriteWL);
using Phase = std::remove_pointer_t<FnArgs<ReadWL>::Arg<2>>;
static_assert(sizeof(Phase) == 4);
static_assert(std::is_same_v<Phase, FnArgs<WriteWL>::Arg<2>>);

template <typename F>
F load(HMODULE dll, const char* name) {
    return reinterpret_cast<F>(GetProcAddress(dll, name));
}

// Returns vendor SDK status or a bridge-specific error below -30000.
extern "C" __declspec(dllexport) int kohdalab_santec_wavelength_v2(
    const wchar_t* sdk_path, DWORD device, DWORD request, DWORD* observed,
    DWORD* phase_bits
) {
    HMODULE sdk = LoadLibraryExW(sdk_path, nullptr,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!sdk) return -30001;

    auto open = load<decltype(&SLM_Ctrl_Open)>(sdk, "SLM_Ctrl_Open");
    auto close = load<decltype(&SLM_Ctrl_Close)>(sdk, "SLM_Ctrl_Close");
    auto read = load<ReadWL>(sdk, "SLM_Ctrl_ReadWL");
    auto write = load<WriteWL>(sdk, "SLM_Ctrl_WriteWL");
    if (!open || !close || !read || !write) {
        FreeLibrary(sdk);
        return -30002;
    }
    int result = open(device);
    if (result != 0) {
        FreeLibrary(sdk);
        return result;
    }
    Phase phase{};
    result = read(device, observed, &phase);
    if (result == 0) {
        std::memcpy(phase_bits, &phase, sizeof(phase));
    }
    if (result == 0 && request != 0 && request != *observed) {
        result = write(device, request, phase);
        if (result == 0) {
            const ULONGLONG deadline = GetTickCount64() + 50000;
            do {
                Sleep(1000);
                DWORD read_nm{};
                Phase read_phase{};
                result = read(device, &read_nm, &read_phase);
                if (result == 0) {
                    *observed = read_nm;
                    std::memcpy(phase_bits, &read_phase, sizeof(read_phase));
                    if (read_nm == request && read_phase == phase) break;
                }
                // 2 is SLM_BS (the device is busy).
                if (result != 0 && result != 2) break;
                result = -30003;
            } while (GetTickCount64() < deadline);
        }
    }
    const int closed = close(device);
    FreeLibrary(sdk);
    return result == 0 ? closed : result;
}

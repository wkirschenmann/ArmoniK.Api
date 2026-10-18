//! Whether this process can run code compiled for x86-64-v4, asked before choosing between an x64
//! library's baseline and x86-64-v4 builds. Compiled for the baseline, so that asking runs no
//! instruction the CPU may lack.

/// 1 when the CPU has every feature `-C target-cpu=x86-64-v4` compiles for beyond the x86-64
/// baseline, AVX-512 F, BW, CD, DQ and VL among them, and the operating system saves the AVX and
/// AVX-512 registers; 0 otherwise, and on any other architecture.
#[no_mangle]
pub extern "C" fn armonik_cpu_probe_runs_x86_64_v4() -> u8 {
    u8::from(runs_x86_64_v4())
}

#[cfg(target_arch = "x86_64")]
fn runs_x86_64_v4() -> bool {
    // The detection of an AVX or AVX-512 feature also asks XGETBV whether the OS enabled its state.
    use std::arch::is_x86_feature_detected as has;
    has!("cmpxchg16b")
        && has!("popcnt")
        && has!("sse3")
        && has!("ssse3")
        && has!("sse4.1")
        && has!("sse4.2")
        && has!("avx")
        && has!("avx2")
        && has!("bmi1")
        && has!("bmi2")
        && has!("f16c")
        && has!("fma")
        && has!("lzcnt")
        && has!("movbe")
        && has!("xsave")
        && has!("avx512f")
        && has!("avx512bw")
        && has!("avx512cd")
        && has!("avx512dq")
        && has!("avx512vl")
}

#[cfg(not(target_arch = "x86_64"))]
fn runs_x86_64_v4() -> bool {
    false
}

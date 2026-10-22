//! How many logical processors the process may run on.

/// The logical processors the process may run on, at least 1.
///
/// On Windows, the processors of its affinity mask: the standard library counts every processor
/// of the system there, and a process started with an affinity mask would be given a thread per
/// processor it cannot use. Elsewhere, the standard library's count, which honours the affinity
/// and the quota of a cgroup.
pub(crate) fn available() -> usize {
    #[cfg(windows)]
    if let Some(count) = in_affinity_mask() {
        return count;
    }
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// The processors of the process's affinity mask, or none when Windows does not say. The mask
/// covers the processor group the process starts in, as the standard library's count does.
#[cfg(windows)]
fn in_affinity_mask() -> Option<usize> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};

    let mut process = 0usize;
    let mut system = 0usize;
    // SAFETY: the pseudo-handle of the current process needs no closing, and both pointers are to
    // locals that outlive the call.
    let answered =
        unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) } != 0;
    let count = process.count_ones() as usize;
    (answered && count > 0).then_some(count)
}

#[cfg(test)]
mod tests {
    #[test]
    fn at_least_one_processor_and_no_more_than_the_system_has() {
        let available = super::available();
        assert!(available >= 1);
        // The standard library counts every processor of the system on Windows, its affinity
        // elsewhere: either way, an upper bound.
        let system = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        assert!(available <= system, "{available} > {system}");
    }
}

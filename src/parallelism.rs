// Copyright (C) 2020-2026 Andy Kurnia.

// how many threads to run at once by default: one a logical processor, or 1
// when that count cannot be had. That is std's count, except on Windows, where
// std counts one processor group's logical processors, 64 at most; there it is
// the active processors of the groups this program's threads may run in, which
// since Windows 11 and Windows Server 2022 is every group by default, and on an
// older Windows the one group it started in.
#[inline]
pub fn available() -> usize {
    #[cfg(windows)]
    if let Some(n) = in_its_groups() {
        return n;
    }
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

// the active processors of the groups this program's threads may run in, or
// None when Windows will not say, as under Wine
#[cfg(windows)]
fn in_its_groups() -> Option<usize> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        // this process, as a handle that needs no closing
        safe fn GetCurrentProcess() -> *mut c_void;
        // the most processor groups the system can have
        safe fn GetMaximumProcessorGroupCount() -> u16;
        // the groups a process's threads may run in; 0 on failure
        fn GetProcessGroupAffinity(
            process: *mut c_void,
            group_count: *mut u16,
            group_array: *mut u16,
        ) -> i32;
        // the active processors in a group; 0 on failure
        safe fn GetActiveProcessorCount(group_number: u16) -> u32;
    }
    let mut group_count = GetMaximumProcessorGroupCount();
    let mut groups = vec![0u16; usize::from(group_count)];
    // group_count says how many groups fit, and then how many were put there
    let found = unsafe {
        GetProcessGroupAffinity(GetCurrentProcess(), &mut group_count, groups.as_mut_ptr())
    };
    if found == 0 {
        return None;
    }
    let n = groups
        .get(..usize::from(group_count))?
        .iter()
        .map(|&group| GetActiveProcessorCount(group) as usize)
        .sum();
    (n != 0).then_some(n)
}

#[cfg(test)]
mod tests {
    #[test]
    #[inline]
    fn every_processor_std_counts_is_counted() {
        let std_count = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        assert!(super::available() >= std_count);
    }

    // Wine does not implement GetProcessGroupAffinity, so this fails there
    #[cfg(windows)]
    #[test]
    #[inline]
    fn windows_says_which_groups_its_threads_may_run_in() {
        assert!(super::in_its_groups().is_some());
    }
}

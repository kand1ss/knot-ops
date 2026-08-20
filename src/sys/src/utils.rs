/// Normalizes a process name for comparison on Windows.
///
/// Windows process names are commonly reported with the `.exe` executable
/// suffix, while callers may provide the executable name without it. This
/// function removes a trailing `.exe` suffix, case-insensitively, so both
/// representations can be compared consistently.
///
/// Only a complete trailing `.exe` suffix is removed. Partial or embedded
/// occurrences are preserved.
///
/// # Examples
///
/// ```rust,ignore
/// # #[cfg(windows)]
/// # {
/// assert_eq!(normalize_process_name("ping.exe"), "ping");
/// assert_eq!(normalize_process_name("PING.EXE"), "PING");
/// assert_eq!(normalize_process_name("ping.Exe"), "ping");
/// assert_eq!(normalize_process_name("myapp.exec"), "myapp.exec");
/// # }
/// ```
#[cfg(windows)]
fn normalize_process_name(name: &str) -> &str {
    const SUFFIX: &str = ".exe";

    if name.len() <= SUFFIX.len() {
        return name;
    }

    let suffix_start = name.len() - SUFFIX.len();
    let (base, suffix) = name.split_at(suffix_start);

    if suffix.eq_ignore_ascii_case(SUFFIX) {
        base
    } else {
        name
    }
}

#[cfg(not(windows))]
fn normalize_process_name(name: &str) -> &str {
    name
}

/// Returns whether two process names match according to the platform-specific
/// process-name comparison rules.
///
/// Comparison is case-insensitive on every supported platform. On Windows,
/// the optional `.exe` suffix is ignored.
///
/// This function should be used instead of comparing process names directly
/// when validating that an operating-system process corresponds to an
/// expected executable name.
pub fn process_names_match(actual: &str, expected: &str) -> bool {
    normalize_process_name(actual).eq_ignore_ascii_case(normalize_process_name(expected))
}

#[cfg(test)]
mod process_name_matching_tests {
    use super::*;

    #[test]
    fn matches_identical_names() {
        assert!(process_names_match("sleep", "sleep"));
    }

    #[test]
    fn matches_case_insensitively() {
        assert!(process_names_match("Sleep", "sleep"));
        assert!(process_names_match("SLEEP", "sleep"));
    }

    #[test]
    fn rejects_different_names() {
        assert!(!process_names_match("sleep", "ping"));
    }

    #[cfg(windows)]
    #[test]
    fn strips_exe_suffix_case_insensitively() {
        assert!(process_names_match("ping.exe", "ping"));
        assert!(process_names_match("ping.EXE", "ping"));
        assert!(process_names_match("PING.Exe", "PING"));
    }

    #[cfg(windows)]
    #[test]
    fn does_not_strip_partial_or_embedded_exe() {
        assert!(!process_names_match("myapp.exec", "myapp"));
    }

    #[cfg(not(windows))]
    #[test]
    fn does_not_strip_exe_suffix_on_unix() {
        assert!(!process_names_match("myapp.exe", "myapp"));
    }
}

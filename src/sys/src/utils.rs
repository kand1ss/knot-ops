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

use knot_core::consts::KNOT_FOLDER_NAME;
use std::path::{Path, PathBuf};
use tracing::{debug, info, instrument, warn};

/// Recursively searches for the `.knot` configuration directory starting from the given path
/// and moving up the directory tree.
///
/// Returns the path to the `.knot` directory if found, or `None` otherwise.
#[instrument(skip_all)]
pub fn recursively_find_knot(start: &Path) -> Option<PathBuf> {
    let mut current_path = start.to_path_buf();
    debug!("Started recursive search of .knot directory");

    loop {
        let potential_knot = current_path.join(KNOT_FOLDER_NAME);
        if potential_knot.is_dir() {
            info!("Directory was found at '{}'", potential_knot.display());
            return Some(potential_knot);
        }

        if !current_path.pop() {
            warn!("Directory .knot was not found");
            break;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_temp_dir(suffix: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let thread_id = std::thread::current().id();
        path.push(format!("knot-utils-test-{}-{:?}", suffix, thread_id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn test_recursively_find_knot_in_current_dir() {
        let root = setup_temp_dir("current");
        let knot_dir = root.join(KNOT_FOLDER_NAME);
        fs::create_dir(&knot_dir).unwrap();

        let result = recursively_find_knot(&root);
        assert_eq!(result, Some(knot_dir));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_recursively_find_knot_in_parent() {
        let root = setup_temp_dir("parent");
        let knot_dir = root.join(KNOT_FOLDER_NAME);
        fs::create_dir(&knot_dir).unwrap();

        let nested = root.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();

        let result = recursively_find_knot(&nested);
        assert_eq!(result, Some(knot_dir));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_recursively_find_knot_not_found() {
        let root = setup_temp_dir("not_found");
        let nested = root.join("x").join("y").join("z");
        fs::create_dir_all(&nested).unwrap();

        let result = recursively_find_knot(&nested);
        assert_eq!(result, None);

        fs::remove_dir_all(&root).unwrap();
    }
}

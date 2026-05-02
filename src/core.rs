use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct Atomic<T> {
    value: Arc<Mutex<T>>,
}

impl<T> Atomic<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Arc::new(Mutex::new(value)),
        }
    }

    pub fn with<R>(&self, closure: impl FnOnce(&mut T) -> R) -> R {
        let mut value = self.value.lock().expect("atomic mutex poisoned");
        closure(&mut value)
    }
}

impl<T: Clone> Atomic<T> {
    pub fn get(&self) -> T {
        self.value.lock().expect("atomic mutex poisoned").clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortedArray<T> {
    pub value: Vec<T>,
}

impl<T: Ord> SortedArray<T> {
    pub fn new<I: IntoIterator<Item = T>>(value: I) -> Self {
        let mut value = value.into_iter().collect::<Vec<_>>();
        value.sort();
        Self { value }
    }

    pub fn first_index_where<F>(&self, mut predicate: F) -> Option<usize>
    where
        F: FnMut(&T) -> bool,
    {
        let last_index = self.value.len();
        let mut bad_index: isize = -1;
        let mut good_index = last_index as isize;
        let mut mid_index = (bad_index + good_index) / 2;

        while bad_index + 1 < good_index {
            if predicate(&self.value[mid_index as usize]) {
                good_index = mid_index;
            } else {
                bad_index = mid_index;
            }
            mid_index = (bad_index + good_index) / 2;
        }

        (good_index != last_index as isize).then_some(good_index as usize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelativePathError {
    UnknownParentDirectory,
    UnmatchedAbsolutePath,
}

pub fn relative_path(
    path: impl AsRef<Path>,
    base: impl AsRef<Path>,
) -> Result<PathBuf, RelativePathError> {
    let path = path.as_ref();
    let base = base.as_ref();
    if path.is_absolute() != base.is_absolute() {
        return Err(RelativePathError::UnmatchedAbsolutePath);
    }

    let path_components = simplify_components(path);
    let base_components = simplify_components(base);
    let mut memo = Vec::new();
    path_components_from(&path_components, &base_components, &mut memo)?;
    Ok(if memo.is_empty() {
        PathBuf::from(".")
    } else {
        memo.iter().collect()
    })
}

fn path_components_from(
    path: &[String],
    base: &[String],
    memo: &mut Vec<String>,
) -> Result<(), RelativePathError> {
    match (base.first(), path.first()) {
        (None, None) => Ok(()),
        (None, Some(rhs)) => {
            if rhs != "." {
                memo.push(rhs.clone());
            }
            path_components_from(&path[1..], base, memo)
        }
        (Some(lhs), Some(rhs)) if memo.is_empty() && lhs == rhs => {
            path_components_from(&path[1..], &base[1..], memo)
        }
        (Some(lhs), _) => {
            if lhs == ".." {
                return Err(RelativePathError::UnknownParentDirectory);
            }
            if lhs != "." {
                memo.push("..".to_owned());
            }
            path_components_from(path, &base[1..], memo)
        }
    }
}

fn simplify_components(path: &Path) -> Vec<String> {
    let mut components = Vec::<String>::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => {}
            Component::CurDir => {
                if components.is_empty() {
                    components.push(".".to_owned());
                }
            }
            Component::ParentDir => {
                if let Some(last) = components.last() {
                    if last != ".." && last != "." {
                        components.pop();
                    } else {
                        components.push("..".to_owned());
                    }
                } else if !path.is_absolute() {
                    components.push("..".to_owned());
                }
            }
            Component::Normal(value) => components.push(value.to_string_lossy().into_owned()),
        }
    }
    if components.is_empty() && !path.is_absolute() {
        components.push(".".to_owned());
    }
    components
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobBehavior {
    BashV3,
    BashV4,
    Gradle,
}

pub fn glob_paths(
    pattern: &str,
    behavior: GlobBehavior,
    blacklisted_directories: &[&str],
) -> Vec<String> {
    let mut results = Vec::new();
    for pattern in expand_braces(pattern) {
        let root = glob_root(&pattern);
        let mut candidates = Vec::new();
        collect_candidates(&root, &mut candidates, blacklisted_directories);
        if root.exists() && matches!(behavior, GlobBehavior::BashV4) {
            candidates.push((root.clone(), true));
        }
        for (candidate, is_dir) in candidates {
            let mut candidate_string = candidate.to_string_lossy().into_owned();
            if is_dir {
                candidate_string.push('/');
            }
            if matches_glob(&pattern, &candidate_string, behavior, is_dir) {
                results.push(candidate_string);
            }
        }
    }
    results.sort();
    results.dedup();
    results
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(start) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let Some(end_offset) = pattern[start..].find('}') else {
        return vec![pattern.to_owned()];
    };
    let end = start + end_offset;
    let prefix = &pattern[..start];
    let suffix = &pattern[end + 1..];
    pattern[start + 1..end]
        .split(',')
        .map(|item| format!("{prefix}{item}{suffix}"))
        .collect()
}

fn glob_root(pattern: &str) -> PathBuf {
    let wildcard = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
    let prefix = &pattern[..wildcard];
    let root = prefix
        .rsplit_once('/')
        .map(|(root, _)| root)
        .unwrap_or(prefix);
    if root.is_empty() {
        PathBuf::from("/")
    } else {
        PathBuf::from(root)
    }
}

fn collect_candidates(
    root: &Path,
    candidates: &mut Vec<(PathBuf, bool)>,
    blacklisted_directories: &[&str],
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = path.is_dir();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if is_dir
            && blacklisted_directories
                .iter()
                .any(|blacklisted| *blacklisted == name)
        {
            continue;
        }
        candidates.push((path.clone(), is_dir));
        if is_dir {
            collect_candidates(&path, candidates, blacklisted_directories);
        }
    }
}

fn matches_glob(pattern: &str, candidate: &str, behavior: GlobBehavior, is_dir: bool) -> bool {
    match behavior {
        GlobBehavior::Gradle => {
            !is_dir
                && glob_match(
                    &pattern.replace("**/", "**").replace("/**", "/**"),
                    candidate,
                    true,
                )
        }
        GlobBehavior::BashV4 => glob_match(pattern, candidate, true),
        GlobBehavior::BashV3 => {
            let candidate = if pattern.ends_with('/') {
                candidate
            } else {
                candidate.trim_end_matches('/')
            };
            glob_match(&pattern.replace("**", "*"), candidate, false)
        }
    }
}

fn glob_match(pattern: &str, candidate: &str, recursive_globstar: bool) -> bool {
    let p = pattern.as_bytes();
    let c = candidate.as_bytes();
    glob_match_bytes(p, c, recursive_globstar)
}

fn glob_match_bytes(pattern: &[u8], candidate: &[u8], recursive_globstar: bool) -> bool {
    if pattern.is_empty() {
        return candidate.is_empty();
    }

    if recursive_globstar && pattern.starts_with(b"**") {
        let rest = &pattern[2..];
        if glob_match_bytes(rest, candidate, true) {
            return true;
        }
        for index in 0..candidate.len() {
            if glob_match_bytes(rest, &candidate[index + 1..], true) {
                return true;
            }
        }
        return false;
    }

    match pattern[0] {
        b'*' => {
            if glob_match_bytes(&pattern[1..], candidate, recursive_globstar) {
                return true;
            }
            for index in 0..candidate.len() {
                if candidate[index] == b'/' {
                    break;
                }
                if glob_match_bytes(&pattern[1..], &candidate[index + 1..], recursive_globstar) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            !candidate.is_empty()
                && candidate[0] != b'/'
                && glob_match_bytes(&pattern[1..], &candidate[1..], recursive_globstar)
        }
        byte => {
            !candidate.is_empty()
                && candidate[0] == byte
                && glob_match_bytes(&pattern[1..], &candidate[1..], recursive_globstar)
        }
    }
}

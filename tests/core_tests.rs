use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::thread;
use tempfile::TempDir;
use xcodegenrust::core::{
    glob_paths, relative_path, Atomic, GlobBehavior, RelativePathError, SortedArray,
};

#[test]
fn sorted_array_matches_xcodegen_tests() {
    let array = SortedArray::new([1, 2, 3, 4, 5]);
    assert_eq!(array.first_index_where(|value| *value > 2), Some(2));
    assert_eq!(array.first_index_where(|value| *value > 10), None);
    assert_eq!(
        SortedArray::<i32>::new([]).first_index_where(|value| *value > 0),
        None
    );
    let array = SortedArray::new([1, 2, 3, 3, 3]);
    assert_eq!(array.first_index_where(|value| *value == 3), Some(2));
    assert_eq!(SortedArray::new([1, 5, 4, 2]).value, vec![1, 2, 4, 5]);
    assert_eq!(SortedArray::<i32>::new([]).value, Vec::<i32>::new());
}

#[test]
fn relative_path_matches_xcodegen_tests() {
    fn rel(path: &str, base: &str) -> Result<String, RelativePathError> {
        relative_path(path, base).map(|path| path.to_string_lossy().into_owned())
    }

    assert_eq!(rel("a", "b").unwrap(), "../a");
    assert_eq!(rel("a", "b/").unwrap(), "../a");
    assert_eq!(rel("a/", "b").unwrap(), "../a");
    assert_eq!(rel("a/", "b/").unwrap(), "../a");
    assert_eq!(rel("/a", "/b").unwrap(), "../a");
    assert_eq!(rel("/a", "/b/").unwrap(), "../a");
    assert_eq!(rel("/a/", "/b").unwrap(), "../a");
    assert_eq!(rel("/a/", "/b/").unwrap(), "../a");
    assert_eq!(rel("a/b", "a/c").unwrap(), "../b");
    assert_eq!(rel("../a", "../b").unwrap(), "../a");
    assert_eq!(rel("a", ".").unwrap(), "a");
    assert_eq!(rel(".", "a").unwrap(), "..");
    assert_eq!(rel(".", ".").unwrap(), ".");
    assert_eq!(rel("..", "..").unwrap(), ".");
    assert_eq!(rel("..", ".").unwrap(), "..");
    assert_eq!(rel("/a/b/c/d", "/a/b").unwrap(), "c/d");
    assert_eq!(rel("/a/b", "/a/b/c/d").unwrap(), "../..");
    assert_eq!(rel("/e", "/a/b/c/d").unwrap(), "../../../../e");
    assert_eq!(rel("a/b/c", "a/d").unwrap(), "../b/c");
    assert_eq!(rel("/../a", "/b").unwrap(), "../a");
    assert_eq!(rel("../a", "b").unwrap(), "../../a");
    assert_eq!(rel("/a/../../b", "/b").unwrap(), ".");
    assert_eq!(rel("a/..", "a").unwrap(), "..");
    assert_eq!(rel("a/../b", "b").unwrap(), ".");
    assert_eq!(rel("/a/c", "/a/b/c").unwrap(), "../../c");
    assert_eq!(rel("a", "b/..").unwrap(), "a");
    assert_eq!(rel("b/c", "b/..").unwrap(), "b/c");
    assert!(rel("/", ".").is_err());
    assert!(rel(".", "/").is_err());
    assert!(rel("a", "..").is_err());
    assert!(rel(".", "..").is_err());
    assert!(rel("a", "b/../..").is_err());
}

#[test]
fn glob_matches_xcodegen_core_tests() {
    let fixture = GlobFixture::new();
    let root = fixture.root();

    assert_eq!(
        glob_paths(&format!("{root}/ba{{r,y,z}}"), GlobBehavior::BashV4, &[]),
        vec![format!("{root}/bar"), format!("{root}/baz")]
    );
    assert_eq!(
        glob_paths(&format!("{root}/nothing"), GlobBehavior::BashV4, &[]),
        Vec::<String>::new()
    );

    assert_eq!(
        glob_paths(&format!("{root}/**"), GlobBehavior::BashV3, &[]),
        vec![
            format!("{root}/bar"),
            format!("{root}/baz"),
            format!("{root}/dir1/"),
            format!("{root}/foo"),
        ]
    );
    assert_eq!(
        glob_paths(&format!("{root}/**/"), GlobBehavior::BashV3, &[]),
        vec![format!("{root}/dir1/")]
    );
    assert_eq!(
        glob_paths(&format!("{root}/**/*"), GlobBehavior::BashV3, &[]),
        vec![
            format!("{root}/dir1/dir2/"),
            format!("{root}/dir1/file1.ext"),
            format!("{root}/dir1/file1.extfoo"),
        ]
    );
    assert_eq!(
        glob_paths(&format!("{root}/**/dir2/**/*"), GlobBehavior::BashV3, &[]),
        vec![format!("{root}/dir1/dir2/dir3/file2.ext")]
    );

    assert_eq!(
        glob_paths(&format!("{root}/**/*.ext"), GlobBehavior::BashV4, &[]),
        vec![
            format!("{root}/dir1/dir2/dir3/file2.ext"),
            format!("{root}/dir1/file1.ext"),
        ]
    );
    assert_eq!(
        glob_paths(&format!("{root}/**/dir2/**/*"), GlobBehavior::BashV4, &[]),
        vec![
            format!("{root}/dir1/dir2/dir3/"),
            format!("{root}/dir1/dir2/dir3/file2.ext"),
        ]
    );

    let gradle_files = vec![
        format!("{root}/bar"),
        format!("{root}/baz"),
        format!("{root}/dir1/dir2/dir3/file2.ext"),
        format!("{root}/dir1/file1.ext"),
        format!("{root}/dir1/file1.extfoo"),
        format!("{root}/foo"),
    ];
    assert_eq!(
        glob_paths(&format!("{root}/**"), GlobBehavior::Gradle, &[]),
        gradle_files
    );
    assert_eq!(
        glob_paths(&format!("{root}/**/*"), GlobBehavior::Gradle, &["dir1"]),
        vec![
            format!("{root}/bar"),
            format!("{root}/baz"),
            format!("{root}/foo")
        ]
    );
}

#[test]
fn atomic_matches_xcodegen_simultaneous_write_test() {
    let atomic = Atomic::new(BTreeMap::<String, usize>::new());
    let mut handles = Vec::new();
    for index in 0..100 {
        let atomic = atomic.clone();
        handles.push(thread::spawn(move || {
            atomic.with(|dictionary| {
                dictionary.insert(index.to_string(), index);
            });
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let expected = (0..100)
        .map(|index| (index.to_string(), index))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(atomic.get(), expected);
}

struct GlobFixture {
    dir: TempDir,
}

impl GlobFixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("dir1/dir2/dir3")).unwrap();
        for file in [
            "foo",
            "bar",
            "baz",
            "dir1/file1.ext",
            "dir1/dir2/dir3/file2.ext",
            "dir1/file1.extfoo",
        ] {
            fs::write(dir.path().join(file), "").unwrap();
        }
        Self { dir }
    }

    fn root(&self) -> String {
        normalize(self.dir.path())
    }
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

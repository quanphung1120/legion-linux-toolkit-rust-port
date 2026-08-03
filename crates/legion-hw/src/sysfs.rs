use std::fs;
use std::path::{Path, PathBuf};

use crate::{HwError, Result};

/// The root every sysfs path in this crate is resolved against.
///
/// Production code uses [`SysRoot::system`] (`/`); tests use
/// [`SysRoot::at`] with a `tempfile::TempDir` holding a fake device tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRoot(PathBuf);

impl Default for SysRoot {
    fn default() -> Self {
        Self::system()
    }
}

impl SysRoot {
    /// The real machine.
    pub fn system() -> Self {
        Self(PathBuf::from("/"))
    }

    /// An alternate root — used by tests and by the daemon's `--sysfs-root`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self(root.into())
    }

    /// Resolve a *relative* sysfs path (e.g. `sys/class/...`) against the root.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    /// The root itself.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Read a sysfs file, trimming the trailing newline.
    pub fn read(&self, rel: &str) -> Result<String> {
        read_trimmed(&self.path(rel))
    }

    /// Read a sysfs file, mapping "absent" to `None` instead of an error.
    pub fn read_opt(&self, rel: &str) -> Option<String> {
        self.read(rel).ok()
    }

    /// Read and parse a sysfs file into any [`std::str::FromStr`] type.
    pub fn read_parsed<T>(&self, rel: &str) -> Result<T>
    where
        T: std::str::FromStr,
    {
        let raw = self.read(rel)?;
        raw.parse::<T>()
            .map_err(|_| HwError::Parse(format!("{rel}: {raw:?}")))
    }

    /// Parse if present and well-formed, otherwise `None`.
    pub fn read_parsed_opt<T>(&self, rel: &str) -> Option<T>
    where
        T: std::str::FromStr,
    {
        self.read_parsed(rel).ok()
    }

    /// Write a value to a sysfs file. Errors are classified by
    /// [`HwError::from_io`], so `EBUSY` becomes [`HwError::Busy`].
    pub fn write(&self, rel: &str, value: &str) -> Result<()> {
        let path = self.path(rel);
        if !path.exists() {
            return Err(HwError::NotSupported);
        }
        fs::write(&path, value).map_err(HwError::from_io)
    }

    /// Does this relative path exist?
    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Sorted names of the entries in a directory; empty when it is absent.
    pub fn list_dir(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = match fs::read_dir(self.path(rel)) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }
}

/// Read a file and trim surrounding whitespace (sysfs values end in `\n`).
pub(crate) fn read_trimmed(path: &Path) -> Result<String> {
    let raw = fs::read_to_string(path).map_err(HwError::from_io)?;
    Ok(raw.trim().to_string())
}

/// Find the hwmon directory whose `name` file equals `wanted`.
///
/// Returns a path relative to the [`SysRoot`], e.g.
/// `sys/class/hwmon/hwmon3`.
pub fn find_hwmon(root: &SysRoot, wanted: &str) -> Option<String> {
    for entry in root.list_dir("sys/class/hwmon") {
        let rel = format!("sys/class/hwmon/{entry}");
        if let Some(name) = root.read_opt(&format!("{rel}/name"))
            && name == wanted
        {
            return Some(rel);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn joins_relative_paths_under_root() {
        let root = SysRoot::at("/fake");
        assert_eq!(root.path("sys/class/x"), PathBuf::from("/fake/sys/class/x"));
    }

    #[test]
    fn read_trims_trailing_newline() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("v"), "balanced\n").unwrap();
        let root = SysRoot::at(tmp.path());
        assert_eq!(root.read("v").unwrap(), "balanced");
    }

    #[test]
    fn missing_file_is_not_supported() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(matches!(root.read("nope"), Err(HwError::NotSupported)));
        assert_eq!(root.read_opt("nope"), None);
    }

    #[test]
    fn write_to_missing_file_is_not_supported() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(matches!(
            root.write("nope", "1"),
            Err(HwError::NotSupported)
        ));
    }

    #[test]
    fn bad_number_is_a_parse_error() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("v"), "notanumber\n").unwrap();
        let root = SysRoot::at(tmp.path());
        assert!(matches!(
            root.read_parsed::<u32>("v"),
            Err(HwError::Parse(_))
        ));
    }

    #[test]
    fn find_hwmon_filters_by_name() {
        let tmp = TempDir::new().unwrap();
        let root = SysRoot::at(tmp.path());
        for (dir, name) in [("hwmon0", "nvme"), ("hwmon1", "k10temp")] {
            let p = tmp.path().join("sys/class/hwmon").join(dir);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("name"), format!("{name}\n")).unwrap();
        }
        assert_eq!(
            find_hwmon(&root, "k10temp"),
            Some("sys/class/hwmon/hwmon1".to_string())
        );
        assert_eq!(find_hwmon(&root, "coretemp"), None);
    }
}

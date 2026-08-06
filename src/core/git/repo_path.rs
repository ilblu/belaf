//! Byte-oriented repository path types.
//!
//! Git paths are byte arrays, not necessarily UTF-8, always `/`-separated.
//! [`RepoPath`] is the borrowed form and [`RepoPathBuf`] the owned one, plus
//! the path-safety validation and the escaping helper used to render
//! possibly-invalid-UTF-8 paths in diagnostics.

#[cfg(windows)]
use anyhow::anyhow;
use anyhow::bail;
use ref_cast::RefCast;
use std::path::Path;

use crate::core::errors::Result;

// Below we have helpers for trying to deal with git's paths properly, on the
// off-chance that they contain invalid UTF-8 and the like.

/// A borrowed reference to a pathname as understood by the backing repository.
///
/// In git, such a path is a byte array. The directory separator is always "/".
/// The bytes are often convertible to UTF-8, but not always. (These are the
/// same semantics as Unix paths.)
#[derive(Debug, Eq, Hash, PartialEq, RefCast)]
#[repr(transparent)]
pub struct RepoPath(pub(super) [u8]);

impl std::convert::AsRef<RepoPath> for [u8] {
    fn as_ref(&self) -> &RepoPath {
        RepoPath::ref_cast(self)
    }
}

impl std::convert::AsRef<[u8]> for RepoPath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl RepoPath {
    pub(super) fn new(p: &[u8]) -> &Self {
        p.as_ref()
    }

    /// Split a path into a directory name and a file basename.
    ///
    /// Returns `(dirname, basename)`. The dirname will be empty if the path
    /// contains no separator. Otherwise, it will end with the path separator.
    /// It is always true that `self = concat(dirname, basename)`.
    pub fn split_basename(&self) -> (&RepoPath, &RepoPath) {
        let basename = self
            .0
            .rsplit(|c| *c == b'/')
            .next()
            .expect("BUG: rsplit always returns at least one element");
        let ndir = self.0.len() - basename.len();
        (self.0[..ndir].as_ref(), basename.as_ref())
    }

    /// Return this path with a trailing directory separator removed, if one is
    /// present.
    pub fn pop_sep(&self) -> &RepoPath {
        let n = self.0.len();

        if n == 0 || self.0[n - 1] != b'/' {
            self
        } else {
            self.0[..n - 1].as_ref()
        }
    }

    /// Get the length of the path, in bytes
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the path is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert the repository path into an OS path.
    pub fn as_path(&self) -> &Path {
        bytes2path(&self.0)
    }

    /// Convert this borrowed reference into an owned copy.
    pub fn to_owned(&self) -> RepoPathBuf {
        RepoPathBuf::new(&self.0[..])
    }

    /// Compute a user-displayable escaped version of this path.
    pub fn escaped(&self) -> String {
        escape_pathlike(&self.0)
    }

    /// Return true if this path starts with the argument.
    pub fn starts_with<P: AsRef<[u8]>>(&self, other: P) -> bool {
        let other = other.as_ref();
        let sn = self.len();
        let on = other.len();

        if sn < on {
            false
        } else {
            &self.0[..on] == other
        }
    }

    /// Return true if this path ends with the argument.
    pub fn ends_with<P: AsRef<[u8]>>(&self, other: P) -> bool {
        let other = other.as_ref();
        let sn = self.len();
        let on = other.len();

        if sn < on {
            false
        } else {
            &self.0[(sn - on)..] == other
        }
    }
}

impl git2::IntoCString for &RepoPath {
    fn into_c_string(self) -> std::result::Result<std::ffi::CString, git2::Error> {
        self.0.into_c_string()
    }
}

// Copied from git2-rs src/util.rs
#[cfg(unix)]
fn bytes2path(b: &[u8]) -> &Path {
    use std::{ffi::OsStr, os::unix::prelude::*};
    Path::new(OsStr::from_bytes(b))
}
#[cfg(windows)]
fn bytes2path(b: &[u8]) -> &Path {
    use std::str;
    Path::new(str::from_utf8(b).expect("BUG: git paths should be valid UTF-8 on Windows"))
}

/// An owned reference to a pathname as understood by the backing repository.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct RepoPathBuf(Vec<u8>);

impl std::convert::AsRef<RepoPath> for RepoPathBuf {
    fn as_ref(&self) -> &RepoPath {
        RepoPath::new(&self.0[..])
    }
}

impl std::convert::AsRef<[u8]> for RepoPathBuf {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

fn validate_safe_repo_path(path: &Path) -> Result<()> {
    use std::path::Component;

    if path.as_os_str().is_empty() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        if bytes.contains(&0) {
            bail!("path contains null byte: `{}`", path.display());
        }

        if (bytes.windows(2).any(|w| w == b"/.") || bytes.starts_with(b"."))
            && bytes
                .split(|&b| b == b'/')
                .any(|seg| seg == b"." || seg == b"..")
        {
            bail!(
                "path contains current or parent directory reference (. or ..): `{}`",
                path.display()
            );
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        let wide_chars: Vec<u16> = path.as_os_str().encode_wide().collect();

        if wide_chars.contains(&0) {
            bail!("path contains null byte: `{}`", path.display());
        }

        let has_dot_segment = wide_chars
            .split(|&c| c == b'/' as u16 || c == b'\\' as u16)
            .any(|seg| seg == [b'.' as u16] || seg == [b'.' as u16, b'.' as u16]);

        if has_dot_segment {
            bail!(
                "path contains current or parent directory reference (. or ..): `{}`",
                path.display()
            );
        }

        let reserved_names = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];

        for component in path.components() {
            if let Component::Normal(comp) = component {
                if let Some(comp_str) = comp.to_str() {
                    let base = comp_str.split('.').next().unwrap_or("");
                    if reserved_names.iter().any(|&r| base.eq_ignore_ascii_case(r)) {
                        bail!(
                            "path contains reserved Windows filename: `{}`",
                            path.display()
                        );
                    }
                }
            }
        }
    }

    for component in path.components() {
        match component {
            Component::ParentDir => {
                bail!(
                    "path contains parent directory reference (..): `{}`",
                    path.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("path must be relative: `{}`", path.display());
            }
            Component::CurDir => {
                bail!(
                    "path contains current directory reference (.): `{}`",
                    path.display()
                );
            }
            Component::Normal(_) => {}
        }
    }

    Ok(())
}

impl RepoPathBuf {
    pub fn new(b: &[u8]) -> Self {
        RepoPathBuf(b.to_vec())
    }

    /// Create a RepoPathBuf from a Path-like. It is assumed that the path is
    /// relative to the repository working directory root and doesn't have any
    /// funny business like ".." in it.
    #[cfg(unix)]
    pub(super) fn from_path<P: AsRef<Path>>(p: P) -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        let path = p.as_ref();

        validate_safe_repo_path(path)?;

        Ok(Self::new(path.as_os_str().as_bytes()))
    }

    /// Create a RepoPathBuf from a Path-like. It is assumed that the path is
    /// relative to the repository working directory root and doesn't have any
    /// funny business like ".." in it.
    #[cfg(windows)]
    pub(super) fn from_path<P: AsRef<Path>>(p: P) -> Result<Self> {
        let path = p.as_ref();

        validate_safe_repo_path(path)?;

        let mut first = true;
        let mut b = Vec::new();

        for cmpt in path.components() {
            if first {
                first = false;
            } else {
                b.push(b'/');
            }

            if let std::path::Component::Normal(c) = cmpt {
                let s = c
                    .to_str()
                    .ok_or_else(|| anyhow!("path component `{:?}` is not valid UTF-8", c))?;
                b.extend(s.as_bytes());
            } else {
                bail!("path with unexpected components: `{}`", path.display());
            }
        }

        Ok(RepoPathBuf(b))
    }

    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    pub fn push<C: AsRef<[u8]>>(&mut self, component: C) {
        let n = self.0.len();

        if n > 0 && self.0[n - 1] != b'/' {
            self.0.push(b'/');
        }

        self.0.extend(component.as_ref());
    }
}

impl std::ops::Deref for RepoPathBuf {
    type Target = RepoPath;

    fn deref(&self) -> &RepoPath {
        RepoPath::new(&self.0[..])
    }
}

/// Convert an arbitrary byte slice to something printable.
///
/// If the bytes can be interpreted as UTF-8, their Unicode stringification will
/// be returned. Otherwise, bytes that aren't printable ASCII will be
/// backslash-escaped, and the whole string will be wrapped in double quotes.
///
/// Special handling for security-relevant characters (null bytes, control chars).
pub fn escape_pathlike(b: &[u8]) -> String {
    if b.contains(&0) {
        let mut buf = String::from("\"<path-with-null-byte:");
        for (i, &byte) in b.iter().enumerate() {
            if byte == 0 {
                buf.push_str(&format!("\\0@{}", i));
            }
        }
        buf.push_str(">\"");
        return buf;
    }

    if let Ok(s) = std::str::from_utf8(b) {
        if s.chars().all(|c| {
            (c.is_ascii_graphic() && c != '"' && c != '\\')
                || c == '/'
                || c == '-'
                || c == '_'
                || c == '.'
        }) {
            return s.to_owned();
        }

        let mut buf = String::from("\"");
        for ch in s.chars() {
            match ch {
                '"' => buf.push_str("\\\""),
                '\\' => buf.push_str("\\\\"),
                '\n' => buf.push_str("\\n"),
                '\r' => buf.push_str("\\r"),
                '\t' => buf.push_str("\\t"),
                c if c.is_control() => buf.push_str(&format!("\\u{{{:04x}}}", c as u32)),
                c => buf.push(c),
            }
        }
        buf.push('"');
        buf
    } else {
        let mut buf = vec![b'\"'];
        buf.extend(b.iter().flat_map(|c| std::ascii::escape_default(*c)));
        buf.push(b'\"');
        String::from_utf8(buf).expect("BUG: ASCII escape sequences should always be valid UTF-8")
    }
}

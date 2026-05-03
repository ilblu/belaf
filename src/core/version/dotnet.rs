//! .NET System.Version versions

use anyhow::bail;
use std::fmt::{Display, Formatter};

use crate::core::errors::{Error, Result};

/// A version compatible with .NET's System.Version
///
/// These versions are simple: they have the form
/// `{major}.{minor}.{build}.{revision}`. Each term must be between 0 and
/// 65534.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DotNetVersion {
    pub major: i32,
    pub minor: i32,
    pub build: i32,
    pub revision: i32,
}

impl Display for DotNetVersion {
    fn fmt(&self, f: &mut Formatter) -> std::result::Result<(), std::fmt::Error> {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.build, self.revision
        )
    }
}

impl std::str::FromStr for DotNetVersion {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let pieces: std::result::Result<Vec<_>, _> = s.split('.').map(|s| s.parse()).collect();

        match pieces.as_ref().map(|v| v.len()) {
            Ok(4) => {}
            _ => bail!("failed to parse `{}` as a .NET version", s),
        }

        let pieces = pieces.expect("BUG: pieces should be Ok after match validation");

        Ok(DotNetVersion {
            major: pieces[0],
            minor: pieces[1],
            build: pieces[2],
            revision: pieces[3],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greater_less() {
        const CASES: &[(&str, &str)] = &[
            ("0.0.0.9999", "0.0.1.0"),
            ("0.0.0.9999", "0.1.0.0"),
            ("0.0.0.9999", "1.0.0.0"),
            ("1.0.0.0", "1.0.0.1"),
        ];

        for (l_text, g_text) in CASES {
            let lesser = l_text
                .parse::<DotNetVersion>()
                .expect("BUG: test case should parse");
            let greater = g_text
                .parse::<DotNetVersion>()
                .expect("BUG: test case should parse");
            assert!(lesser < greater);
            assert!(greater > lesser);
        }
    }
}

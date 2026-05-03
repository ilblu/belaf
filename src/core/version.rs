// Copyright 2020 Peter Williams <peter@newton.cx> and collaborators
// Licensed under the MIT License.

//! Version numbers.

use anyhow::bail;
use std::fmt::{Display, Formatter};
use thiserror::Error as ThisError;
use time::OffsetDateTime;

use crate::core::errors::Result;

const SECONDS_PER_DAY: i64 = 86400;
const PEP440_YEAR_MULTIPLIER: usize = 10000;
const PEP440_MONTH_MULTIPLIER: usize = 100;

/// A version number associated with a project.
///
/// This is an enumeration because different kinds of projects may subscribe to
/// different kinds of versioning schemes.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum Version {
    /// A version compatible with the semantic versioning specification.
    Semver(semver::Version),

    // A version compatible with the Python PEP-440 specification.
    Pep440(pep440::Pep440Version),

    // A version compatible with the .NET System.Version type.
    DotNet(dotnet::DotNetVersion),
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Version::Semver(ref v) => write!(f, "{v}"),
            Version::Pep440(ref v) => write!(f, "{v}"),
            Version::DotNet(ref v) => write!(f, "{v}"),
        }
    }
}

impl Version {
    /// Given a template version, parse another version
    pub fn parse_like<T: AsRef<str>>(&self, text: T) -> Result<Version> {
        Ok(match self {
            Version::Semver(_) => Version::Semver(semver::Version::parse(text.as_ref())?),
            Version::Pep440(_) => Version::Pep440(text.as_ref().parse()?),
            Version::DotNet(_) => Version::DotNet(text.as_ref().parse()?),
        })
    }

    /// Given a template version, compute its "zero"
    pub fn zero_like(&self) -> Version {
        match self {
            Version::Semver(_) => Version::Semver(semver::Version::new(0, 0, 0)),
            Version::Pep440(_) => Version::Pep440(pep440::Pep440Version::default()),
            Version::DotNet(_) => Version::DotNet(dotnet::DotNetVersion::default()),
        }
    }

    /// Mutate this version to be Belaf's default "development mode" value.
    pub fn set_to_dev_value(&mut self) {
        match self {
            Version::Semver(v) => {
                v.major = 0;
                v.minor = 0;
                v.patch = 0;
                v.pre = semver::Prerelease::new("dev.0")
                    .expect("BUG: 'dev.0' is a valid semver prerelease");
                v.build = semver::BuildMetadata::EMPTY;
            }

            Version::Pep440(v) => {
                v.epoch = 0;
                v.segments.clear();
                v.segments.push(0);
                v.pre_release = None;
                v.post_release = None;
                v.dev_release = Some(0);
                v.local_identifier = None;
            }

            Version::DotNet(v) => {
                // Quasi-hack for WWT
                v.minor = 99;
                v.build = 0;
                v.revision = 0;
            }
        }
    }

    /// Given a template version, parse a "bump scheme" from a textual
    /// description.
    ///
    /// Not all bump schemes are compatible with all versioning styles, which is
    /// why this operation depends on the version template and is fallible.
    #[expect(clippy::result_large_err)]
    pub fn parse_bump_scheme(
        &self,
        text: &str,
    ) -> std::result::Result<VersionBumpScheme, UnsupportedBumpSchemeError> {
        if let Some(force_text) = text.strip_prefix("force ") {
            return Ok(VersionBumpScheme::Force(force_text.to_owned()));
        }

        match text {
            "patch" => Ok(VersionBumpScheme::MicroBump),
            "minor" => Ok(VersionBumpScheme::MinorBump),
            "major" => Ok(VersionBumpScheme::MajorBump),
            _ => Err(UnsupportedBumpSchemeError(text.to_owned(), self.clone())),
        }
    }

    pub fn as_pep440_tuple_literal(&self) -> Result<String> {
        if let Version::Pep440(v) = self {
            v.as_tuple_literal()
        } else {
            bail!("version {} cannot be rendered as a PEP440 literal since it is not a PEP440 version", self)
        }
    }
}

/// An error returned when a "version bump scheme" cannot be parsed, or if it is
/// not allowed for the version template. The first inner value is the bump
/// scheme text, and the second inner value is the template version.
#[derive(Debug, ThisError)]
#[error("illegal version-bump scheme \"{0}\" for version template {1:?}")]
pub struct UnsupportedBumpSchemeError(pub String, pub Version);

/// A scheme for assigning a new version number to a project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionBumpScheme {
    /// Assigns a development-mode version (likely 0.0.0) with a YYYYMMDD date code included.
    DevDatecode,

    /// Increment the third-most-significant version number, resetting any
    /// less-significant entries.
    MicroBump,

    /// Increment the second-most-significant version number, resetting any
    /// less-significant entries.
    MinorBump,

    /// Increment the most-significant version number, resetting any
    /// less-significant entries.
    MajorBump,

    /// Force the version to the specified value.
    Force(String),
}

impl VersionBumpScheme {
    /// Apply this bump to a version.
    pub fn apply(&self, version: &mut Version) -> Result<()> {
        // This function inherently has to matrix over versioning schemes and
        // versioning systems, so it gets a little hairy.
        return match self {
            VersionBumpScheme::DevDatecode => apply_dev_datecode(version),
            VersionBumpScheme::MicroBump => apply_micro_bump(version),
            VersionBumpScheme::MinorBump => apply_minor_bump(version),
            VersionBumpScheme::MajorBump => apply_major_bump(version),
            VersionBumpScheme::Force(ref t) => apply_force(version, t),
        };

        #[expect(clippy::unnecessary_wraps)]
        fn apply_dev_datecode(version: &mut Version) -> Result<()> {
            let now = OffsetDateTime::now_utc();

            match version {
                Version::Semver(v) => {
                    let code = format!("{:04}{:02}{:02}", now.year(), now.month() as u8, now.day());
                    v.build = semver::BuildMetadata::new(&code).expect(
                        "BUG: YYYYMMDD date format should always be valid semver build metadata",
                    );
                }

                Version::Pep440(v) => {
                    // Here we use a `dev` series number rather than the `local_identifier` so
                    // that it can be expressed as a version_info tuple if needed.
                    let num = PEP440_YEAR_MULTIPLIER * (now.year() as usize)
                        + PEP440_MONTH_MULTIPLIER * (now.month() as usize)
                        + (now.day() as usize);
                    v.dev_release = Some(num);
                }

                Version::DotNet(v) => {
                    // We can't use a human-readable date-code because version
                    // terms have a maximum value of 65534, so we use a number
                    // that's about the number of days since 1970. That should
                    // take us to about the year 2149.
                    v.revision = (now.unix_timestamp() / SECONDS_PER_DAY) as i32;
                }
            }

            Ok(())
        }

        #[expect(clippy::unnecessary_wraps)]
        fn apply_micro_bump(version: &mut Version) -> Result<()> {
            match version {
                Version::Semver(v) => {
                    v.pre = semver::Prerelease::EMPTY;
                    v.build = semver::BuildMetadata::EMPTY;
                    v.patch += 1;
                }

                Version::Pep440(v) => {
                    while v.segments.len() < 3 {
                        v.segments.push(0);
                    }

                    v.pre_release = None;
                    v.post_release = None;
                    v.dev_release = None;
                    v.local_identifier = None;

                    v.segments[2] += 1;
                    v.segments.truncate(3);
                }

                Version::DotNet(v) => {
                    v.revision = 0;
                    v.build += 1;
                }
            }

            Ok(())
        }

        #[expect(clippy::unnecessary_wraps)]
        fn apply_minor_bump(version: &mut Version) -> Result<()> {
            match version {
                Version::Semver(v) => {
                    v.pre = semver::Prerelease::EMPTY;
                    v.build = semver::BuildMetadata::EMPTY;
                    v.patch = 0;
                    v.minor += 1;
                }

                Version::Pep440(v) => {
                    while v.segments.len() < 3 {
                        v.segments.push(0);
                    }

                    v.pre_release = None;
                    v.post_release = None;
                    v.dev_release = None;
                    v.local_identifier = None;

                    v.segments[1] += 1;
                    v.segments[2] = 0;
                    v.segments.truncate(3);
                }

                Version::DotNet(v) => {
                    v.revision = 0;
                    v.build = 0;
                    v.minor += 1;
                }
            }

            Ok(())
        }

        #[expect(clippy::unnecessary_wraps)]
        fn apply_major_bump(version: &mut Version) -> Result<()> {
            match version {
                Version::Semver(v) => {
                    v.pre = semver::Prerelease::EMPTY;
                    v.build = semver::BuildMetadata::EMPTY;
                    v.patch = 0;
                    v.minor = 0;
                    v.major += 1;
                }

                Version::Pep440(v) => {
                    while v.segments.len() < 3 {
                        v.segments.push(0);
                    }

                    v.pre_release = None;
                    v.post_release = None;
                    v.dev_release = None;
                    v.local_identifier = None;

                    v.segments[0] += 1;
                    v.segments[1] = 0;
                    v.segments[2] = 0;
                    v.segments.truncate(3);
                }

                Version::DotNet(v) => {
                    v.revision = 0;
                    v.build = 0;
                    v.minor = 0;
                    v.major += 1;
                }
            }

            Ok(())
        }

        fn apply_force(version: &mut Version, text: &str) -> Result<()> {
            *version = version.parse_like(text)?;
            Ok(())
        }
    }
}

pub mod dotnet;
pub mod pep440;

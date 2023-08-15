use std::{
    fmt::{format, Display},
    str::FromStr,
};

use docker_api::opts::{ImageBuildOpts, ImageFilter};
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct DockerImage {
    raw_name: String,
    registry: Option<String>,
    repository: String,
    version: Version,
    build_instructions: Option<BuildImageInstructions>,
}

impl DockerImage {
    fn new<S: Into<String>>(
        raw_name: S,
        registry: Option<S>,
        repository: S,
        version: Version,
    ) -> Self {
        DockerImage {
            raw_name: raw_name.into(),
            registry: registry.map(|r| r.into()),
            repository: repository.into(),
            version,
            build_instructions: None,
        }
    }

    pub fn get_full_name(&self) -> String {
        self.raw_name.clone()
    }
}

impl Display for DockerImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw_name)
    }
}

impl FromStr for DockerImage {
    type Err = String;

    fn from_str(full_image_name: &str) -> Result<Self, Self::Err> {
        let (registry, repository_and_version) = match full_image_name.split_once("/") {
            Some((registry, repository_and_version)) => {
                if registry.contains(".")
                    || registry.contains(":")
                    || registry.contains("localhost")
                {
                    (Some(registry), repository_and_version)
                } else {
                    (None, full_image_name)
                }
            }
            None => (None, full_image_name),
        };

        let (repository, version) = match repository_and_version.split_once("@sha256:") {
            Some((repository, version)) => (repository, Version::from_sha256(version)?),
            None => match repository_and_version.split_once(":") {
                Some((repository, version)) => (repository, Version::from_tag(version)?),
                None => (repository_and_version, Version::Any),
            },
        };
        if repository.contains("@") || repository.contains(":") {
            Err(format!("invalid repository name: {repository}"))
        } else {
            Ok(DockerImage {
                raw_name: full_image_name.into(),
                registry: registry.map(|r| r.into()),
                repository: repository.into(),
                version,
                build_instructions: None,
            })
        }
    }
}

impl From<&str> for DockerImage {
    fn from(full_image_name: &str) -> Self {
        full_image_name
            .parse()
            .expect(format!("docker image name should be parseable: {full_image_name}").as_str())
    }
}

impl Into<ImageFilter> for DockerImage {
    fn into(self) -> ImageFilter {
        let image = self
            .registry
            .map(|registry| format!("{registry}/{}", self.repository))
            .unwrap_or(self.repository);
        let tag = match self.version {
            Version::Any => None,
            Version::Sha256(sha256) => Some(sha256),
            Version::Tag(tag) => Some(tag),
        };
        ImageFilter::Reference(image, tag)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Version {
    Any,
    Sha256(String),
    Tag(String),
}
impl Version {
    fn from_sha256(hash: &str) -> Result<Version, String> {
        let re_hash = Regex::new("^[0-9a-fA-F]{32,}$").unwrap();
        if re_hash.is_match(hash) {
            Ok(Version::Sha256(hash.into()))
        } else {
            Err(format!("invalid sha256 hash version: {hash}"))
        }
    }

    fn from_tag(tag: &str) -> Result<Version, String> {
        let re_tag = Regex::new(r"^[\w][\w.\-]{0,127}$").unwrap();
        if re_tag.is_match(tag) {
            Ok(Version::Tag(tag.into()))
        } else {
            Err(format!("invalid tag version: {tag}"))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BuildImageInstructions {
    path: String,
}

impl Into<Option<ImageBuildOpts>> for DockerImage {
    fn into(self) -> Option<ImageBuildOpts> {
        self.build_instructions.map(|i| {
            let opts = ImageBuildOpts::builder(i.path);
            opts.build()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_local_image_name() {
        assert_eq!(
            DockerImage::from_str("rust").unwrap(),
            DockerImage::new("rust", None, "rust", Version::Any)
        );
        assert_eq!(
            DockerImage::from_str("myname:latest").unwrap(),
            DockerImage::new(
                "myname:latest",
                None,
                "myname",
                Version::Tag("latest".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("repo/my-name:1.0").unwrap(),
            DockerImage::new(
                "repo/my-name:1.0",
                None,
                "repo/my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("registry.foo.com:1234/my-name:1.0").unwrap(),
            DockerImage::new(
                "registry.foo.com:1234/my-name:1.0",
                Some("registry.foo.com:1234"),
                "my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("registry.foo.com/my-name:1.0").unwrap(),
            DockerImage::new(
                "registry.foo.com/my-name:1.0",
                Some("registry.foo.com"),
                "my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("registry.foo.com:1234/repo_here/my-name:1.0").unwrap(),
            DockerImage::new(
                "registry.foo.com:1234/repo_here/my-name:1.0",
                Some("registry.foo.com:1234"),
                "repo_here/my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str(
                "registry.foo.com:1234/repo-here/my-name@sha256:1234abcd1234abcd1234abcd1234abcd"
            )
            .unwrap(),
            DockerImage::new(
                "registry.foo.com:1234/repo-here/my-name@sha256:1234abcd1234abcd1234abcd1234abcd",
                Some("registry.foo.com:1234"),
                "repo-here/my-name",
                Version::Sha256("1234abcd1234abcd1234abcd1234abcd".into())
            )
        );
        assert_eq!(
            DockerImage::from_str(
                "registry.foo.com:1234/my-name@sha256:1234abcd1234abcd1234abcd1234abcd"
            )
            .unwrap(),
            DockerImage::new(
                "registry.foo.com:1234/my-name@sha256:1234abcd1234abcd1234abcd1234abcd",
                Some("registry.foo.com:1234"),
                "my-name",
                Version::Sha256("1234abcd1234abcd1234abcd1234abcd".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("1.2.3.4/my-name:1.0").unwrap(),
            DockerImage::new(
                "1.2.3.4/my-name:1.0",
                Some("1.2.3.4"),
                "my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("1.2.3.4:1234/my-name:1.0").unwrap(),
            DockerImage::new(
                "1.2.3.4:1234/my-name:1.0",
                Some("1.2.3.4:1234"),
                "my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("1.2.3.4/repo-here/my-name:1.0").unwrap(),
            DockerImage::new(
                "1.2.3.4/repo-here/my-name:1.0",
                Some("1.2.3.4"),
                "repo-here/my-name",
                Version::Tag("1.0".into())
            )
        );
        assert_eq!(
            DockerImage::from_str("1.2.3.4:1234/repo-here/my-name:1.0").unwrap(),
            DockerImage::new(
                "1.2.3.4:1234/repo-here/my-name:1.0",
                Some("1.2.3.4:1234"),
                "repo-here/my-name",
                Version::Tag("1.0".into())
            )
        );
    }

    #[test]
    fn cant_parse_invalid_version() {
        assert_eq!(
            DockerImage::from_str("rust@invalid"),
            Err("invalid repository name: rust@invalid".into())
        );
        assert_eq!(
            DockerImage::from_str("repo:rust:invalid"),
            Err("invalid tag version: rust:invalid".into())
        );
    }
}

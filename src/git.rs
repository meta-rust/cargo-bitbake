/*
 * Copyright 2016-2017 Doug Goldstein <cardoe@cardoe.com>
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
 * option. This file may not be copied, modified, or distributed
 * except according to those terms.
 */

use cargo::Config;
use cargo::util::{CargoResult, CargoResultExt};
use git2::{self, Repository};
use regex::Regex;
use std::default::Default;
use std::fmt::{self, Display};

/// basic pattern to match ssh style remote URLs
/// so that they can be fixed up
/// git@github.com:cardoe/cargo-bitbake.git should match
const SSH_STYLE_REMOTE_STR: &'static str = r".*@.*:.*";

lazy_static! {
    static ref SSH_STYLE_REMOTE: Regex = Regex::new(SSH_STYLE_REMOTE_STR).unwrap();
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GitPrefix {
    Git,
    GitSubmodule,
}

impl Default for GitPrefix {
    fn default() -> GitPrefix {
        GitPrefix::Git
    }
}

impl Display for GitPrefix {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", match *self {
            GitPrefix::Git => "git",
            GitPrefix::GitSubmodule => "gitsm",
        })
    }
}

/// converts a GIT URL to a Yocto GIT URL
pub fn git_to_yocto_git_url(url: &str, name: Option<&str>, prefix: GitPrefix) -> String {
    // check if its a git@github.com:cardoe/cargo-bitbake.git style URL
    // and fix it up if it is
    let fixed_url = if SSH_STYLE_REMOTE.is_match(url) {
        format!("ssh://{}", url.replace(":", "/"))
    } else {
        url.to_string()
    };


    // convert the protocol to one that Yocto understands
    // https://... -> git://...;protocol=https
    // ssh://... -> git://...;protocol=ssh
    // and append metadata necessary for Yocto to generate
    // data for Cargo to understand
    let yocto_url = match fixed_url.split_at(fixed_url.find(':').unwrap()) {
        (proto @ "ssh", rest) |
        (proto @ "http", rest) |
        (proto @ "https", rest) => format!("{}{};protocol={}", prefix, rest, proto),
        (_, _) => fixed_url.to_owned(),
    };

    if let Some(name) = name {
        format!("{};name={};destsuffix={}", yocto_url, name, name)
    } else {
        yocto_url
    }
}

#[derive(Debug, Default)]
pub struct ProjectRepo {
    pub uri: String,
    pub branch: String,
    pub rev: String,
    pub tag: bool,
}

impl ProjectRepo {
    /// Attempts to guess at the upstream repo this project can be fetched from
    pub fn new(config: &Config) -> CargoResult<ProjectRepo> {
        let repo = Repository::discover(config.cwd())
            .chain_err(|| "Unable to determine git repo for this project")?;

        let remote = repo.find_remote("origin")
            .chain_err(|| "Unable to find remote 'origin' for this project")?;

        let submodules = repo.submodules().chain_err(|| "Unable to determine the submodules")?;
        let prefix = if submodules.is_empty() { GitPrefix::Git } else { GitPrefix::GitSubmodule };

        let uri = remote.url().ok_or("No URL for remote 'origin'")?;
        let uri = git_to_yocto_git_url(uri, None, prefix);

        let head = repo.head().chain_err(|| "Unable to find HEAD")?;
        let branch = head.shorthand().ok_or("Unable resolve HEAD to a branch")?;

        // if the branch is master or HEAD we don't want it
        let uri = if branch == "master" || branch == "HEAD" {
            uri
        } else {
            format!("{};branch={}", uri, branch)
        };

        let rev = head.target().ok_or("Unable to resolve HEAD to a commit")?;

        Ok(ProjectRepo {
               uri: uri,
               branch: branch.to_string(),
               rev: rev.to_string(),
               tag: Self::rev_is_tag(&repo, &rev),
           })
    }

    /// attempts to determine if the specific revision is a tag
    fn rev_is_tag(repo: &git2::Repository, rev: &git2::Oid) -> bool {
        // gather up all the tags, if there are none then its not a tag
        let tags = match repo.tag_names(None) {
            Ok(t) => t,
            Err(_) => return false,
        };

        // walk through all the tags and resolve them to their commitish
        // return true if we find a tag that matches our revision
        tags.iter()
            .filter_map(|tag| tag)
            .filter_map(|tag| repo.revparse_single(tag).ok())
            .filter_map(|tag| tag.peel(git2::ObjectType::Commit).ok())
            .any(|t| t.id() == *rev)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn remote_http() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://github.com/rust-lang/cargo.git;protocol=http;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_https() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://github.com/rust-lang/cargo.git;protocol=https;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_ssh() {
        let repo = "git@github.com:rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://git@github.com/rust-lang/cargo.git;protocol=ssh;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_http_nosuffix() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None, GitPrefix::Git);
        assert_eq!(url, "git://github.com/rust-lang/cargo.git;protocol=http");
    }

    #[test]
    fn remote_https_nosuffix() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None, GitPrefix::Git);
        assert_eq!(url, "git://github.com/rust-lang/cargo.git;protocol=https");
    }

    #[test]
    fn remote_ssh_nosuffix() {
        let repo = "git@github.com:rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None, GitPrefix::Git);
        assert_eq!(url, "git://git@github.com/rust-lang/cargo.git;protocol=ssh");
    }

    #[test]
    fn cargo_http() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://github.com/rust-lang/cargo.git;protocol=http;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn cargo_https() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://github.com/rust-lang/cargo.git;protocol=https;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn cargo_ssh() {
        let repo = "ssh://git@github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::Git);
        assert_eq!(url,
                "git://git@github.com/rust-lang/cargo.git;protocol=ssh;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_ssh_with_submodules() {
        let repo = "git@github.com:rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"), GitPrefix::GitSubmodule);
        assert_eq!(url,
                "gitsm://git@github.com/rust-lang/cargo.git;protocol=ssh;name=cargo;destsuffix=cargo");
    }
}

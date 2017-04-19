use cargo::Config;
use cargo::util::{human, CargoResult};
use git2::Repository;
use regex::Regex;

/// basic pattern to match ssh style remote URLs
/// so that they can be fixed up
/// git@github.com:cardoe/cargo-bitbake.git should match
const SSH_STYLE_REMOTE_STR: &'static str = r".*@.*:.*";

lazy_static! {
    static ref SSH_STYLE_REMOTE: Regex = Regex::new(SSH_STYLE_REMOTE_STR).unwrap();
}

/// converts a GIT URL to a Yocto GIT URL
pub fn git_to_yocto_git_url(url: &str, name: Option<&str>) -> String {
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
        (proto @ "https", rest) => format!("git{};protocol={}", rest, proto),
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
}

impl ProjectRepo {
    /// Attempts to guess at the upstream repo this project can be fetched from
    pub fn new(config: &Config) -> CargoResult<ProjectRepo> {
        let repo =
            Repository::discover(config.cwd()).map_err(|e| {
                             human(format!("Unable to determine git repo for this project: {}", e))
                         })?;

        let remote =
            repo.find_remote("origin")
                .map_err(|e| {
                             human(format!("Unable to find remote 'origin' for this project: {}",
                                           e))
                         })?;

        let uri = remote.url().ok_or(human("No URL for remote 'origin'"))?;
        let uri = git_to_yocto_git_url(uri, None);

        let head = repo.head().map_err(|e| human(format!("Unable to find HEAD: {}", e)))?;
        let branch = head.shorthand().ok_or(human("Unable resolve HEAD to a branch"))?;

        // if the branch isn't master we need to record
        let uri = if branch != "master" {
            format!("{};branch={}", uri, branch)
        } else {
            uri
        };

        let rev = head.target().ok_or(human("Unable to resolve HEAD to a commit"))?;

        Ok(ProjectRepo {
               uri: uri,
               branch: branch.to_string(),
               rev: rev.to_string(),
           })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn remote_http() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://github.com/rust-lang/cargo.git;protocol=http;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_https() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://github.com/rust-lang/cargo.git;protocol=https;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_ssh() {
        let repo = "git@github.com:rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://git@github.com/rust-lang/cargo.git;protocol=ssh;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn remote_http_nosuffix() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None);
        assert!(url == "git://github.com/rust-lang/cargo.git;protocol=http");
    }

    #[test]
    fn remote_https_nosuffix() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None);
        assert!(url == "git://github.com/rust-lang/cargo.git;protocol=https");
    }

    #[test]
    fn remote_ssh_nosuffix() {
        let repo = "git@github.com:rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, None);
        assert!(url == "git://git@github.com/rust-lang/cargo.git;protocol=ssh");
    }

    #[test]
    fn cargo_http() {
        let repo = "http://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://github.com/rust-lang/cargo.git;protocol=http;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn cargo_https() {
        let repo = "https://github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://github.com/rust-lang/cargo.git;protocol=https;name=cargo;destsuffix=cargo");
    }

    #[test]
    fn cargo_ssh() {
        let repo = "ssh://git@github.com/rust-lang/cargo.git";
        let url = git_to_yocto_git_url(repo, Some("cargo"));
        assert!(url ==
                "git://git@github.com/rust-lang/cargo.git;protocol=ssh;name=cargo;destsuffix=cargo");
    }
}

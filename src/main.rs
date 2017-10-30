extern crate cargo;
extern crate itertools;
#[macro_use]
extern crate lazy_static;
extern crate git2;
extern crate md5;
extern crate regex;
extern crate rustc_serialize;

use cargo::{Config, CliResult};
use cargo::core::{Package, PackageSet, Resolve, Workspace};
use cargo::core::registry::PackageRegistry;
use cargo::core::source::GitReference;
use cargo::core::resolver::Method;
use cargo::ops;
use cargo::util::{human, important_paths, CargoResult};
use itertools::Itertools;
use std::default::Default;
use std::env;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

mod git;
mod license;

const CRATES_IO_URL: &'static str = "crates.io";

/// Represents the package we are trying to generate a recipe for
struct PackageInfo<'cfg> {
    cfg: &'cfg Config,
    current_manifest: PathBuf,
    ws: Workspace<'cfg>,
}

impl<'cfg> PackageInfo<'cfg> {
    /// creates our package info from the config and the manifest_path,
    /// which may not be provided
    fn new(config: &Config, manifest_path: Option<String>) -> CargoResult<PackageInfo> {
        let root = important_paths::find_root_manifest_for_wd(manifest_path, config.cwd())?;
        let ws = Workspace::new(&root, config)?;
        Ok(PackageInfo {
               cfg: config,
               current_manifest: root,
               ws: ws,
           })
    }

    /// provides the current package we are working with
    fn package(&self) -> CargoResult<&Package> {
        self.ws.current()
    }

    /// Generates a package registry by using the Cargo.lock or
    /// creating one as necessary
    fn registry(&self) -> CargoResult<PackageRegistry<'cfg>> {
        let mut registry = PackageRegistry::new(self.cfg)?;
        let package = self.package()?;
        registry
            .add_sources(&[package.package_id().source_id().clone()])?;
        Ok(registry)
    }

    /// Resolve the packages necessary for the workspace
    fn resolve(&self) -> CargoResult<(PackageSet<'cfg>, Resolve)> {
        // build up our registry
        let mut registry = self.registry()?;

        // resolve our dependencies
        let (packages, resolve) = ops::resolve_ws(&self.ws)?;

        // resolve with all features set so we ensure we get all of the depends downloaded
        let resolve = ops::resolve_with_previous(&mut registry,
                                                 &self.ws,
                                                 /* resolve it all */
                                                 Method::Everything,
                                                 /* previous */
                                                 Some(&resolve),
                                                 /* don't avoid any */
                                                 None,
                                                 /* specs */
                                                 &[])?;

        Ok((packages, resolve))
    }

    /// packages that are part of a workspace are a sub directory from the
    /// top level which we need to record, this provides us with that
    /// relative directory
    fn rel_dir(&self) -> CargoResult<PathBuf> {
        // this is the top level of the workspace
        let root = self.ws.root().to_path_buf();
        // path where our current package's Cargo.toml lives
        let cwd = self.current_manifest.parent().unwrap();

        cwd.strip_prefix(&root)
            .map(|p| p.to_path_buf())
            .map_err(|e| human(format!("Unable to if Cargo.toml is in a sub directory: {}", e)))
    }
}

/// command line options for this command
#[derive(RustcDecodable)]
struct Options {
    flag_verbose: u32,
    flag_quiet: Option<bool>,
}

const USAGE: &'static str = r#"
Create BitBake recipe for a project

Usage:
    cargo bitbake [options]

Options:
    -h, --help          Print this message
    -v, --verbose       Use verbose output
    -q, --quiet         No output printed to stdout
"#;

fn main() {
    let config = Config::default().unwrap();
    let args = env::args().collect::<Vec<_>>();
    let result = cargo::call_main_without_stdin(real_main, &config, USAGE, &args, false);
    if let Err(e) = result {
        cargo::exit_with_error(e, &mut *config.shell());
    }
}

fn real_main(options: Options, config: &Config) -> CliResult {
    config
        .configure(options.flag_verbose,
                   options.flag_quiet,
                   /* color */
                   &None,
                   /* frozen */
                   false,
                   /* locked */
                   false)?;

    // Build up data about the package we are attempting to generate a recipe for
    let md = PackageInfo::new(config, None)?;

    // Our current package
    let package = md.package()?;
    let crate_root = package
        .manifest_path()
        .parent()
        .expect("Cargo.toml must have a parent");

    // Resolve all dependencies (generate or use Cargo.lock as necessary)
    let resolve = md.resolve()?;

    // build the crate URIs
    let mut src_uri_extras = vec![];
    let mut src_uris = resolve
        .1
        .iter()
        .filter_map(|pkg| {
            // get the source info for this package
            let src_id = pkg.source_id();
            if pkg.name() == package.name() {
                None
            } else if src_id.is_registry() {
                // this package appears in a crate registry
                Some(format!("crate://{}/{}/{} \\\n",
                             CRATES_IO_URL,
                             pkg.name(),
                             pkg.version()))
            } else if src_id.is_path() {
                // we don't want to spit out path based
                // entries since they're within the crate
                // we are packaging
                None
            } else if src_id.is_git() {
                let url = git::git_to_yocto_git_url(src_id.url().as_str(), Some(pkg.name()));

                // save revision
                src_uri_extras.push(format!("SRCREV_FORMAT .= \"_{}\"", pkg.name()));
                let rev = match *src_id.git_reference().unwrap() {
                    GitReference::Tag(ref s) |
                    GitReference::Rev(ref s) => s.to_owned(),
                    GitReference::Branch(ref s) => {
                        if s == "master" {
                            String::from("${AUTOREV}")
                        } else {
                            s.to_owned()
                        }
                    }
                };

                src_uri_extras.push(format!("SRCREV_{} = \"{}\"", pkg.name(), rev));
                // instruct Cargo where to find this
                src_uri_extras
                    .push(format!("EXTRA_OECARGO_PATHS += \"${{WORKDIR}}/{}\"", pkg.name()));

                Some(url)
            } else {
                Some(format!("{} \\\n", src_id.url().to_string()))
            }
        })
        .collect::<Vec<String>>();

    // sort the crate list
    src_uris.sort();

    // root package metadata
    let metadata = package.manifest().metadata();

    // package description is used as BitBake summary
    let summary = metadata.description.as_ref().map_or_else(
        || {
            println!("No package.description set in your Cargo.toml, using package.name");
            package.name()
        },
        |s| s.trim(),
    );

    // package homepage (or source code location)
    let homepage = metadata
        .homepage
        .as_ref()
        .map_or_else(
            || {
                println!("No package.homepage set in your Cargo.toml, trying package.repository");
                metadata
                    .repository
                    .as_ref()
                    .ok_or_else(|| human("No package.repository set in your Cargo.toml"))
            },
            |s| Ok(s),
        )?
        .trim();

    // package license
    let license = metadata.license.as_ref().map_or_else(
        || {
            println!("No package.license set in your Cargo.toml, trying package.license_file");
            metadata.license_file.as_ref().map_or_else(
                || {
                    println!("No package.license_file set in your Cargo.toml");
                    println!("Assuming {} license", license::CLOSED_LICENSE);
                    license::CLOSED_LICENSE
                },
                |s| s.as_str(),
            )
        },
        |s| s.as_str(),
    );

    // compute the relative directory into the repo our Cargo.toml is at
    let rel_dir = md.rel_dir().unwrap();

    // license files for the package
    let mut lic_files = vec![];
    for lic in license.split('/') {
        lic_files.push(license::file(&crate_root, &rel_dir, lic));
    }

    // license data in Yocto fmt
    let license = license.split('/').map(|f| f.trim()).join(" | ");

    // attempt to figure out the git repo for this project
    let project_repo = git::ProjectRepo::new(config).unwrap_or_else(|e| {
                                                                        println!("{}", e);
                                                                        Default::default()
                                                                    });

    // if this is not a tag we need to include some data about the version in PV so that
    // the sstate cache remains valid
    let git_srcpv = if project_repo.tag && project_repo.rev.len() > 10 {
        // its a tag so nothing needed
        "".into()
    } else {
        // we should be using ${SRCPV} here but due to a bitbake bug we cannot. see:
        // https://github.com/meta-rust/meta-rust/issues/136
        format!("PV_append = \".AUTOINC+{}\"",
                project_repo.rev.split_at(10).0)
    };

    // build up the path
    let recipe_path = PathBuf::from(format!("{}_{}.bb", package.name(), package.version()));

    // Open the file where we'll write the BitBake recipe
    let mut file = try!(OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(true)
                            .open(&recipe_path)
                            .map_err(|err| {
                                         human(format!("failed to create BitBake recipe: {}",
                                                       err.description()))
                                     }));

    // write the contents out
    try!(write!(file,
                include_str!("bitbake.template"),
                name = package.name(),
                version = package.version(),
                summary = summary,
                homepage = homepage,
                license = license,
                lic_files = lic_files.join(""),
                src_uri = src_uris.join(""),
                src_uri_extras = src_uri_extras.join("\n"),
                project_rel_dir = rel_dir.display(),
                project_src_uri = project_repo.uri,
                project_src_rev = project_repo.rev,
                git_srcpv = git_srcpv,
                cargo_bitbake_ver = env!("CARGO_PKG_VERSION"),
                ).map_err(|err| {
                          human(format!("unable to write BitBake recipe to disk: {}",
                                        err.description()))
                      }));

    println!("Wrote: {}", recipe_path.display());

    Ok(())
}

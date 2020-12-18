/*
 * Copyright 2016-2017 Doug Goldstein <cardoe@cardoe.com>
 *
 * Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
 * http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
 * <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
 * option. This file may not be copied, modified, or distributed
 * except according to those terms.
 */

extern crate anyhow;
extern crate cargo;
extern crate git2;
extern crate itertools;
extern crate lazy_static;
extern crate md5;
extern crate regex;
extern crate structopt;

use anyhow::{anyhow, Context as _};
use cargo::core::registry::PackageRegistry;
use cargo::core::resolver::features::HasDevUnits;
use cargo::core::resolver::CliFeatures;
use cargo::core::source::GitReference;
use cargo::core::{Package, PackageSet, Resolve, Workspace};
use cargo::ops;
use cargo::util::{important_paths, CargoResult};
use cargo::{CliResult, Config};
use itertools::Itertools;
use std::default::Default;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use structopt::clap::AppSettings;
use structopt::StructOpt;

mod git;
mod license;

const CRATES_IO_URL: &str = "crates.io";

/// Represents the package we are trying to generate a recipe for
struct PackageInfo<'cfg> {
    cfg: &'cfg Config,
    current_manifest: PathBuf,
    ws: Workspace<'cfg>,
}

impl<'cfg> PackageInfo<'cfg> {
    /// creates our package info from the config and the `manifest_path`,
    /// which may not be provided
    fn new(config: &Config, manifest_path: Option<String>) -> CargoResult<PackageInfo> {
        let manifest_path = manifest_path.map_or_else(|| config.cwd().to_path_buf(), PathBuf::from);
        let root = important_paths::find_root_manifest_for_wd(&manifest_path)?;
        let ws = Workspace::new(&root, config)?;
        Ok(PackageInfo {
            cfg: config,
            current_manifest: root,
            ws,
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
        registry.add_sources(vec![package.package_id().source_id()])?;
        Ok(registry)
    }

    /// Resolve the packages necessary for the workspace
    fn resolve(&self) -> CargoResult<(PackageSet<'cfg>, Resolve)> {
        // build up our registry
        let mut registry = self.registry()?;

        // resolve our dependencies
        let (packages, resolve) = ops::resolve_ws(&self.ws)?;

        // resolve with all features set so we ensure we get all of the depends downloaded
        let resolve = ops::resolve_with_previous(
            &mut registry,
            &self.ws,
            /* resolve it all */
            &CliFeatures::new_all(true),
            HasDevUnits::No,
            /* previous */
            Some(&resolve),
            /* don't avoid any */
            None,
            /* specs */
            &[],
            /* warn? */
            true,
        )?;

        Ok((packages, resolve))
    }

    /// packages that are part of a workspace are a sub directory from the
    /// top level which we need to record, this provides us with that
    /// relative directory
    fn rel_dir(&self) -> CargoResult<PathBuf> {
        // this is the top level of the workspace
        let root = self.ws.root().to_path_buf();
        // path where our current package's Cargo.toml lives
        let cwd = self.current_manifest.parent().ok_or_else(|| {
            anyhow!(
                "Could not get parent of directory '{}'",
                self.current_manifest.display()
            )
        })?;

        cwd.strip_prefix(&root)
            .map(Path::to_path_buf)
            .context("Unable to if Cargo.toml is in a sub directory")
    }
}

#[derive(StructOpt, Debug)]
struct Args {
    /// Silence all output
    #[structopt(short = "q")]
    quiet: bool,

    /// Verbose mode (-v, -vv, -vvv, etc.)
    #[structopt(short = "v", parse(from_occurrences))]
    verbose: usize,

    /// Reproducible mode: Output exact git references for git projects
    #[structopt(short = "R")]
    reproducible: bool,

    /// Legacy Overrides: Use legacy override syntax
    #[structopt(short = "l", long = "--legacy-overrides")]
    legacy_overrides: bool,
}

#[derive(StructOpt, Debug)]
#[structopt(
    name = "cargo-bitbake",
    bin_name = "cargo",
    author,
    about = "Generates a BitBake recipe for a given Cargo project",
    global_settings(&[AppSettings::ColoredHelp])
)]
enum Opt {
    /// Generates a BitBake recipe for a given Cargo project
    #[structopt(name = "bitbake")]
    Bitbake(Args),
}

fn main() {
    let mut config = Config::default().unwrap();
    let Opt::Bitbake(opt) = Opt::from_args();
    let result = real_main(opt, &mut config);
    if let Err(e) = result {
        cargo::exit_with_error(e, &mut *config.shell());
    }
}

fn real_main(options: Args, config: &mut Config) -> CliResult {
    config.configure(
        options.verbose as u32,
        options.quiet,
        /* color */
        None,
        /* frozen */
        false,
        /* locked */
        false,
        /* offline */
        false,
        /* target dir */
        &None,
        /* unstable flags */
        &[],
        /* CLI config */
        &[],
    )?;

    // Build up data about the package we are attempting to generate a recipe for
    let md = PackageInfo::new(config, None)?;

    // Our current package
    let package = md.package()?;
    let crate_root = package
        .manifest_path()
        .parent()
        .expect("Cargo.toml must have a parent");

    if package.name().contains('_') {
        println!("Package name contains an underscore");
    }

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
                Some(format!(
                    "    crate://{}/{}/{} \\\n",
                    CRATES_IO_URL,
                    pkg.name(),
                    pkg.version()
                ))
            } else if src_id.is_path() {
                // we don't want to spit out path based
                // entries since they're within the crate
                // we are packaging
                None
            } else if src_id.is_git() {
                // Just use the default download method for git repositories
                // found in the source URIs, since cargo currently cannot
                // initialize submodules for git dependencies anyway.
                let url = git::git_to_yocto_git_url(
                    src_id.url().as_str(),
                    Some(pkg.name().as_str()),
                    git::GitPrefix::default(),
                );

                // save revision
                src_uri_extras.push(format!("SRCREV_FORMAT .= \"_{}\"", pkg.name()));

                let precise = if options.reproducible {
                    src_id.precise()
                } else {
                    None
                };

                let rev = if let Some(precise) = precise {
                    precise
                } else {
                    match *src_id.git_reference()? {
                        GitReference::Tag(ref s) => s,
                        GitReference::Rev(ref s) => {
                            if s.len() == 40 {
                                // avoid reduced hashes
                                s
                            } else {
                                let precise = src_id.precise();
                                if let Some(p) = precise {
                                    p
                                } else {
                                    panic!("cannot find rev in correct format!");
                                }
                            }
                        }
                        GitReference::Branch(ref s) => {
                            if s == "master" {
                                "${AUTOREV}"
                            } else {
                                s
                            }
                        }
                        GitReference::DefaultBranch => "${AUTOREV}",
                    }
                };

                src_uri_extras.push(format!("SRCREV_{} = \"{}\"", pkg.name(), rev));
                // instruct Cargo where to find this
                src_uri_extras.push(format!(
                    "EXTRA_OECARGO_PATHS += \"${{WORKDIR}}/{}\"",
                    pkg.name()
                ));

                Some(format!("    {} \\\n", url))
            } else {
                Some(format!("    {} \\\n", src_id.url()))
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
        |s| cargo::util::interning::InternedString::new(&s.trim().replace("\n", " \\\n")),
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
                    .ok_or_else(|| anyhow!("No package.repository set in your Cargo.toml"))
            },
            Ok,
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
                String::as_str,
            )
        },
        String::as_str,
    );

    // compute the relative directory into the repo our Cargo.toml is at
    let rel_dir = md.rel_dir()?;

    // license files for the package
    let mut lic_files = vec![];
    let licenses: Vec<&str> = license.split('/').collect();
    let single_license = licenses.len() == 1;
    for lic in licenses {
        lic_files.push(format!(
            "    {}",
            license::file(crate_root, &rel_dir, lic, single_license)
        ));
    }

    // license data in Yocto fmt
    let license = license.split('/').map(str::trim).join(" | ");

    // attempt to figure out the git repo for this project
    let project_repo = git::ProjectRepo::new(config).unwrap_or_else(|e| {
        println!("{}", e);
        Default::default()
    });

    // if this is not a tag we need to include some data about the version in PV so that
    // the sstate cache remains valid
    let git_srcpv = if !project_repo.tag && project_repo.rev.len() > 10 {
        let mut pv_append_key = "PV:append";
        // Override PV override with legacy syntax if flagged
        if options.legacy_overrides {
            pv_append_key = "PV_append";
        }
        // we should be using ${SRCPV} here but due to a bitbake bug we cannot. see:
        // https://github.com/meta-rust/meta-rust/issues/136
        format!("{} = \".AUTOINC+{}\"", pv_append_key, &project_repo.rev[..10])
    } else {
        // its a tag so nothing needed
        "".into()
    };

    // Build up the recipe inc path
    let recipe_inc_path = PathBuf::from(format!("{}_{}.inc", package.name(), package.version()));

    // Open the file where we'll write the BitBake include recipe
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&recipe_inc_path)
        // CliResult accepts only failure::Error, not failure::Context
        .map_err(|e| anyhow::format_err!("Unable to open bitbake recipe inc file with: {}", e))?;

    // Write the contents out
    write!(
        file,
        include_str!("bitbake_inc.template"),
        src_uri = src_uris.join(""),
    )
    .map_err(|e| anyhow::format_err!("Unable to write to bitbake recipe inc file with: {}", e))?;

    println!("Wrote: {}", recipe_inc_path.display());

    // build up the path
    let recipe_path = PathBuf::from(format!("{}_{}.bb", package.name(), package.version()));

    // Open the file where we'll write the BitBake recipe
    file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&recipe_path)
        // CliResult accepts only failure::Error, not failure::Context
        .map_err(|e| anyhow!("Unable to open bitbake recipe file with: {}", e))?;

    // write the contents out
    write!(
        file,
        include_str!("bitbake.template"),
        name = package.name(),
        version = package.version(),
        summary = summary,
        homepage = homepage,
        license = license,
        lic_files = lic_files.join(""),
        src_uri_inc_path = recipe_inc_path.display(),
        src_uri_extras = src_uri_extras.join("\n"),
        project_rel_dir = rel_dir.display(),
        project_src_uri = project_repo.uri,
        project_src_rev = project_repo.rev,
        git_srcpv = git_srcpv,
        cargo_bitbake_ver = env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| anyhow!("Unable to write to bitbake recipe file with: {}", e))?;

    println!("Wrote: {}", recipe_path.display());

    Ok(())
}

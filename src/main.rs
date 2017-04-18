extern crate cargo;
extern crate itertools;
extern crate md5;
extern crate rustc_serialize;

use cargo::{Config, CliResult};
use cargo::core::{Package, PackageSet, Resolve, Workspace};
use cargo::core::registry::PackageRegistry;
use cargo::core::source::GitReference;
use cargo::core::resolver::Method;
use cargo::ops;
use cargo::util::{human, important_paths, CargoResult};
use itertools::Itertools;
use md5::Context;
use std::env;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const CRATES_IO_URL: &'static str = "crates.io";

fn file_md5<P: AsRef<Path>>(license_file: P) -> Result<String, io::Error> {
    let mut file = try!(File::open(license_file));
    let mut context = Context::new();

    try!(io::copy(&mut file, &mut context));
    Ok(format!("{:x}", context.compute()))
}

fn license_file(license_name: &str) -> String {
    // if the license exists at the top level then
    // return the right URL to it. try to handle the special
    // case license path we support as well
    let special_name = format!("LICENSE-{}", license_name);
    let lic_path = Path::new(license_name);
    let spec_path = Path::new(&special_name);

    if lic_path.exists() {
        let md5sum = file_md5(&license_name).unwrap_or_else(|_| String::from("generateme"));
        format!("file://{};md5={} \\\n", license_name, md5sum)
    } else if spec_path.exists() {
        // the special case
        let md5sum = file_md5(&special_name).unwrap_or_else(|_| String::from("generateme"));
        format!("file://{};md5={} \\\n", special_name, md5sum)
    } else {
        // fall through
        format!("file://{};md5=generateme \\\n", license_name)
    }
}

/// Finds the root Cargo.toml of the workspace
fn workspace(config: &Config, manifest_path: Option<String>) -> CargoResult<Workspace> {
    let root = important_paths::find_root_manifest_for_wd(manifest_path, config.cwd())?;
    Workspace::new(&root, config)
}

/// Generates a package registry by using the Cargo.lock or creating one as necessary
fn registry<'a>(config: &'a Config, package: &Package) -> CargoResult<PackageRegistry<'a>> {
    let mut registry = PackageRegistry::new(config)?;
    registry.add_sources(&[package.package_id().source_id().clone()])?;
    Ok(registry)
}

/// Resolve the packages necessary for the workspace
fn resolve<'a>(registry: &mut PackageRegistry,
               workspace: &'a Workspace)
               -> CargoResult<(PackageSet<'a>, Resolve)> {
    // resolve our dependencies
    let (packages, resolve) = ops::resolve_ws(workspace)?;

    // resolve with all features set so we ensure we get all of the depends downloaded
    let resolve = ops::resolve_with_previous(registry,
                                             workspace,
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
        cargo::handle_cli_error(e, &mut *config.shell());
    }
}

fn real_main(options: Options, config: &Config) -> CliResult {
    config.configure(options.flag_verbose,
                     options.flag_quiet,
                     /* color */
                     &None,
                     /* frozen */
                     false,
                     /* locked */
                     false)?;

    // Load the workspace and current package
    let workspace = workspace(config, None)?;
    let package = workspace.current()?;

    // Resolve all dependencies (generate or use Cargo.lock as necessary)
    let mut registry = registry(config, package)?;
    let resolve = resolve(&mut registry, &workspace)?;

    // build the crate URIs
    let mut src_uri_extras = vec![];
    let mut src_uris = resolve.1
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
                let url = src_id.url().to_string();

                // covert the protocol to one that Yocto understands
                // https://... -> git://...;protocol=https
                // ssh://... -> git://...;protocol=ssh
                // and append metadata necessary for Yocto to generate
                // data for Cargo to understand
                let url = match url.split_at(url.find(':').unwrap()) {
                    (proto @ "ssh", rest) |
                    (proto @ "https", rest) => {
                        format!("git{};protocol={};name={};destsuffix={}",
                                rest,
                                proto,
                                pkg.name(),
                                pkg.name())
                    }
                    (_, _) => format!("{};name={};destsuffix={}", url, pkg.name(), pkg.name()),
                };

                // save revision
                src_uri_extras.push(format!("SRCREV_FORMAT .= \"_{}\"", pkg.name()));
                let rev = match *src_id.git_reference().unwrap() {
                    GitReference::Tag(ref s) |
                    GitReference::Rev(ref s) => s.to_owned(),
                    GitReference::Branch(ref s) => {
                        if s == "master" {
                            String::from("${{AUTOREV}}")
                        } else {
                            s.to_owned()
                        }
                    }
                };

                src_uri_extras.push(format!("SRCREV_{} = \"{}\"", pkg.name(), rev));
                // instruct Cargo where to find this
                src_uri_extras.push(format!("EXTRA_OECARGO_PATHS += \"${{WORKDIR}}/{}\"",
                                            pkg.name()));

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
    let summary = metadata.description.as_ref().map_or_else(|| {
            println!("No package.description set in your Cargo.toml, using package.name");
            package.name()
    }, |s| s.trim());

    // package homepage (or source code location)
    let homepage = metadata.homepage.as_ref().map_or_else(|| {
        println!("No package.homepage set in your Cargo.toml, trying package.repository");
        metadata.repository.as_ref().ok_or_else(|| {
            human("No package.repository set in your Cargo.toml")
        })
    }, |s| Ok(s))?.trim();

    // package license
    let license = metadata.license.as_ref().map_or_else(|| {
        println!("No package.license set in your Cargo.toml, trying package.license_file");
        metadata.license_file.as_ref().ok_or_else(|| {
            human("No package.license_file set in your Cargo.toml")
        })
    }, |s| Ok(s))?;

    // license files for the package
    let lic_files = license.clone()
        .split('/')
        .map(license_file)
        .join("");

    // license data in Yocto fmt
    let license = license.split('/').map(|f| f.trim()).join(" | ");

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
                lic_files = lic_files,
                src_uri = src_uris.join(""),
                src_uri_extras = src_uri_extras.join("\n"),
                cargo_bitbake_ver = env!("CARGO_PKG_VERSION"),
                )
                 .map_err(|err| {
                              human(format!("unable to write BitBake recipe to disk: {}",
                                            err.description()))
                          }));

    println!("Wrote: {}", recipe_path.display());

    Ok(())
}

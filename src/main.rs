extern crate cargo;
extern crate itertools;
extern crate rustc_serialize;

use cargo::{Config, CliResult, CliError};
use cargo::core::Package;
use cargo::core::registry::PackageRegistry;
use cargo::ops;
use cargo::util::important_paths;
use itertools::Itertools;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

const CRATES_IO_URL: &'static str = "crates.io";

fn license_file(license_name: &str) -> String {
    // if the license exists at the top level then
    // return the right URL to it. try to handle the special
    // case license path we support as well
    let special_name = format!("LICENSE-{}", license_name);
    let lic_path = Path::new(license_name);
    let spec_path = Path::new(&special_name);

    if lic_path.exists() {
        format!("file://{};md5=generateme \\\n", license_name)
    } else if spec_path.exists() {
        // the special case
        format!("file://{};md5=generateme \\\n", special_name)
    } else {
        // fall through
        format!("file://{};md5=generateme \\\n", license_name)
    }
}

#[derive(RustcDecodable)]
struct Options {
    flag_verbose: bool,
    flag_quiet: bool,
}

fn main() {
    cargo::execute_main_without_stdin(real_main,
                                      false,
                                      r#"
Create BitBake recipe for a project

Usage:
    cargo bitbake [options]

Options:
    -h, --help          Print this message
    -v, --verbose       Use verbose output
    -q, --quiet         No output printed to stdout
"#)
}

fn real_main(options: Options, config: &Config) -> CliResult<Option<()>> {
    try!(config.shell().set_verbosity(options.flag_verbose, options.flag_quiet));

    // Load the root package
    let root = try!(important_paths::find_root_manifest_for_wd(None, config.cwd()));
    let package = try!(Package::for_path(&root, config));

    // Resolve all dependencies (generate or use Cargo.lock as necessary)
    let mut registry = PackageRegistry::new(config);
    try!(registry.add_sources(&[package.package_id().source_id().clone()]));
    let resolve = try!(ops::resolve_pkg(&mut registry, &package, config));

    // build the crate URIs
    let mut src_uris = resolve.iter()
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
    let summary = metadata.description
        .as_ref()
        .cloned()
        .unwrap_or_else(|| String::from(package.name()));

    // package homepage (or source code location)
    let homepage = metadata.homepage
        .as_ref()
        .cloned()
        .unwrap_or(metadata.repository
            .as_ref()
            .cloned()
            .unwrap_or_else(|| String::from("")));

    // package license
    let license = metadata.license
        .as_ref()
        .cloned()
        .unwrap_or(metadata.license_file
            .as_ref()
            .cloned()
            .unwrap_or_else(|| String::from("unknown")))
        .split('/')
        .map(|s| s.trim())
        .join(" | ");

    // license files for the package
    let lic_files = metadata.license
        .as_ref()
        .cloned()
        .unwrap_or(metadata.license_file
            .as_ref()
            .cloned()
            .unwrap_or_else(|| String::from("unknown")))
        .split('/')
        .map(license_file)
        .join("");

    // build up the path
    let recipe_path = PathBuf::from(format!("{}_{}.bb", package.name(), package.version()));

    // Open the file where we'll write the BitBake recipe
    let mut file = try!(OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&recipe_path)
        .map_err(|err| {
            CliError::new(&format!("failed to create BitBake recipe: {}", err.description()),
                          1)
        }));

    // write the contents out
    try!(write!(file,
                include_str!("bitbake.template"),
                name = package.name(),
                version = package.version(),
                summary = summary.trim(),
                homepage = homepage.trim(),
                license = license.trim(),
                lic_files = lic_files.trim(),
                src_uri = src_uris.join(""),
                cargo_bitbake_ver = env!("CARGO_PKG_VERSION"),
                )
        .map_err(|err| {
            CliError::new(&format!("unable to write BitBake recipe to disk: {}",
                                   err.description()),
                          1)
        }));

    println!("Wrote: {}", recipe_path.display());

    Ok(None)
}

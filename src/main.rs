extern crate cargo;
extern crate rustache;
extern crate rustc_serialize;

use cargo::{Config, CliResult, CliError};
use cargo::core::Package;
use cargo::core::registry::PackageRegistry;
use cargo::ops;
use cargo::util::important_paths;
use rustache::HashBuilder;
use std::error::Error;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

const CRATES_IO_URL: &'static str = "crates.io";

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
    let src_uris = resolve.iter()
        .map(|pkg| {
            // get the source info for this package
            let src_id = pkg.source_id();
            if src_id.is_registry() {
                // this package appears in a crate registry
                format!("crate://{}/{}/{} \\\n",
                        CRATES_IO_URL,
                        pkg.name(),
                        pkg.version())
            } else {
                format!("{} \\\n", src_id.url().to_string())
            }
        })
        .collect::<Vec<String>>();

    let index_src_uri = String::from("crate-index://crates.io/CARGO_INDEX_COMMIT");

    // the bitbake recipe template
    let template = include_str!("bitbake.template");

    // root package metadata
    let metadata = package.manifest().metadata();

    // package description is used as BitBake summary
    let summary = metadata.description
        .as_ref()
        .map(|t| t.clone())
        .unwrap_or(String::from("unknown summary"));

    // package repository
    let repo = metadata.repository
        .as_ref()
        .map(|t| t.clone())
        .unwrap_or(String::from("unknown repo"));

    // build up the path
    let recipe_path = PathBuf::from(format!("{}_{}.bb", package.name(), package.version()));

    // build up the varibles for the template
    let data = HashBuilder::new()
        .insert_string("summary", summary.trim())
        .insert_string("repository", repo.trim())
        .insert_string("index_src_uri", index_src_uri.trim())
        .insert_string("src_uri", src_uris.join(""));

    // generate the BitBake recipe using Rustache to process the template
    let mut templ = try!(rustache::render_text(template, data)
        .map_err(|_| CliError::new("unable to generate BitBake recipe: {}", 1)));

    // Open the file where we'll write the BitBake recipe
    let mut file = try!(OpenOptions::new()
        .write(true)
        .create(true)
        .open(&recipe_path)
        .map_err(|err| {
            CliError::new(&format!("failed to create BitBake recipe: {}", err.description()),
                          1)
        }));

    // write the contents out
    try!(io::copy(&mut templ, &mut file).map_err(|err| {
        CliError::new(&format!("unable to write BitBake recipe to disk: {}",
                               err.description()),
                      1)
    }));

    println!("Wrote: {}", recipe_path.display());

    Ok(None)
}

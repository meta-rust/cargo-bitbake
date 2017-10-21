
use md5::Context;
use std::fs::File;
use std::io;
use std::path::Path;

/// For a given file at path `license_file`, generate the MD5 sum
fn file_md5<P: AsRef<Path>>(license_file: P) -> Result<String, io::Error> {
    let mut file = try!(File::open(license_file));
    let mut context = Context::new();

    try!(io::copy(&mut file, &mut context));
    Ok(format!("{:x}", context.compute()))
}

/// Given the top level of the crate at `crate_root`, attempt to find
/// the license file based on the name of the license in `license_name`.
pub fn file(crate_root: &Path, rel_dir: &Path, license_name: &str) -> String {
    // if the license exists at the top level then
    // return the right URL to it. try to handle the special
    // case license path we support as well
    let special_name = format!("LICENSE-{}", license_name);
    let lic_path = Path::new(license_name);
    let spec_path = Path::new(&special_name);

    let lic_abs_path = crate_root.join(lic_path);
    let spec_abs_path = crate_root.join(spec_path);

    if lic_abs_path.exists() {
        let md5sum = file_md5(lic_abs_path).unwrap_or_else(|_| String::from("generateme"));
        format!("file://{};md5={} \\\n",
                rel_dir.join(lic_path).display(),
                md5sum)
    } else if spec_abs_path.exists() {
        // the special case
        let md5sum = file_md5(spec_abs_path).unwrap_or_else(|_| String::from("generateme"));
        format!("file://{};md5={} \\\n",
                rel_dir.join(spec_path).display(),
                md5sum)
    } else {
        // fall through
        format!("file://{};md5=generateme \\\n", license_name)
    }
}

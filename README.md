# cargo-bitbake

[![Build Status](https://travis-ci.org/cardoe/cargo-bitbake.svg?branch=master)](https://travis-ci.org/cardoe/cargo-bitbake) [![Latest Version](https://img.shields.io/crates/v/cargo-bitbake.svg)](https://crates.io/crates/cargo-bitbake)

`cargo bitbake` is a Cargo subcommand that generates a
[BitBake](https://en.wikipedia.org/wiki/BitBake) recipe that uses
[meta-rust](https://github.com/meta-rust/meta-rust) to build a Cargo based
project for [Yocto](https://yoctoproject.org)

Install it with Cargo:

```
$ cargo install --locked cargo-bitbake
```

In its default mode, `cargo bitbake` will write the recipe for the
local crate:

```
$ cargo bitbake
Wrote: cargo-bitbake_0.1.0.bb
```

## Parameter Mapping
|  Yocto           |          Cargo              |
| ---------------- | --------------------------- |
| SRC_URI          | each line in `dependencies` |
| SUMMARY          | `package.description` |
| HOMEPAGE         | `package.homepage` or `package.repository` |
| LICENSE          | `package.license` or `package.license-file`
| LIC_FILES_CHKSUM | `package.license` or `package.license-file`. See below |

### LIC_FILES_CHKSUM

`LIC_FILES_CHKSUM` is treated a bit specially. If the user specifies `package.license-file` then the
filename is taken directly. If `package.license` is specified then it checks for the filename directly
and falls back to checking `LICENSE-{license}`. If nothing can be found then you are expected to generate
the md5sum yourself.

The license field supports any valid Cargo value and can be separated by `/` to specify multiple licenses.

## API

API documentation is available at [docs.rs](https://docs.rs/crate/cargo-bitbake/).

## Example output
```
$ cat cargo-bitbake_0.1.0.bb
inherit cargo_util

SRC_URI = " \
crate://crates.io/libssh2-sys/0.1.37 \
crate://crates.io/crates-io/0.2.0 \
crate://crates.io/openssl-sys/0.7.14 \
crate://crates.io/nom/1.2.3 \
crate://crates.io/rustache/0.0.3 \
crate://crates.io/url/1.1.1 \
crate://crates.io/unicode-bidi/0.2.3 \
crate://crates.io/num_cpus/0.2.13 \
crate://crates.io/libc/0.2.14 \
crate://crates.io/strsim/0.3.0 \
crate://crates.io/fs2/0.2.5 \
crate://crates.io/curl/0.2.19 \
crate://crates.io/pkg-config/0.3.8 \
crate://crates.io/filetime/0.1.10 \
crate://crates.io/flate2/0.2.14 \
crate://crates.io/matches/0.1.2 \
crate://crates.io/unicode-normalization/0.1.2 \
crate://crates.io/tar/0.4.6 \
crate://crates.io/memchr/0.1.11 \
crate://crates.io/git2/0.4.4 \
crate://crates.io/git2-curl/0.4.1 \
crate://crates.io/env_logger/0.3.4 \
crate://crates.io/winapi/0.2.8 \
crate://crates.io/miniz-sys/0.1.7 \
crate://crates.io/libgit2-sys/0.4.4 \
crate://crates.io/advapi32-sys/0.1.2 \
crate://crates.io/toml/0.1.30 \
crate://crates.io/pnacl-build-helper/1.4.10 \
crate://crates.io/gcc/0.3.31 \
crate://crates.io/tempdir/0.3.4 \
crate://crates.io/thread-id/2.0.0 \
crate://crates.io/libz-sys/1.0.5 \
crate://crates.io/url/0.2.38 \
crate://crates.io/thread_local/0.2.6 \
crate://crates.io/kernel32-sys/0.2.2 \
crate://crates.io/rustc-serialize/0.3.19 \
crate://crates.io/user32-sys/0.2.0 \
crate://crates.io/regex-syntax/0.3.4 \
crate://crates.io/libressl-pnacl-sys/2.1.6 \
crate://crates.io/crossbeam/0.2.9 \
crate://crates.io/bitflags/0.1.1 \
crate://crates.io/memstream/0.0.1 \
crate://crates.io/winapi-build/0.1.1 \
crate://crates.io/idna/0.1.0 \
crate://crates.io/glob/0.2.11 \
crate://crates.io/semver/0.2.3 \
crate://crates.io/time/0.1.35 \
crate://crates.io/gdi32-sys/0.2.0 \
crate://crates.io/utf8-ranges/0.1.3 \
crate://crates.io/term/0.4.4 \
crate://crates.io/rand/0.3.14 \
crate://crates.io/uuid/0.1.18 \
crate://crates.io/cargo/0.10.0 \
crate://crates.io/curl-sys/0.1.34 \
crate://crates.io/docopt/0.6.81 \
crate://crates.io/regex/0.1.73 \
crate://crates.io/cmake/0.1.17 \
crate://crates.io/log/0.3.6 \
crate://crates.io/aho-corasick/0.5.2 \
crate://crates.io/cargo-bitbake/0.1.0 \
crate-index://crates.io/CARGO_INDEX_COMMIT \
"
SRC_URI[index.md5sum] = "79f10f436dbf26737cc80445746f16b4"
SRC_URI[index.sha256sum] = "86114b93f1f51aaf0aec3af0751d214b351f4ff9839ba031315c1b19dcbb1913"

LIC_FILES_CHKSUM=" \
    file://LICENSE-APACHE;md5=1836efb2eb779966696f473ee8540542 \
    file://LICENSE-MIT;md5=0b29d505d9225d1f0815cbdcf602b901 \
"

SUMMARY = "Generates a BitBake recipe for a package utilizing meta-rust's classes."
HOMEPAGE = "https://github.com/cardoe/cargo-bitbake"
LICENSE = "MIT | Apache-2.0"
```

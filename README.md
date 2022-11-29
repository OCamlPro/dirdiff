[![Crates.io badge](https://img.shields.io/crates/v/dirdiff?style=flat-square)](https://crates.io/crates/dirdiff-ocamlpro)
[![github release badge badge](https://img.shields.io/github/v/release/OCamlPro/dirdiff?style=flat-square)](https://github.com/OCamlPro/dirdiff/releases/latest)
![github downloads badge](https://img.shields.io/github/downloads/OCamlPro/dirdiff/total?style=flat-square)
<br/>
[<img src="resources/red-iron-sponsor.png" alt="This project is proudly sponsored by Red Iron, the Rust division of OCamlPro" width="372"/>](https://red-iron.eu/)

Dirdiff
=======

Dirdiff efficiently computes the differences between two directories. It lists files that either:

1. exist only in one of the directories, or
2. exist in both directories but with different content.

Dirdiff is intended to work on large directories, thanks to multi-threading, and by not trying to display the diff of the files' content.

Installation
------------

## Released binary

Precompiled binaries for (relatively recent) Linux/amd64 are available for every tagged [release](https://github.com/OCamlPro/dirdiff/releases).

## Install (by compiling from sources) using cargo

```
cargo install dirdiff-ocamlpro
```

## Building

Dirdiff is written in Rust. To build it you will need to have the rust toolchain installed. 

Once you have obtained the source, the following command will build the binary and put it in the root directory of the repo.

```bash
cd dirdiff/
cargo build --release
# Copy the binary to the root of the repo
mv target/release/dirdiff dirdiff
```

Usage
-----

```
Usage: dirdiff [OPTIONS] <DIR1> <DIR2>

Arguments:
  <DIR1>
          First directory to diff from

  <DIR2>
          Second directory to diff from

Options:
  -j, --jobs <JOBS>
          Number of parallel threads to use.

          Use 0 or no option for auto-detection.

      --check-mtime
          Whether to check if the mtime is different.

          Only applies to file whose content is otherwise the same, and gets its specific output tag: `[Differ by mtime only]`.

  -L, --follow-symlink
          Whether to follow symlinks when comparing directories' content

  -H
          Whether to follow symlinks for program's arguments

  -h, --help
          Print help information (use `-h` for a summary)

  -V, --version
          Print version information
```

Sample output
-------------

Columns are tab separated

```
[Files differ]	"foo/bar"
[Present in first dir. only]	"subdir_a"
[Present in second dir. only]	"subdir_b"
```

The diff is outputted to `stdout`.
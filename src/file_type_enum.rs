// This code is adapted from:
// https://github.com/marcospb19/file_type_enum
// The fork is made to ensure that directories are ordered first.

//! An enum with a variant for each file type.
//!
//! Note that the [`FileType::from_path`] follows symlinks and [`FileType::from_symlink_path`] does not.
//!
//! [`FileType::from_path`]: https://docs.rs/file_type_enum/latest/file_type_enum/enum.FileType.html#method.from_path
//! [`FileType::from_symlink_path`]: https://docs.rs/file_type_enum/latest/file_type_enum/enum.FileType.html#method.from_symlink_path
//!
//! # Conversions
//!
//! - From [`AsRef<Path>`], [`fs::Metadata`] and [std's `FileType`].
//! - From and into [`libc::mode_t`] (via the feature `"mode-t-conversion"`).
//!
//! [`AsRef<Path>`]: https://doc.rust-lang.org/std/path/struct.Path.html
//! [`fs::Metadata`]: https://doc.rust-lang.org/std/fs/struct.Metadata.html
//! [std's `FileType`]: https://doc.rust-lang.org/std/fs/struct.FileType.html
//! [`libc::mode_t`]: https://docs.rs/libc/latest/libc/type.mode_t.html



#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
// use std::{fmt, fs, io, path::Path};
use std::{fmt, fs};

/// An enum with a variant for each file type.
#[derive(Debug, Eq, PartialEq, Hash, Clone, Copy, Ord, PartialOrd)]
pub enum FileType {
    /// A directory, folder of files.
    Directory,
    /// A regular file (e.g. '.txt', '.rs', '.zip').
    Regular,
    /// A symbolic link, points to another path.
    Symlink,
    /// Unix block device.
    #[cfg(unix)] BlockDevice,
    /// Unix char device.
    #[cfg(unix)] CharDevice,
    /// Unix FIFO.
    #[cfg(unix)] Fifo,
    /// Unix socket.
    #[cfg(unix)] Socket,
}

impl FileType {
    // /// Reads a `FileType` from a path.
    // ///
    // /// This function follows symlinks, so it can never return a `FileType::Symlink`.
    // ///
    // /// # Example
    // ///
    // /// ```rust
    // /// use file_type_enum::FileType;
    // /// use std::io;
    // ///
    // /// fn main() -> io::Result<()> {
    // ///     let is_everything_alright = FileType::from_path("/dev/tty")?.is_char_device();
    // ///     Ok(())
    // /// }
    // /// ```
    // ///
    // /// # Errors
    // ///
    // /// - Path does not exist, or
    // /// - Current user lacks permissions to read `fs::Metadata` of `path`.
    // pub fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
    //     let fs_file_type = fs::metadata(path.as_ref())?.file_type();
    //     let result = FileType::from(fs_file_type);
    //     Ok(result)
    // }

    // /// Reads a `FileType` from a path, considers symlinks.
    // ///
    // /// This function does not follow symlinks, so the result can be the variant `FileType::Symlink` too, unlike [`FileType::from_path`].
    // ///
    // /// # Example
    // ///
    // /// ```rust
    // /// use file_type_enum::FileType;
    // ///
    // /// let path = "/dev/stdout";
    // /// let file_type = FileType::from_symlink_path(path).unwrap();
    // ///
    // /// println!("There's a {file_type} at {path}");
    // /// // Out:  "There's a symlink     at /dev/stdout"
    // /// ```
    // ///
    // /// # Errors
    // ///
    // /// - Path does not exist, or
    // /// - Current user lacks permissions to read `fs::Metadata` of `path`.
    // pub fn from_symlink_path(path: impl AsRef<Path>) -> io::Result<Self> {
    //     let fs_file_type = fs::symlink_metadata(path.as_ref())?.file_type();
    //     let result = FileType::from(fs_file_type);
    //     Ok(result)
    // }

    // /// Returns true if is a [`FileType::Regular`].
    // pub fn is_regular(&self) -> bool {
    //     matches!(self, FileType::Regular)
    // }

    // /// Returns true if is a [`FileType::Directory`].
    // pub fn is_directory(&self) -> bool {
    //     matches!(self, FileType::Directory)
    // }

    // /// Returns true if is a [`FileType::Symlink`].
    // pub fn is_symlink(&self) -> bool {
    //     matches!(self, FileType::Symlink)
    // }

    // /// Returns true if is a [`FileType::BlockDevice`].
    // #[cfg(unix)]
    // pub fn is_block_device(&self) -> bool {
    //     matches!(self, FileType::BlockDevice)
    // }

    // /// Returns true if is a [`FileType::CharDevice`].
    // #[cfg(unix)]
    // pub fn is_char_device(&self) -> bool {
    //     matches!(self, FileType::CharDevice)
    // }

    // /// Returns true if is a [`FileType::Fifo`].
    // #[cfg(unix)]
    // pub fn is_fifo(&self) -> bool {
    //     matches!(self, FileType::Fifo)
    // }

    // /// Returns true if is a [`FileType::Socket`].
    // #[cfg(unix)]
    // pub fn is_socket(&self) -> bool {
    //     matches!(self, FileType::Socket)
    // }
}

impl From<fs::FileType> for FileType {
    fn from(ft: fs::FileType) -> Self {
        // Check each type
        #[cfg(unix)]
        let result = {
            if ft.is_file() {
                FileType::Regular
            } else if ft.is_dir() {
                FileType::Directory
            } else if ft.is_symlink() {
                FileType::Symlink
            } else if ft.is_block_device() {
                FileType::BlockDevice
            } else if ft.is_char_device() {
                FileType::CharDevice
            } else if ft.is_fifo() {
                FileType::Fifo
            } else if ft.is_socket() {
                FileType::Socket
            } else {
                unreachable!("file_type_enum: unexpected file type: {:?}.", ft)
            }
        };

        #[cfg(not(unix))]
        let result = {
            if ft.is_file() {
                FileType::Regular
            } else if ft.is_dir() {
                FileType::Directory
            } else if ft.is_symlink() {
                FileType::Symlink
            } else {
                unreachable!("file_type_enum: unexpected file type: {:?}.", ft)
            }
        };

        result
    }
}

impl From<fs::Metadata> for FileType {
    fn from(metadata: fs::Metadata) -> Self {
        metadata.file_type().into()
    }
}

impl fmt::Display for FileType {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FileType::Regular => write!(f, "regular file"),
            FileType::Directory => write!(f, "directory"),
            FileType::Symlink => write!(f, "symbolic link"),
            #[cfg(unix)] FileType::BlockDevice => write!(f, "block device"),
            #[cfg(unix)] FileType::CharDevice => write!(f, "char device"),
            #[cfg(unix)] FileType::Fifo => write!(f, "FIFO"),
            #[cfg(unix)] FileType::Socket => write!(f, "socket"),
        }
    }
}

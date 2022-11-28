//! # Dirdiff
//!
//! ## Symbolic links handling
//!
//! By default, **dirdiff** treats symbolic links as a third file type along with regular files and directories.
//! A symbolic link can only be equal to another symbolic link that has the same name and points to files with the
//! same name and location relative to the link itself. At this stage link couldn't be compared to other files
//! or directories.
//!
//! **For example:**
//! ```
//! dir1/
//! |---link(&a/file)
//! |---a/
//! |   |--- file
//!
//! dir2/
//! |---link(&a/file)
//! |---b/
//! |---a/
//! |   |--- file
//!
//! ```
//! Here symlink that has name *link* in both directories are considered equal, even if they points to two files with
//! different content.
//!
//! When the *-L* option is passed, links are treated quite differently. Its literal meaning is "follow the links". Links are
//! no longer a separate file type, but an imitation of their own target. That is, they are compared and processed with
//! the same file types that they refer to. The name of the link is still used to compare against other files or other
//! links as a first step in the comparison. If the link points to a file with the same content, even if the target
//! names are different, then the link is equal to what it is compared to.
//!
//! **For example:**
//! ```
//! dir1/
//! |---file(&a/target)
//! |---a/
//! |   |---target
//!
//! dir2/
//! |---file
//! ```
//! Here symlink that has name *file* is equal to the file with the same name in *dir2* only if *dir1/a/target* and
//! *dir2/file* have the same content. When dealing with directories, it verifies recursively their both contents.
//!
//! **For example:**
//! ```
//! dir1/
//! |---src(&a/src)
//! |---a/
//! |   |---src
//! |   |   |---file1
//! |   |   |---file2
//!
//! dir2/
//! |---src
//! |   |---file1
//! |   |---file2
//! ```
//! Here symlink that has name *src* is equal to directory with the same name. The files *file1* and *file2* are verified
//! separately.
//!
//! When the *-H* option is passed, the program arguments are unwinded only if they contain a link.

use anyhow::bail;
use anyhow::Context;
use clap::Parser;
use crossbeam_deque::{Steal, Stealer, Worker};
use crossbeam_utils::Backoff;
use std::fs::canonicalize;
use std::fs::{read_link, DirEntry, FileType, Metadata};
use std::{
    ffi::OsString,
    fs::{read_dir, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc,
    },
    thread,
};

struct StackUnit {
    dir: PathBuf,
}

/// Extraction of useful metadata for iterated files.
#[derive(Debug)]
struct FileT {
    entry: DirEntry,
    file_type: FileType,
    path: Option<PathBuf>,
}

impl FileT {
    /// Create new extraction from directory entry.
    ///
    /// It will check file type, and if [follow_link] flag is set and passed entry points to the symbolic link,
    /// then path and type of target file are cached.
    fn new(entry: DirEntry, follow_link: bool) -> Self {
        let mut file_type = entry.file_type().unwrap();
        if follow_link && file_type.is_symlink() {
            let mut path = entry.path();
            path = canonicalize(&path)
                .with_context(|| format!("Error while following link {}", &path.display()))
                .unwrap();
            file_type = path.metadata().unwrap().file_type();
            FileT {
                entry,
                file_type,
                path: Some(path),
            }
        } else {
            FileT {
                entry,
                file_type,
                path: None,
            }
        }
    }

    /// Returns file name. If structure contains symlink, then its name will be returned instead of its target name.
    fn filename(&self) -> OsString {
        self.entry.file_name()
    }

    /// Path to file. If structure contains symlink, then path to its target is returned at constant time.
    fn path(&mut self) -> &PathBuf {
        if let None = &self.path {
            self.path = Some(self.entry.path());
        }
        self.path.as_ref().unwrap()
    }

    /// Ordering files by their types
    fn file_type_order(&self) -> u8 {
        match self.file_type {
            ft if ft.is_dir() => 0,
            ft if ft.is_symlink() => 1,
            ft if ft.is_file() => 2,
            _ => 3,
        }
    }

    /// Metadata of the file. If structure contains symlink, then metadata to its target is returned.
    fn metadata(&self) -> Metadata {
        match &self.path {
            Some(p) => p.metadata().unwrap(),
            None => self.entry.metadata().unwrap(),
        }
    }
}

struct StackHandle {
    own: Worker<StackUnit>,
    stealers: Vec<Stealer<StackUnit>>,
    non_idle: Arc<AtomicU16>,
}

impl StackHandle {
    fn new(n_threads: u16) -> Vec<Self> {
        let mut workers = Vec::new();
        let mut stealers = vec![Vec::new(); n_threads as usize];
        for i in 0..n_threads {
            let w = Worker::new_lifo();
            for (j, stlrs) in stealers.iter_mut().enumerate() {
                if (i as usize) != j {
                    stlrs.push(w.stealer());
                }
            }
            workers.push(w);
        }
        let non_idle = Arc::new(n_threads.into());
        let mut res = Vec::new();
        for (w, stealers) in workers.into_iter().zip(stealers) {
            res.push(Self {
                own: w,
                stealers,
                non_idle: Arc::clone(&non_idle),
            })
        }
        res
    }
}

#[derive(Debug)]
// TODO check rewrite using reference to pathbuf
enum Diff {
    InDir1Only(PathBuf, OsString),
    InDir2Only(PathBuf, OsString),
    Different(PathBuf, OsString),
    SameButDifferentMTime(PathBuf, OsString),
}

trait DiffHandler {
    fn process(&self, root1: &Path, root2: &Path, diff: Diff);
}

struct DirWorker<H: DiffHandler> {
    root1: PathBuf,
    root2: PathBuf,
    stack: StackHandle,
    diff_handler: Arc<H>,
    check_mtime: bool,
    follow_symlink: bool,
}

impl<H: DiffHandler> DirWorker<H> {
    fn new(
        root1: PathBuf,
        root2: PathBuf,
        diff_handler: Arc<H>,
        stack: StackHandle,
        check_mtime: bool,
        follow_symlink: bool,
    ) -> Self {
        Self {
            root1,
            root2,
            stack,
            diff_handler,
            check_mtime,
            follow_symlink,
        }
    }

    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            if let Some(su) = self.stack.own.pop() {
                self.process_path(su.dir)?;
                continue;
            }
            //TODO(arthur): better ordering
            self.stack.non_idle.fetch_sub(1, Ordering::SeqCst);
            let empty_backoff = Backoff::new();
            let retry_backoff = Backoff::new();
            loop {
                let stollen: Steal<_> = self.stack.stealers.iter().map(|s| s.steal()).collect();
                match stollen {
                    Steal::Retry => {
                        retry_backoff.snooze();
                    }
                    Steal::Empty => {
                        if self.stack.non_idle.load(Ordering::SeqCst) == 0 {
                            return Ok(());
                        }
                        empty_backoff.snooze();
                    }
                    Steal::Success(su) => {
                        self.stack.non_idle.fetch_add(1, Ordering::SeqCst);
                        self.process_path(su.dir)?;
                        break;
                    }
                }
            }
        }
    }

    fn process_diff(&mut self, diff: Diff) {
        self.diff_handler.process(&self.root1, &self.root2, diff)
    }

    fn push_to_stack(&mut self, dir: PathBuf) {
        self.stack.own.push(StackUnit { dir })
    }

    fn process_path(&mut self, dir: PathBuf) -> anyhow::Result<()> {
        // dbg!(&dir);
        let dir1 = PathBuf::from_iter([&self.root1, &dir]);
        let dir2 = PathBuf::from_iter([&self.root2, &dir]);
        let mut dir_content1 = read_dir(&dir1)?
            .map(|r| r.map(|e| FileT::new(e, self.follow_symlink)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut dir_content2 = read_dir(&dir2)?
            .map(|r| r.map(|e| FileT::new(e, self.follow_symlink)))
            .collect::<Result<Vec<_>, _>>()?;
        // Put dir first to minimize time spent with an empty stack
        // in case work needs to be stollen by others
        dir_content1.sort_unstable_by_key(|e| (e.file_type_order(), e.filename()));
        dir_content2.sort_unstable_by_key(|e| (e.file_type_order(), e.filename()));
        loop {
            if dir_content1.is_empty() {
                for e in dir_content2 {
                    self.process_diff(Diff::InDir2Only(dir.clone(), e.filename()))
                }
                return Ok(());
            }
            if dir_content2.is_empty() {
                for e in dir_content1 {
                    self.process_diff(Diff::InDir1Only(dir.clone(), e.filename()))
                }
                return Ok(());
            }
            let e1 = dir_content1.last().unwrap();
            let e2 = dir_content2.last().unwrap();
            let ord1 = e1.file_type_order();
            let ord2 = e2.file_type_order();
            match (ord1, e1.filename()).cmp(&(ord2, e2.filename())) {
                std::cmp::Ordering::Less => {
                    let e = dir_content2.pop().unwrap();
                    self.process_diff(Diff::InDir2Only(dir.clone(), e.filename()));
                    continue;
                }
                std::cmp::Ordering::Greater => {
                    let e = dir_content1.pop().unwrap();
                    self.process_diff(Diff::InDir1Only(dir.clone(), e.filename()));
                    continue;
                }
                std::cmp::Ordering::Equal => {
                    let mut e1 = dir_content1.pop().unwrap();
                    let mut e2 = dir_content2.pop().unwrap();
                    let ft1 = e1.file_type_order();
                    let ft2 = e2.file_type_order();
                    assert_eq!(ft1, ft2);
                    match ft1 {
                        0 => {
                            let mut p = dir.clone();
                            p.push(e1.filename());
                            self.push_to_stack(p);
                        }
                        1 => {
                            if read_link(e1.path())? != read_link(e2.path())? {
                                self.process_diff(Diff::Different(dir.clone(), e1.filename()));
                            }
                        }
                        2 => {
                            let e1_meta = e1.metadata();
                            let e2_meta = e2.metadata();
                            let same_content = if e1_meta.len() != e2_meta.len() {
                                false
                            } else {
                                let mut f1 = BufReader::new(File::open(e1.path())?);
                                let mut f2 = BufReader::new(File::open(e2.path())?);
                                loop {
                                    let s1 = f1.fill_buf()?;
                                    let s2 = f2.fill_buf()?;
                                    if s1.is_empty() {
                                        break s2.is_empty();
                                    }
                                    let common_size = std::cmp::min(s1.len(), s2.len());
                                    if s1[..common_size] != s2[..common_size] {
                                        break false;
                                    } else {
                                        f1.consume(common_size);
                                        f2.consume(common_size);
                                    }
                                }
                            };
                            if !same_content {
                                self.process_diff(Diff::Different(dir.clone(), e1.filename()));
                            } else if self.check_mtime
                                && (e1_meta.modified()? != e2_meta.modified()?)
                            {
                                self.process_diff(Diff::SameButDifferentMTime(
                                    dir.clone(),
                                    e2.filename(),
                                ));
                            }
                        }
                        _ => {
                            let mut p = dir;
                            p.push(e1.filename());
                            bail!(
                                "Unimplemented filetype. File {} has type {:?}",
                                p.display(),
                                ft1
                            );
                        }
                    }
                }
            }
        }
    }
}

struct GrepableHandler;

impl GrepableHandler {
    fn new() -> Self {
        Self
    }
}

impl DiffHandler for GrepableHandler {
    fn process(&self, _root1: &Path, _root2: &Path, diff: Diff) {
        let (diff_type, mut p, file) = match diff {
            Diff::Different(dir, file) => ("Files differ", dir, file),
            Diff::InDir1Only(dir, file) => ("Present in first dir. only", dir, file),
            Diff::InDir2Only(dir, file) => ("Present in second dir. only", dir, file),
            Diff::SameButDifferentMTime(dir, file) => ("Differ by mtime only", dir, file),
        };
        p.push(file);
        println!("[{}]\t{:?}", diff_type, p.display());
    }
}

/// Output the diff of two directories.
///
/// Intended to be efficient and usable on very large directories.
///
/// Not intended to output the diff of files' content.
#[derive(Debug, Parser)]
#[command(author, version)]
struct CliArgs {
    /// First directory to diff from.
    dir1: PathBuf,
    /// Second directory to diff from.
    dir2: PathBuf,
    #[arg(short, long)]
    /// Number of parallel threads to use.
    ///
    /// Use 0 or no option for auto-detection.
    jobs: Option<u16>,
    /// Whether to check if the mtime is different.
    ///
    /// Only applies to file whose content is otherwise the same,
    /// and gets its specific output tag: `[Differ by mtime only]`.
    #[arg(long)]
    check_mtime: bool,
    /// Whether to follow symlinks when comparing directories' content
    #[arg(short = 'L', long)]
    follow_symlink: bool,
    /// Whether to follow symlinks for program's arguments.
    #[arg(short = 'H')]
    follow_symlink_args: bool,
}

fn main() -> anyhow::Result<()> {
    let unwind_path = |path: PathBuf| {
        path.canonicalize()
            .context(format!("Couldn't unwind path {}.", path.display()))
    };
    let cli_args: CliArgs = CliArgs::parse();
    let n_threads = match cli_args.jobs {
        Some(u) if u > 0 => u,
        _ => thread::available_parallelism()
            .context("Could not determine available parallelisme, specify the -j option with a non zero value.")?
            .get() as _,
    };
    let h = Arc::new(GrepableHandler::new());
    let stack_handlers = StackHandle::new(n_threads);
    let mut first = true;
    let mut joins = Vec::new();
    let (dir1, dir2) = if cli_args.follow_symlink_args {
        (unwind_path(cli_args.dir1)?, unwind_path(cli_args.dir2)?)
    } else {
        (cli_args.dir1, cli_args.dir2)
    };
    for sh in stack_handlers {
        let mut worker = DirWorker::new(
            dir1.clone(),
            dir2.clone(),
            h.clone(),
            sh,
            cli_args.check_mtime,
            cli_args.follow_symlink,
        );
        if first {
            worker.push_to_stack(PathBuf::new());
            first = false;
        }
        joins.push(thread::spawn(move || worker.run()));
    }
    for j in joins {
        j.join().unwrap()?;
    }
    Ok(())
}

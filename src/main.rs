use anyhow::bail;
use anyhow::Context;
use clap::Parser;
use crossbeam_deque::{Steal, Stealer, Worker};
use crossbeam_utils::Backoff;
use std::fs::read_link;
use std::sync::atomic::AtomicBool;
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
enum Diff {
    InDir1Only(PathBuf, OsString),
    InDir2Only(PathBuf, OsString),
    Different(PathBuf, OsString),
    SameButDifferentMTime(PathBuf, OsString),
}

trait DiffHandler {
    fn process(&self, root1: &Path, root2: &Path, diff: Diff);

    fn warn_symlink(&self);
}
struct DirWorker<H: DiffHandler> {
    root1: PathBuf,
    root2: PathBuf,
    stack: StackHandle,
    diff_handler: Arc<H>,
    check_mtime: bool,
}

impl<H: DiffHandler> DirWorker<H> {
    fn new(
        root1: PathBuf,
        root2: PathBuf,
        diff_handler: Arc<H>,
        stack: StackHandle,
        check_mtime: bool,
    ) -> Self {
        Self {
            root1,
            root2,
            stack,
            diff_handler,
            check_mtime,
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
        let mut dir_content1 = read_dir(&dir1)?.collect::<Result<Vec<_>, _>>()?;
        let mut dir_content2 = read_dir(&dir2)?.collect::<Result<Vec<_>, _>>()?;
        // Put dir first to minimize time spent with an empty stack
        // in case work needs to be stollen by others
        dir_content1.sort_unstable_by_key(|e| (!e.file_type().unwrap().is_dir(), e.file_name()));
        dir_content2.sort_unstable_by_key(|e| (!e.file_type().unwrap().is_dir(), e.file_name()));
        loop {
            if dir_content1.is_empty() {
                for e in dir_content2 {
                    self.process_diff(Diff::InDir2Only(dir.clone(), e.file_name()))
                }
                return Ok(());
            }
            if dir_content2.is_empty() {
                for e in dir_content1 {
                    self.process_diff(Diff::InDir1Only(dir.clone(), e.file_name()))
                }
                return Ok(());
            }
            let e1 = dir_content1.last().unwrap();
            let e2 = dir_content2.last().unwrap();
            let e1_not_dir = !e1.file_type().unwrap().is_dir();
            let e2_not_dir = !e2.file_type().unwrap().is_dir();
            match (e1_not_dir, e1.file_name()).cmp(&(e2_not_dir, e2.file_name())) {
                std::cmp::Ordering::Less => {
                    let e = dir_content2.pop().unwrap();
                    self.process_diff(Diff::InDir2Only(dir.clone(), e.file_name()));
                    continue;
                }
                std::cmp::Ordering::Greater => {
                    let e = dir_content1.pop().unwrap();
                    self.process_diff(Diff::InDir1Only(dir.clone(), e.file_name()));
                    continue;
                }
                std::cmp::Ordering::Equal => {
                    let e1 = dir_content1.pop().unwrap();
                    let e2 = dir_content2.pop().unwrap();
                    let ft1 = e1.file_type()?;
                    let ft2 = e2.file_type()?;
                    if ft1 != ft2 {
                        self.process_diff(Diff::Different(dir.clone(), e1.file_name()));
                    } else if ft1.is_dir() {
                        let mut p = dir.clone();
                        p.push(e1.file_name());
                        self.push_to_stack(p);
                    } else if ft1.is_symlink() {
                        self.diff_handler.warn_symlink();
                        if read_link(e1.path())? != read_link(e2.path())? {
                            self.process_diff(Diff::Different(dir.clone(), e1.file_name()));
                        }
                    } else if ft1.is_file() {
                        let same_content = if e1.metadata()?.len() != e2.metadata()?.len() {
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
                            self.process_diff(Diff::Different(dir.clone(), e1.file_name()));
                        } else if self.check_mtime
                            && (e1.metadata()?.modified()? != e2.metadata()?.modified()?)
                        {
                            self.process_diff(Diff::SameButDifferentMTime(
                                dir.clone(),
                                e1.file_name(),
                            ));
                        }
                    } else {
                        let mut p = dir;
                        p.push(e1.file_name());
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

struct GrepableHandler {
    warned_symlink: AtomicBool,
}

impl GrepableHandler {
    fn new() -> Self {
        Self {
            warned_symlink: AtomicBool::new(false),
        }
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

    fn warn_symlink(&self) {
        if !self.warned_symlink.swap(true, Ordering::SeqCst) {
            eprintln!(
                "[warning!] A symlink was detected. \
                Be advised that symlink are only checked for equality of their \
                target and not processed recursively."
            )
        }
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
}

fn main() -> anyhow::Result<()> {
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
    for sh in stack_handlers {
        let mut worker = DirWorker::new(
            cli_args.dir1.clone(),
            cli_args.dir2.clone(),
            h.clone(),
            sh,
            cli_args.check_mtime,
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

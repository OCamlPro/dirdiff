use anyhow::bail;
use std::{
    ffi::OsString,
    fs::{read_dir, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
struct StackUnit {
    dir: PathBuf,
}

type Stack = Vec<StackUnit>;

#[derive(Debug)]
enum Diff {
    InDir1Only(PathBuf, OsString),
    InDir2Only(PathBuf, OsString),
    Different(PathBuf, OsString),
}

trait DiffHandler {
    fn process(&mut self, root1: &Path, root2: &Path, diff: Diff);
}
struct Worker<'root, H: DiffHandler> {
    root1: &'root Path,
    root2: &'root Path,
    stack: Stack,
    diff_handler: H,
}

impl<'root, H: DiffHandler> Worker<'root, H> {
    fn new<P: AsRef<Path>>(root1: &'root P, root2: &'root P, diff_handler: H) -> Self {
        let w = Self {
            root1: root1.as_ref(),
            root2: root2.as_ref(),
            stack: Stack::default(),
            diff_handler,
        };
        w
    }

    fn run(&mut self) -> anyhow::Result<()> {
        while let Some(sunit) = self.stack.pop() {
            self.process_path(sunit.dir)?;
        }
        Ok(())
    }

    fn process_diff(&mut self, diff: Diff) {
        self.diff_handler.process(self.root1, self.root2, diff)
    }

    fn push_to_stack(&mut self, dir: PathBuf) {
        self.stack.push(StackUnit { dir })
    }

    fn process_path(&mut self, dir: PathBuf) -> anyhow::Result<()> {
        // dbg!(&dir);
        let dir1 = PathBuf::from_iter([self.root1, &dir]);
        let dir2 = PathBuf::from_iter([self.root2, &dir]);
        let mut dir_content1 = read_dir(&dir1)?.collect::<Result<Vec<_>, _>>()?;
        let mut dir_content2 = read_dir(&dir2)?.collect::<Result<Vec<_>, _>>()?;
        dir_content1.sort_unstable_by_key(|e| e.file_name());
        dir_content2.sort_unstable_by_key(|e| e.file_name());
        loop {
            if dir_content1.len() == 0 {
                for e in dir_content2 {
                    self.process_diff(Diff::InDir2Only(dir.clone(), e.file_name()))
                }
                return Ok(());
            }
            if dir_content2.len() == 0 {
                for e in dir_content1 {
                    self.process_diff(Diff::InDir1Only(dir.clone(), e.file_name()))
                }
                return Ok(());
            }
            let e1 = dir_content1.last().unwrap();
            let e2 = dir_content2.last().unwrap();
            match e1.file_name().cmp(&e2.file_name()) {
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
                        // todo!("Symlink are not currently handled")
                        //TODO(Arthur): better
                        continue;
                    } else if ft1.is_file() {
                        let same_content;
                        if e1.metadata()?.len() != e2.metadata()?.len() {
                            same_content = false;
                        } else {
                            let mut f1 = BufReader::new(File::open(e1.path())?);
                            let mut f2 = BufReader::new(File::open(e2.path())?);
                            same_content = loop {
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
                        }
                        if !same_content {
                            self.process_diff(Diff::Different(dir.clone(), e1.file_name()));
                        }
                    } else {
                        let mut p = dir.clone();
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

struct GrepableHandler {}

impl GrepableHandler {
    fn new() -> Self {
        Self {}
    }
}

impl DiffHandler for GrepableHandler {
    fn process(&mut self, _root1: &Path, _root2: &Path, diff: Diff) {
        let (diff_type, mut p, file) = match diff {
            Diff::Different(dir, file) => ("Files differ", dir, file),
            Diff::InDir1Only(dir, file) => ("Present in first dir. only", dir, file),
            Diff::InDir2Only(dir, file) => ("Present in second dir. only", dir, file),
        };
        p.push(file);
        println!("[{}]\t{:?}", diff_type, p.display());
    }
}

fn main() -> anyhow::Result<()> {
    let h = GrepableHandler::new();
    let mut worker = Worker::new(
        &"/home/arthur/mozilla-unified",
        &"/home/arthur/mozilla-unified-post-build",
        h,
    );
    worker.push_to_stack(PathBuf::new());
    worker.run()
}

use std::ffi::CString;

use clap::ArgMatches;
use fnmatch_sys::{fnmatch, FNM_NOMATCH};
use std::collections::HashSet;
use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
use std::path::Path;
use subprocess::Exec;

/// Describes a command line given predicate.
type Predicate = Box<dyn Fn(&Path, &Metadata) -> io::Result<bool>>;

/// Provides a filter for the --type flag.
///
/// Panics on unknown flag. This should be handled by Clap.
fn type_predicate(predicate: Predicate, accepted: Vec<String>) -> Predicate {
    macro_rules! false_when {
        ($b:expr) => {
            if $b {
                return Ok(false);
            }
        };
    }
    Box::new(move |p, m: &Metadata| {
        let ft = m.file_type();
        Ok(predicate(p, m)? && {
            for t in &accepted {
                let t: &str = &t;
                match t {
                    "b" => false_when!(!ft.is_block_device()),
                    "c" => false_when!(!ft.is_char_device()),
                    "d" => false_when!(!ft.is_dir()),
                    "p" => false_when!(!ft.is_fifo()),
                    "f" => false_when!(!ft.is_file()),
                    "l" => false_when!(!ft.is_symlink()),
                    "s" => false_when!(!ft.is_socket()),
                    _ => panic!("Found unimplemented type"),
                }
            }
            true
        })
    })
}

/// Filters on the `--mtime` argument.
fn time_predicate(predicate: Predicate, accepted: i32) -> Predicate {
    Box::new(move |p, m: &Metadata| {
        Ok(predicate(p, m)? && {
            let modified = m.modified()?;
            let now = std::time::SystemTime::now();
            let time_delta = if modified > now {
                modified.duration_since(now).unwrap().as_secs()
            } else {
                now.duration_since(modified).unwrap().as_secs()
            } as f64
                / 60.0 // to minutes
                / 60.0 // To hours
                / 24.0; // To days
            if accepted == 0 {
                time_delta < 1.0
            } else if accepted > 0 {
                time_delta > accepted as f64
            } else {
                time_delta < (-accepted) as f64
            }
        })
    })
}

/// Filters on the `--name` argument.
///
/// Panics when `fnmatch` provides an error code.
fn name_predicate(predicate: Predicate, name: CString) -> Predicate {
    Box::new(move |p, m| {
        Ok(predicate(p, m)? && {
            let path = p.components().last().unwrap().as_os_str();
            let path = unsafe { CString::from_vec_unchecked(path.as_bytes().to_vec()) };
            let result = unsafe {
                fnmatch(
                    name.as_ptr() as _,
                    path.as_bytes_with_nul().as_ptr() as _,
                    0,
                )
            };
            if result == 0 {
                true
            } else if result == unsafe { FNM_NOMATCH } {
                false
            } else {
                panic!("fnmatch failed")
            }
        })
    })
}

/// Filters on the `--exec` predicate.
///
/// Panics when the pipe fails.
fn exec_predicate(predicate: Predicate, command: String) -> Predicate {
    Box::new(move |p, m| {
        Ok(predicate(p, m)? && {
            match Exec::shell(command.to_string().replace("{}", &p.to_string_lossy())).join() {
                Ok(_) => {}
                Err(e) => Error::Custom(&e).sig(),
            };
            false
        })
    })
}

type SigError = bool;

/// Recursivly traverse `path`. Only adds if `predicate(path, metadata(path))`
/// returns true. If a file is a directory, `crawl_path` will still traverse.
/// Symlinks will be followed if `follow_syms` is `true`.
pub fn crawl_path(
    path: &Path,
    predicate: &Predicate,
    follow_syms: bool,
    visited: &mut HashSet<u64>,
) -> Result<SigError, io::Error> {
    let meta = if follow_syms {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    if predicate(path, &meta)? {
        println!("{}", path.display());
    }
    let mut sig_error = false;
    if meta.is_dir()
        && (follow_syms || !meta.file_type().is_symlink())
        && visited.insert(meta.ino())
    // This tests if meta.ino() is already in
    // visited
    {
        for fs in std::fs::read_dir(path)? {
            let fs = fs?.path();
            match crawl_path(&fs, predicate, follow_syms, visited) {
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::NotFound => println!("{}", fs.display()),
                        _ => Error::from_io(e, fs.display()).sig(),
                    }
                    sig_error = true;
                }
                Ok(sig) => sig_error = sig,
            }
        }
    }
    Ok(sig_error)
}

/// Takes args given and forms a predicate to correctly filter them.
pub fn form_predicate(opts: &ArgMatches) -> Predicate {
    // Default predicate: everything passes.
    let mut predicate: Predicate = Box::new(|_, _| Ok(true));
    if let Some(types) = opts.values_of("type").take() {
        // Apply type arg
        predicate = type_predicate(predicate, types.map(|f| f.to_string()).collect());
    }
    if let Some(names) = opts.values_of("name").take() {
        // apply name are
        predicate = names.fold(predicate, |predicate, name| {
            let name = unsafe { CString::from_vec_unchecked(name.as_bytes().to_vec()) };
            name_predicate(predicate, name)
        })
    }
    if let Some(mtimes) = opts.values_of("mtime").take() {
        predicate = mtimes.fold(predicate, |predicate, mtime| {
            time_predicate(predicate, mtime.parse().unwrap())
        })
    }
    if let Some(execs) = opts.values_of("exec").take() {
        predicate = execs.fold(predicate, |predicate, exec| {
            exec_predicate(predicate, exec.to_string())
        })
    }
    // As far as I can tell, `-print` does not do anything for this stub of a
    // program.
    //
    // if opts.is_present("print") {
    //     // we print everything anyway
    //     predicate = Box::new(move |p, m| {
    //         predicate(p, m)?;
    //         Ok(true)
    //     });
    // }
    predicate
}

pub enum Error<T: std::fmt::Display> {
    TooManySymlinks(T),
    Custom(T),
    NoSuchFile(T),
}

impl<T: std::fmt::Display> std::fmt::Display for Error<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Error::TooManySymlinks(s) => {
                write!(f, "‘{}’: Too many levels of symbolic links", s)
            }
            Error::Custom(s) => {
                write!(f, "{}", s)
            }
            Error::NoSuchFile(s) => {
                write!(f, "‘{}’: No such file or directory", s)
            }
        }
    }
}

impl<T: std::fmt::Display> Error<T> {
    /// Signal an error to the user. Does not exit.
    pub fn sig(&self) {
        eprintln!("gfind: {}", self);
    }

    pub fn from_io(error: io::Error, path: T) -> Error<String> {
        match error.kind() {
            // The only way to examine a file that doesn't exist is to fail at
            // the first search queary, or to have the file system change while
            // a search is occuring. While the second is possible, I don't think
            // it likely to be tested.
            std::io::ErrorKind::NotFound => Error::NoSuchFile(format!("{}", path)),
            std::io::ErrorKind::Other if error.raw_os_error() == Some(62) => {
                Error::TooManySymlinks(format!("{}", path))
            }
            _ => Error::Custom(format!("{}", error)),
        }
    }
}

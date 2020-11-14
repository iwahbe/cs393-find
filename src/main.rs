use clap::{App, Arg, ArgMatches};
use std::fs::Metadata;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::path::PathBuf;

use std::ffi::CString;

use fnmatch_sys::{fnmatch, FNM_NOMATCH};

fn getopts() -> ArgMatches {
    App::new("Something like unix `find` command.")
                .version("0.1")
                .author("Ian Wahbe <github @iwahbe>")
                .about("An reimplimentation of find for scholastic purposes.")
                .arg(Arg::new("L").short('L').about("Follow symbolic links"))
                .arg(Arg::new("starting_point").about("Starting point for search"))
                .arg(Arg::new("name")
                        .long("name")
                        .value_name("pattern")
                        .about("Sets a custom config file")
                        .takes_value(true))
                .arg(Arg::new("mtime")
                        .long("mtime")
                        .about("File's data was last modified less than, more than or exactly n*24 hours ago")
                        .takes_value(true)
                        .value_name("n"))
                .arg(Arg::new("type")
                        .long("type")
                        .takes_value(true)
                     .value_name("t")
                .possible_values(&["b", "c", "d", "p", "f", "l", "s"]).require_delimiter(true))
                .arg(Arg::new("exec")
                        .long("exec")
                        .about(
                                "Execute `command`; true if 0 status is returned. All following arguments to find are taken to be arguments to the command until an argument consisting of `;' is encountered. The string `{}' is replaced by the current file name being processed everywhere it occurs in the arguments to the command.",
                        )
                        .takes_value(true)
                        .value_name("command"))
                .arg(Arg::new("print")
                        .long("print")
                        .about("True; print the full file name on the standard output, followed by a newline"))
                .get_matches()
}

type Predicate = Box<dyn Fn(&Path, &Metadata) -> bool>;

fn type_predicate(predicate: Predicate, accepted: Vec<String>) -> Predicate {
    Box::new(move |p, m: &Metadata| {
        let ft = m.file_type();
        predicate(p, m)
            && if ft.is_block_device() {
                accepted.contains(&String::from("b"))
            } else if ft.is_char_device() {
                accepted.contains(&String::from("c"))
            } else if ft.is_dir() {
                accepted.contains(&String::from("d"))
            } else if ft.is_fifo() {
                accepted.contains(&String::from("p"))
            } else if ft.is_file() {
                accepted.contains(&String::from("f"))
            } else if ft.is_symlink() {
                accepted.contains(&String::from("l"))
            } else if ft.is_socket() {
                accepted.contains(&String::from("s"))
            } else {
                panic!("Found unimplemented type")
            }
    })
}

fn name_predicate(predicate: Predicate, name: CString) -> Predicate {
    Box::new(move |p, m| {
        predicate(p, m) && {
            let path = p.file_name().unwrap();
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
        }
    })
}

fn main() -> std::io::Result<()> {
    let opts = getopts();
    let starting_point = Path::new(opts.value_of("starting_point").unwrap_or("."))
        .canonicalize()
        .unwrap();
    // Default predicate: everything passes.
    let mut predicate: Predicate = Box::new(|_, _| true);
    if let Some(types) = opts.values_of("type").take() {
        // Apply type arg
        predicate = type_predicate(predicate, types.map(|f| f.to_string()).collect());
    }
    if let Some(name) = opts.value_of("name").take() {
        // apply name are
        let name = unsafe { CString::from_vec_unchecked(name.as_bytes().to_vec()) };
        predicate = name_predicate(predicate, name);
    }
    for p in crawl_path(&starting_point, &predicate, opts.is_present("L"))? {
        println!("{}", p.display());
    }
    Ok(())
}

fn crawl_path<P: Fn(&Path, &Metadata) -> bool + ?Sized>(
    path: &Path,
    predicate: &P,
    follow_syms: bool,
) -> Result<impl Iterator<Item = PathBuf>, std::io::Error> {
    let meta = if follow_syms {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    if path.is_dir() {
        let mut out: Vec<PathBuf> = Vec::new();
        for fs in std::fs::read_dir(path).unwrap() {
            out.extend(crawl_path(&fs.unwrap().path(), predicate, follow_syms)?)
        }
        Ok(out.into_iter())
    } else {
        if predicate(path, &meta) {
            Ok(vec![(path).to_path_buf()].into_iter())
        } else {
            Ok(vec![].into_iter())
        }
    }
}

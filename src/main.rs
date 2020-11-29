use clap::{App, Arg, ArgMatches};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::process::exit;

mod lib;
use lib::{crawl_path, form_predicate, preprocess_args, Error};

/// Process command line arguments into a usefull output.
/// This uses the `Clap` library.
fn getopts(preprocessed_args: Vec<String>) -> ArgMatches {
    App::new("Something like unix `find` command.")
        .version("0.1")
        .author("Ian Wahbe <github @iwahbe>")
        .about("An reimplimentation of find for scholastic purposes.")
        .arg(Arg::new("L").short('L').about("Follow symbolic links"))
        .arg(Arg::new("C").short('C').about("Canonicalize paths"))
        .arg(
            Arg::new("starting_point")
                .about("Starting point for search")
                .multiple(true),
        )
        .arg(
            Arg::new("name")
                .long("name")
                .value_name("pattern")
                .about("Filters file names according to a glob expanded path.")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("mtime")
                .long("mtime")
                .about(
                    "File's data was last modified less than, more than or exactly n*24 hours ago",
                )
                .takes_value(true)
                .value_name("n")
                .validator(|s| match s.parse::<i32>() {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                })
                .allow_hyphen_values(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .takes_value(true)
                .value_name("t")
                .possible_values(&["b", "c", "d", "p", "f", "l", "s"])
                .require_delimiter(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("exec")
                .long("exec")
                .about(
                    "Execute `command`; true if 0 status is returned. All \
                                 following arguments to find are taken to be arguments \
                                 to the command until an argument consisting of `;' is \
                                 encountered. The string `{}' is replaced by the current \
                                 file name being processed everywhere it occurs in the \
                                 arguments to the command.",
                )
                .takes_value(true)
                .value_name("command")
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("print")
                .long("print")
                .about(
                    "True; print the full file name on the standard output, followed by a newline",
                )
                .multiple_occurrences(true),
        )
        .get_matches_from(preprocessed_args)
}

fn main() -> io::Result<()> {
    let args = std::env::args();
    let pre_process = match preprocess_args(args) {
        Ok(args) => args,
        Err(e) => {
            Error::Custom(e).sig();
            exit(1);
        }
    };
    let opts = getopts(pre_process);
    let starting_points: Vec<_> = {
        opts.values_of("starting_point")
            .unwrap()
            .map(|path| {
                let p = Path::new(path);
                if opts.is_present("C") {
                    p.canonicalize().unwrap()
                } else {
                    p.to_path_buf()
                }
            })
            .collect()
    };
    let mut error_no: i32 = 0;
    for starting_point in starting_points {
        let mut visited = HashSet::new();
        let predicate = form_predicate(&opts);
        match crawl_path(
            &starting_point,
            &predicate,
            opts.is_present("L"),
            &mut visited,
        ) {
            Ok(sig_error) => {
                if sig_error {
                    error_no = 1;
                }
            }
            Err(error) => {
                Error::from_io(error, starting_point.display()).sig();
                error_no = 1;
            }
        }
    }
    exit(error_no);
}

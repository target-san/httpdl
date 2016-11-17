#[macro_use]
extern crate clap;
extern crate hyper;
extern crate thread_scoped;

use std::process::exit;
use std::fmt::Arguments;
use std::fs;
use std::io::{self, Write};
use std::str::FromStr;
use std::path::Path;
use std::sync::{Mutex};
use std::collections::{HashMap};

use thread_scoped::scoped;

fn pull_files<'a, I>(dest_folder: &'a str, list: &'a Mutex<I>)
    where I: Iterator<Item = (&'a str, Vec<&'a str>)> + Send
{
    loop {
        // Having this as separate expression should prevent locking for the whole duration
        let item = list.lock().unwrap().next();
        match item {
            None => return,
            Some((url, dests)) => {
                let client = hyper::Client::new();
                // TODO: load each acquired URL here
            }
        }
    }
}

fn main() {
    // First, configure our command line
    let args = clap_app!(httpdl =>
        (version: crate_version!())
        (author:  crate_authors!())
        (about: "Downloads files via HTTP")
        (@arg directory:   -o +required +takes_value
            "Directory where to place downloaded files"
        )
        (@arg list_file:   -f +required +takes_value
            "File which contains list of URLs to download and the local file names to use"
        )
        (@arg speed_limit: -l +takes_value
            "Limit download speed to specified value, in bytes; suffixes like 'k' (kilobytes) and 'm' (megabytes)"
        )
        (@arg threads_num: -n +takes_value
            "Download files using N threads"
        )
    ).get_matches();
    // Complains about error processing specified arg, then dumps usage and exits with error
    let fail_arg = |name: &str, format: Arguments| -> ! {
        let _ = writeln!(&mut io::stderr(), "<{}>: {}\n{}", name, format, &args.usage());
        exit(1)
    };

    // Read and validate destination dir
    // NB: can unwrap because it's required and thus can't be None
    let dest_dir    = args.value_of("directory").unwrap();
    {
        match fs::metadata(dest_dir) {
            Err(_)   => fail_arg("directory", format_args!("'{}' does not exist or inaccessible", dest_dir)),
            Ok(meta) => if !meta.is_dir() {
                fail_arg("directory", format_args!("'{}' is not a directory", dest_dir))
            }
        }
    }
    // Read and validate path to lists file
    // NB: can unwrap because it's required and thus can't be None
    let list_file   = args.value_of("list_file").unwrap();
    {
        match fs::metadata(list_file) {
            Err(_)   => fail_arg("list_file", format_args!("'{}' does not exist or inaccessible", list_file)),
            Ok(meta) => if !meta.is_file() {
                fail_arg("list_file", format_args!("'{}' is not a file", list_file))
            }
        }
    }
    // Read and parse download speed limit
    let speed_limit = match args.value_of("speed_limit") {
        None => 0,
        Some(value) => 0 // TODO: proper parsing
    };
    // Read and parse number of threads which should be used
    let threads_num = match args.value_of("threads_num") {
        None => 1usize,
        Some(value) => match usize::from_str(value) {
            Err(_)  => fail_arg("threads_num", format_args!("'{}' cannot be parsed as unsigned integer", value)),
            Ok(0)   => fail_arg("threads_num", format_args!("cannot be zero")),
            Ok(num) => num,
        }
    };

    let files_map: HashMap<&str, Vec<&str>> = HashMap::new();
    // TODO: fill files map
    let files_seq = Mutex::new(files_map.into_iter().fuse());
    // Now, create N - 1 worker threads and each will pull files
    // Looks simpler than fancy tricks like recursive guards
    let mut worker_guards = Vec::with_capacity(threads_num);
    for _ in 1..threads_num {
        worker_guards.push(
            unsafe { scoped(|| pull_files(dest_dir, &files_seq)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(dest_dir, &files_seq);
    // Vector of guards will stop right here
}

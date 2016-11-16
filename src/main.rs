#[macro_use]
extern crate clap;
extern crate hyper;
extern crate thread_scoped;

use std::str::FromStr;
use std::path::Path;
use std::sync::{Mutex};
use std::collections::{HashMap};

use thread_scoped::scoped;

fn pull_files<'a, I>(n_children: usize, dest_folder: &'a Path, list: &'a Mutex<I>)
    where I: Iterator<Item = (&'a str, Vec<&'a str>)> + Send
{
    // If child processing threads are required, then construct scoped thread
    // which would receive all the arguments and children count reduced by 1
    let child_thread_guard =
        if n_children == 0 { None }
        else {
            Some(unsafe { scoped(|| pull_files(n_children - 1, dest_folder, list)) })
        };

    loop {
        // Having this as separate expression should prevent locking for the whole duration
        let item = list.lock().unwrap().next();
        match item {
            None => return,
            Some((url, dests)) => {
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

    let dest_dir    = Path::new(args.value_of("directory").unwrap_or("."));
    let list_file   = args.value_of("list_file").unwrap();
    let speed_limit = args.value_of("speed_limit")
        .map(|lim| usize::from_str(lim));
    let threads_num = 1; /*args.value_of("threads_num")
        .map(|n| usize::from_str(n)).unwrap_or(1);*/

    let files_map: HashMap<&str, Vec<&str>> = HashMap::new();

    let files_seq = Mutex::new(files_map.into_iter().fuse());

    pull_files(threads_num - 1, dest_dir, &files_seq);
}

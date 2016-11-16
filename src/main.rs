extern crate clap;
extern crate hyper;
extern crate thread_scoped;

use std::path::Path;
use std::sync::{Mutex};
use std::collections::{HashMap};

use thread_scoped::scoped;

fn pull_files<'a, I>(dest_folder: &'a Path, list: &'a Mutex<I>)
    where I: Iterator<Item = (&'a str, Vec<&'a str>)>
{
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
    let dest_folder = Path::new("/home/igor");
    let n_threads = 2;
    let files_map: HashMap<&str, Vec<&str>> = HashMap::new();

    let files_seq = Mutex::new(files_map.into_iter().fuse());
    let mut n_guards  = Vec::with_capacity(n_threads - 1);

    for _ in 0..n_threads - 1 {
        n_guards.push(
            unsafe { scoped(|| pull_files(&dest_folder, &files_seq) ) }
        );
    }

    pull_files(&dest_folder, &files_seq);
}

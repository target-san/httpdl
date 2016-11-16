extern crate clap;
extern crate hyper;
extern crate thread_id; // FIXME: drop this one, not needed
extern crate thread_scoped;

use std::path::Path;
use std::sync::{Mutex};
use std::collections::{HashMap};

use thread_scoped::scoped;

fn pull_files<'a>(dest_folder: &Path, list: &'a Mutex<HashMap<&'a str, Vec<&'a str> >>) {
    println!("Thread {:?} finished", thread_id::get());
}

fn main() {
    let dest_folder = Path::new("/home/igor");
    let n_threads = 2;
    let files_map = Mutex::new(HashMap::new());
    let mut n_guards  = Vec::with_capacity(n_threads - 1);

    for _ in 0..n_threads - 1 {
        n_guards.push(
            unsafe { scoped(|| pull_files(&dest_folder, &files_map) ) }
        );
    }

    pull_files(&dest_folder, &files_map);
}

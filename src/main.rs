// First, we declare all external dependencies we need here
use std::cell::RefCell;
// Next, import actual symbols and modules we need
use std::fs;
// NB: to use Read and Write traits, we need to bring them into scope explicitly
use std::io::Read;
use std::path::Path;
use std::process::exit;
use std::sync::Mutex;

use clap::Parser;
use thread_scoped::scoped;

use thiserror::Error;

mod token_bucket;
use token_bucket::TokenBucket;

mod config;
use config::Config;

mod copy_with_speedlimit;
use copy_with_speedlimit::copy_with_speedlimit;

// Program starting point, as usual
fn main() {
    // First, parse arguments
    let Config { dest_dir, list_file, threads_num, speed_limit } = Config::parse();
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        // Open file with list of files to download
        let mut fd = match fs::File::open(&list_file) {
            Ok(val) => val,
            Err(err)  => {
                eprintln!("Failed to open {}: {}", list_file, err);
                exit(1)
            }
        };
        // Then read all of its contents into buffer
        let mut text = String::new();
        if let Err(err) = fd.read_to_string(&mut text) {
            eprintln!("Failed to read contents of {}: {}", list_file, err);
            exit(1)
        }
        text
    };
    // Next, we split the whole file into lines in-place
    // And for each line which contains proper url-filename tuple,
    // We yield that tuple
    let files_seq = all_text
        .lines()
        .filter_map(|line| {
            let mut pieces = line.split(|c| " \r\n\t".contains(c)).filter(|s| !s.is_empty());
            let url = pieces.next();
            let filename = pieces.next();
            if let (Some(url_value), Some(fname_value)) = (url, filename) {
                Some((url_value, fname_value))
            }
            else { None }
        })
        .fuse();
    // If number of threads is 1, use non-threaded version
    if threads_num == 1 {
        // In case of single thread, wrap both files sequence and bucket
        // into RefCell to preserve immutable-to-mutable lifetimes transition
        // In principle, this may be achieved via smth like UnsafeCell (?),
        // but we don't care that much about performance
        let files_seq = RefCell::new(files_seq);
        let fetch_src_dest = move || {
            files_seq
                .try_borrow_mut()
                .ok()
                .map(|mut seq| seq.next())
                .flatten()
        };

        let bucket = RefCell::new(TokenBucket::new(speed_limit));
        let take_tokens = move |amount| {
            bucket
                .try_borrow_mut()
                .ok()
                .map(|mut bucket| bucket.take(amount))
                .unwrap_or(0)
        };
        
        pull_files(0, &dest_dir, &take_tokens, &fetch_src_dest);
        return;
    }
    // Otherwise, pack files sequence and token bucket under mutex
    // and run N-1 additional threads

    // To consume this iterator in case of multithreading, we put it under mutex
    let files_seq = Mutex::new(files_seq);
    let fetch_src_dest = move || {
        files_seq
            .try_lock()
            .ok()
            .map(|mut seq| seq.next())
            .flatten()
    };
    // Also, construct token bucket to control download speed
    // And put it under mutex likewise
    let bucket = Mutex::new(TokenBucket::new(speed_limit));
    let take_tokens = move |amount| {
        bucket
            .try_lock()
            .ok()
            .map(|mut bucket| bucket.take(amount))
            .unwrap_or(0)
    };

    // Pool for N-1 worker thread guards 
    let mut worker_guards = Vec::with_capacity(threads_num - 1);
    // Finally, create worker threads
    for i in 1..threads_num {
        // A minor annoyance - we need to create separate reference variables
        let fetch_src_dest_ref = &fetch_src_dest;
        let take_tokens_ref = &take_tokens;
        let dest_dir_ref = &dest_dir;
        // Create scoped worker thread and put its guard object to vector
        worker_guards.push(
            unsafe { scoped(move || pull_files(i, dest_dir_ref, take_tokens_ref, fetch_src_dest_ref)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(0, &dest_dir, &take_tokens, &fetch_src_dest);
    // Vector of guards will be dropped right here
}

fn pull_files<'a>(
    thread_num: usize,
    dest_dir: &str,
    bucket: &impl Fn(usize) -> usize,
    fetch_src_dest: &impl Fn() -> Option<(&'a str, &'a str)>
) {
    loop {
        // Retrieve next file in sequence
        let (url, dest_path) = match fetch_src_dest() {
            None => break,
            Some((url, dest)) => (url, Path::new(dest_dir).join(dest)) 
        };
        println!("#{}: Downloading {} -> {}", thread_num, url, dest_path.display());
        if let Err(error) = pull_file(url, &dest_path, bucket) {
            eprintln!("#{}: Failed {} -> {} due to:\n    {}", thread_num, url, dest_path.display(), error);
        }
    }
}

#[derive(Error, Debug)]
enum DownloadError {
    #[error("HTTP request error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("HTTP request error: code {0}")]
    StatusCode(reqwest::StatusCode),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn pull_file(src_url: &str, dest_path: &Path, bucket: &impl Fn(usize) -> usize) -> Result<(), DownloadError> {
    let mut response = reqwest::blocking::get(src_url)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(DownloadError::StatusCode(response.status()));
    }
    let mut dest_file = fs::File::create(&dest_path)?;
    let _ = copy_with_speedlimit(&mut response, &mut dest_file, bucket)?;
    Ok(())
}

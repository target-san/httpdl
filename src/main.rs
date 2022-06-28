//
// Uses from stdlib
//
use std::cell::RefCell;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
//
// Uses from external crates
//
use clap::Parser;
use crossbeam_utils::thread::scope;
use anyhow::Result;
//
// Submodules
//
mod token_bucket;
use token_bucket::TokenBucket;

mod config;
use config::Config;

mod copy_with_speedlimit;
use copy_with_speedlimit::copy_with_speedlimit;

// Program starting point, as usual
fn main() -> Result<()> {
    // First, parse arguments
    let Config { dest_dir, list_file, threads_num, speed_limit } = Config::try_parse()?;
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        // Open file with list of files to download
        let mut fd = fs::File::open(&list_file)?;
        // Then read all of its contents into buffer
        let mut text = String::new();
        fd.read_to_string(&mut text)?;
        text
    };
    // Next, we split the whole file into lines in-place
    // And for each line which contains proper url-filename tuple,
    // We yield that tuple
    let mut files_seq = all_text
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

    let mut bucket = TokenBucket::new(speed_limit);
    let dest_dir = Path::new(&dest_dir);

    let _ = copy_streams(
        threads_num,
        move |_| {
            files_seq.next().map(|(url, name)| {
                let dest_path = dest_dir.join(name);
                let response = reqwest::blocking::get(url)?;
                let dest_file = fs::File::create(&dest_path)?;
                
                Ok((response, dest_file))
            })
        },
        move |amount| bucket.take(amount)
    );

    Ok(())
}

fn copy_streams<F, L, R, W>(threads_num: usize, stream_fn: F, limiter_fn: L) -> Result<()>
    where
        F: FnMut(usize) -> Option<Result<(R, W)>>,
        F: Send,
        L: FnMut(usize) -> usize,
        L: Send,
        R: Read,
        W: Write
{
    assert!(threads_num > 0);

    if threads_num == 1 {
        let stream_fn = RefCell::new(stream_fn);
        let stream_fn = move |thread_id| {
            stream_fn
                .try_borrow_mut()
                .ok()
                .map(|mut inner| inner(thread_id))
                .flatten()
        };

        let limiter_fn = RefCell::new(limiter_fn);
        let limiter_fn = move |amount| {
            limiter_fn
                .try_borrow_mut()
                .ok()
                .map(|mut inner| inner(amount))
                .unwrap_or(0)
        };

        copy_streams_thread(0, &stream_fn, &limiter_fn);

        return Ok(());
    }

    let stream_fn = Mutex::new(stream_fn);
    let stream_fn = move |thread_id| {
        stream_fn
            .try_lock()
            .ok()
            .map(|mut inner| inner(thread_id))
            .flatten()
    };

    let limiter_fn = Mutex::new(limiter_fn);
    let limiter_fn = move |amount| {
        limiter_fn
            .try_lock()
            .ok()
            .map(|mut inner| inner(amount))
            .unwrap_or(0)
    };

    let _ = scope(|s| {
        for i in 1..threads_num {
            // Create separate references, so that we can move them to scoped thread
            let thread_id = i;
            let stream_fn_ref = &stream_fn;
            let limiter_fn_ref = &limiter_fn;

            s.spawn(move |_| copy_streams_thread(thread_id, stream_fn_ref, limiter_fn_ref));
        }

        copy_streams_thread(0, &stream_fn, &limiter_fn);
    });

    return Ok(());
}

fn copy_streams_thread<R: Read, W: Write>(
    thread_id:  usize,
    stream_fn:  &impl Fn(usize) -> Option<Result<(R, W)>>,
    limiter_fn: &impl Fn(usize) -> usize
) {
    loop {
        // Retrieve next file in sequence
        let (mut reader, mut writer) = match stream_fn(thread_id) {
            None             => break,    // No additional jobs
            Some(Err(_))     => continue, // Failed, maybe need to do additional reporting
            Some(Ok((r, w))) => (r, w) 
        };

        let _ = copy_with_speedlimit(&mut reader, &mut writer, &limiter_fn);
    }
}

//
// Uses from stdlib
//
use std::io::Read;
use std::path::Path;
//
// Uses from external crates
//
use anyhow::Result;
use clap::Parser;
use futures::StreamExt;
//
// Submodules
//
mod token_bucket;

mod config;
use config::Config;

mod copy_with_speedlimit;

mod downloader;
use downloader::{new_downloader, Progress};

// Program starting point, as usual
fn main() -> Result<()> {
    // First, parse arguments
    let Config {
        dest_dir,
        list_file,
        threads_num,
        speed_limit,
    } = Config::try_parse()?;
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        // Open file with list of files to download
        let mut fd = std::fs::File::open(&list_file)?;
        // Then read all of its contents into buffer
        let mut text = String::new();
        fd.read_to_string(&mut text)?;
        text
    };
    // Next, we split the whole file into lines in-place
    // And for each line which contains proper url-filename tuple,
    // We yield that tuple
    let files_seq = all_text
        .lines()
        .filter_map(|line| {
            let mut pieces = line
                .split(|c| " \r\n\t".contains(c))
                .filter(|s| !s.is_empty());
            let url = pieces.next()?;
            let filename = pieces.next()?;
            Some((url, filename))
        })
        .fuse();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let files_seq = files_seq;
            let (dl, mut notify) =
                new_downloader(files_seq, Path::new(&dest_dir), threads_num, speed_limit);
            let notifier = tokio::spawn(async move {
                while let Some((i, src, dst, status)) = notify.next().await {
                    match status {
                        Progress::Started => {
                            println!("#{} {} -> {}: Download started", i, src, dst)
                        }
                        Progress::Finished(Ok(_)) => {
                            println!("#{} {} -> {}: Download finished", i, src, dst)
                        }
                        Progress::Finished(Err(err)) => {
                            eprintln!("#{} {} -> {}: Download failed due to {}", i, src, dst, err)
                        }
                    }
                }
            });

            dl.await;
            let _ = notifier.await;
        });

    Ok(())
}

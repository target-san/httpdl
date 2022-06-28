//
// Uses from stdlib
//
use std::path::Path;
use std::sync::{Mutex, Arc};
//
// Uses from external crates
//
use anyhow::Result;
use clap::Parser;
use tokio_util::io::StreamReader;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::spawn;
use reqwest::Client;
use futures::TryStreamExt as _;
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
#[tokio::main]
async fn main() -> Result<()> {
    // First, parse arguments
    let Config { dest_dir, list_file, threads_num, speed_limit } = Config::try_parse()?;
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        // Open file with list of files to download
        let mut fd = fs::File::open(&list_file).await?;
        // Then read all of its contents into buffer
        let mut text = String::new();
        fd.read_to_string(&mut text).await?;
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

    let bucket = Arc::new(Mutex::new(TokenBucket::new(speed_limit)));
    let dest_dir = Path::new(&dest_dir);
    let client = Client::new();

    for (i, (url_str, dest_file_str)) in files_seq.enumerate() {
        let src_url = url::Url::parse(url_str)?;
        let dest_path = dest_dir.join(dest_file_str);
        let client = client.clone();
        let i = i;  // Dupe loop counter into local scope
        let bucket = bucket.clone();
        let limiter = move |amount| {
            bucket
                .try_lock()
                .ok()
                .map(|mut inner| inner.take(amount))
                .unwrap_or(0)
        };

        spawn(async move {
            println!(
                "#{}: Started {} -> {}",
                i,
                src_url,
                dest_path.file_name().and_then(|name| name.to_str()).unwrap_or("")
            );
            match download_file(client, src_url.clone(), &dest_path, &limiter).await {
                Ok(_) => println!(
                    "#{}: Completed {} -> {}",
                    i,
                    src_url,
                    dest_path.file_name().and_then(|name| name.to_str()).unwrap_or("")
                ),
                Err(err) => eprintln!(
                    "#{}: Failed {} -> {}: {}",
                    i,
                    src_url,
                    dest_path.file_name().and_then(|name| name.to_str()).unwrap_or(""),
                    err
                ),
            }
        });
    }

    Ok(())
}

async fn download_file(
    client:     Client,
    src_url:    impl reqwest::IntoUrl,
    dest_path:  impl AsRef<Path>,
    limiter:   &impl Fn(usize) -> usize
) -> Result<()> { 
    // HTTP client makes request, response body is converted into AsyncRead object
    let src_body = client.get(src_url).send().await?.bytes_stream();
    let mut src_body = StreamReader::new(
        src_body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    );
    // Create destination file and obtain buffered writer around it
    let dest_file = fs::File::create(dest_path).await?;
    let mut dest_file = BufWriter::new(dest_file);
    // Perform actual copying via async version of copy_with_speedlimit
    copy_with_speedlimit(&mut src_body, &mut dest_file, &limiter).await?;
    // Must flush tokio::io::BufWriter manually.
    // It will *not* flush itself automatically when dropped.
    // This note was obtained from one of hyper's issue threads
    dest_file.flush().await?;

    Ok(())
}

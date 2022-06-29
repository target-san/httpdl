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
use reqwest::Client;
use futures::{StreamExt as _, TryStreamExt as _};
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
            let url = pieces.next()?;
            let filename = pieces.next()?;
            Some((url, filename))
        })
        .fuse();

    download_files(files_seq, Path::new(&dest_dir), threads_num, speed_limit).await;

    Ok(())
}

async fn download_files<'a>(
    files:          impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    dest_dir:       impl AsRef<Path>,
    threads_num:    usize,
    speed_limit:    usize
) {
    let bucket = Arc::new(Mutex::new(TokenBucket::new(speed_limit)));
    let client = Client::new();

    let files = futures::stream::iter(files.into_iter().enumerate());

    files
        .map(|(i, (url_str, name_str))| {
            let url_str = url_str.as_ref();
            let name_str = name_str.as_ref();
            println!("#{}: Started {} -> {}", i, url_str, name_str);
            
            let src_url   = url_str.to_string();
            let dest_path = dest_dir.as_ref().join(name_str);
            
            let client = client.clone();
            // Construct separate avatar of token bucket for each task
            let bucket = bucket.clone();
            let limiter = move |amount| {
                bucket
                    .try_lock()
                    .ok()
                    .map(|mut inner| inner.take(amount))
                    .unwrap_or(0)
            };
        
            async move {
                let result = download_file(client, &src_url, &dest_path, &limiter).await;

                (i, src_url, dest_path, result)
            }
        })
        .buffer_unordered(threads_num)
        .for_each(|(i, src_url, dest_path, result)| async move {
            let dest_name = dest_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            
            match result {
                Ok(_) =>
                    println!("#{}: Completed {} -> {}", i, src_url, dest_name ),
                Err(err) =>
                    eprintln!("#{}: Failed {} -> {}: {}", i, src_url, dest_name, err),
            }
        })
        .await
    ;
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
    // Obtained from: https://github.com/seanmonstar/reqwest/issues/482#issuecomment-584245674
    dest_file.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::copy_with_speedlimit::BUFFER_SIZE;
    use rand::{thread_rng, RngCore};
    use warp::Filter;
    use std::io::{BufWriter, Write, Read};
    use std::fs::File;
    use tokio::task::spawn;
    use tokio::runtime::Builder;
    use tokio::sync::oneshot::channel;

    #[test]
    fn successful_downloads() {
        // NB: Yes, I know that testing of private APIs is considered bad practice.
        // ATM download_files is the closest thing to pub api we have.
        // I also have plans to move download procedure into library,
        // so it seems fine in this particular situation
        let src_dir = tempfile::tempdir().unwrap();
        // List of sample file sizes, also used as their names
        let sample_files = [
            0,
            10,
            BUFFER_SIZE - 1,
            BUFFER_SIZE,
            BUFFER_SIZE + 1,
            BUFFER_SIZE * 2,
            BUFFER_SIZE * 256
        ];
        // Generate sample files in source directory
        (&sample_files)
            .map(|size| {
                // Generate 
                let mut buf = vec![0u8; size];
                thread_rng().fill_bytes(&mut buf);

                let file = File::create(src_dir.path().join(size.to_string())).unwrap();
                let mut file = BufWriter::new(file);

                file.write_all(&buf).unwrap();
                file.flush().unwrap();
            })
        ;
        // Generate parameters for files download
        let src_path = src_dir.path().to_owned();
        let dest_dir = tempfile::tempdir().unwrap();
        let dl_names = (&sample_files).map(|size| size.to_string());
        // Perform async download, with local stub server running
        Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                // Routes for all files in source test directory
                let routes = warp::path("files").and(warp::fs::dir(src_path.clone()));
                // Construct shutdown channel
                let (tx, rx) = channel();
                // Construct server future
                let (addr, server) = warp::serve(routes)
                    .bind_with_graceful_shutdown(
                        ([127, 0, 0, 1], 0),
                        async {
                            rx.await.ok();
                        }
                    );
                // Spawn the server into a runtime
                let jh = spawn(server);
                // Download files in question
                let files = dl_names
                    .map(|name| (
                        format!("http://127.0.0.1:{}/files/{}", addr.port(), name),
                        name)
                    );
                // Simple single-threaded unbounded download
                super::download_files(files.iter().map(|(url, name)| (url, name)), &dest_dir, 1, 0).await;
                // Validate files in dest_dir against same files in src_dir
                for (_, name) in &files {
                    let mut src_data = Vec::new();
                    File::open(src_path.join(name)).unwrap().read_to_end(&mut src_data).unwrap();
                    
                    let mut dest_data = Vec::new();
                    File::open(dest_dir.path().join(name)).unwrap().read_to_end(&mut dest_data).unwrap();

                    assert_eq!(src_data, dest_data);
                }
                // At the end, send shutdown signal and wait for termination
                let _ = tx.send(());
                let _ = jh.await;
            });
    }
}

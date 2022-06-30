//
// Uses from stdlib
//
use std::io::Read;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
//
// Uses from external crates
//
use anyhow::Result;
use clap::Parser;
use futures::channel::mpsc;
use futures::{Stream, StreamExt, TryStreamExt, Sink, SinkExt};
use reqwest::Client;
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::io::StreamReader;
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
            let (dl, mut notify) = new_downloader(files_seq, Path::new(&dest_dir), threads_num, speed_limit);
            let notifier = tokio::spawn(async move {
                while let Some((i, src, dst, status)) = notify.next().await {
                    match status {
                        Progress::Started =>
                            println!("#{} {} -> {}: Download started", i, src, dst),
                        Progress::Finished(Ok(_)) =>
                            println!("#{} {} -> {}: Download finished", i, src, dst),
                        Progress::Finished(Err(err)) =>
                            eprintln!("#{} {} -> {}: Download failed due to {}", i, src, dst, err),
                    }
                }
            });
            
            dl.await;
            let _ = notifier.await;
        });

    Ok(())
}

/// Status of specific download job
pub enum Progress {
    /// Job has started
    Started,
    /// Job either finished successfully or failed
    Finished(Result<()>),
}

/// Notifier stream
/// 
/// Unlike underlying UnboundedReceiver, closes itself explicitly upon drop,
/// thus preventing progress messages being sent if not needed
pub struct Notifier<T>(mpsc::UnboundedReceiver<T>);

impl<T> Notifier<T> {
    fn new(recv: mpsc::UnboundedReceiver<T>) -> Notifier<T> { Notifier(recv) }
}

impl<T> Drop for Notifier<T> {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl<T> Stream for Notifier<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
/// Creates new asynchronous file downloader, along with progress notification stream
/// 
/// # Arguments
/// * files - sequence of pairs of source URL and destination file name
/// * dest_dir - destination directory, where to put downloaded files
/// * thread_num - number of concurrent downloads
/// * speed_limit - max download speed, in bytes per second
/// 
/// # Returns
/// Returns pair of values
/// * first element is downloader's future;
///     it completes when all downloads are finished, one or another way
/// * second element is a notification stream which reports states of download jobs;
///     please note that in order to receive notifications in time, client code should
///     spawn separate future which will pull data from stream
/// 
/// Downloader future starts multiple child futures, one future per downloaded file,
/// and up to 'threads_num' futures at once. Files are downloaded into specified directory.
/// Process isn't terminated if some file fails, instead failure is reported through
/// notifier channel.
pub fn new_downloader(
    files: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    dest_dir: impl AsRef<Path>,
    threads_num: usize,
    speed_limit: usize,
) -> (
    impl Future<Output = ()>,
    Notifier<(usize, String, String, Progress)>
) {
    let (send, recv) = mpsc::unbounded();

    let dl_future = async move {
        download_files(files, dest_dir, threads_num, speed_limit, send).await
    };

    (dl_future, Notifier::new(recv))
}

async fn download_files(
    files:       impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    dest_dir:    impl AsRef<Path>,
    threads_num: usize,
    speed_limit: usize,
    notifier:    impl Sink<(usize, String, String, Progress)> + Clone + Send + Unpin
) {
    let bucket = Arc::new(Mutex::new(TokenBucket::new(speed_limit)));
    let client = Client::new();

    let files = futures::stream::iter(files.into_iter().enumerate());

    files
        // Combination of map, buffer_unordered and for_each
        // Produces futures, one per source stream item,
        // and executes up to specified number concurrently
        .for_each_concurrent(threads_num, |(i, (url_str, name_str))| {
            // Notifier function, feeds status notification into sink
            // and produces future to wait for send to complete, if needed
            let mut notifier = notifier.clone();
            let src = url_str.as_ref().to_owned();
            let dst = name_str.as_ref().to_owned();

            let get_limit = {
                let bucket = bucket.clone();
                move |amount| {
                    bucket
                        .try_lock()
                        .ok()
                        .map(|mut inner| inner.take(amount))
                        .unwrap_or(0)
                }
            };

            let src_url = url_str.as_ref().to_owned();
            let dest_path = dest_dir.as_ref().join(name_str.as_ref());
            let client = client.clone();
            
            async move {
                let _ = notifier.feed((i, src.clone(), dst.clone(), Progress::Started)).await;
                let result = download_file(client, &src_url, &dest_path, &get_limit).await;
                let _ = notifier.feed((i, src.clone(), dst.clone(), Progress::Finished(result))).await;
            }
        })
        // Finally, consume whole stream by awaiting on for_each future
        .await;
}

async fn download_file(
    client: Client,
    src_url: impl reqwest::IntoUrl,
    dest_path: impl AsRef<Path>,
    limiter: &impl Fn(usize) -> usize,
) -> Result<()> {
    // HTTP client makes request, response body is converted into AsyncRead object
    let src_body = client.get(src_url).send().await?.bytes_stream();
    let mut src_body =
        StreamReader::new(src_body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
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
    use std::fs::File;
    use std::io::{BufWriter, Read, Write};
    use tokio::runtime::Builder;
    use tokio::sync::oneshot::channel;
    use tokio::task::spawn;
    use warp::Filter;

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
            BUFFER_SIZE * 256,
        ];
        // Generate sample files in source directory
        (&sample_files).map(|size| {
            // Generate
            let mut buf = vec![0u8; size];
            thread_rng().fill_bytes(&mut buf);

            let file = File::create(src_dir.path().join(size.to_string())).unwrap();
            let mut file = BufWriter::new(file);

            file.write_all(&buf).unwrap();
            file.flush().unwrap();
        });
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
                let (addr, server) =
                    warp::serve(routes).bind_with_graceful_shutdown(([127, 0, 0, 1], 0), async {
                        rx.await.ok();
                    });
                // Spawn the server into a runtime
                let jh = spawn(server);
                // Download files in question
                let files = dl_names.map(|name| {
                    (
                        format!("http://127.0.0.1:{}/files/{}", addr.port(), name),
                        name,
                    )
                });
                // Simple single-threaded unbounded download
                let (dl, _) = super::new_downloader(files.iter().map(|(url, name)| (url, name)), &dest_dir, 1, 0);
                dl.await;
                // Validate files in dest_dir against same files in src_dir
                for (_, name) in &files {
                    let mut src_data = Vec::new();
                    File::open(src_path.join(name))
                        .unwrap()
                        .read_to_end(&mut src_data)
                        .unwrap();

                    let mut dest_data = Vec::new();
                    File::open(dest_dir.path().join(name))
                        .unwrap()
                        .read_to_end(&mut dest_data)
                        .unwrap();

                    assert_eq!(src_data, dest_data);
                }
                // At the end, send shutdown signal and wait for termination
                let _ = tx.send(());
                let _ = jh.await;
            });
    }
}

//
// Uses from stdlib
//
use std::path::Path;
//
// Uses from external crates
//
use anyhow::Result;
use clap::Parser;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::spawn;
use reqwest::Client;
//
// Submodules
//
mod token_bucket;

mod config;
use config::Config;

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

    let dest_dir = Path::new(&dest_dir);
    let client = Client::new();

    for (i, (url_str, dest_file_str)) in files_seq.enumerate() {
        let src_url = url::Url::parse(url_str)?;
        let dest_path = dest_dir.join(dest_file_str);
        let client = client.clone();
        let i = i;  // Dupe loop counter into local scope

        spawn(async move {
            println!(
                "#{}: Started {} -> {}",
                i,
                src_url,
                dest_path.file_name().and_then(|name| name.to_str()).unwrap_or("")
            );
            match download_and_write(client, src_url.clone(), &dest_path).await {
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

async fn download_and_write(
    client: Client,
    src_url: impl reqwest::IntoUrl,
    dest_path: impl AsRef<Path>
) -> Result<()> { 
    let mut src_body = client.get(src_url).send().await?;
    let dest_file = fs::File::create(dest_path).await?;
    let mut dest_file = BufWriter::new(dest_file);

    // Do an asynchronous, buffered copy of the download to the output file
    while let Some(chunk) = src_body.chunk().await? {
        dest_file.write(&chunk).await?;
    }
    
    // Must flush tokio::io::BufWriter manually.
    // It will *not* flush itself automatically when dropped.
    dest_file.flush().await?;

    Ok(())
}

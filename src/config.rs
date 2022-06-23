use std::fs;
use std::str::FromStr;

use anyhow::{bail, Result};

use clap::Parser;

// Pre-parsed arguments received from command line
#[derive(Parser)]
#[clap(author, version, about)]
pub struct Config
{
    #[clap(short = 'o', value_parser = parse_dest_dir)]
    /// Destination directory where to store downloaded files
    pub dest_dir:    String,
    #[clap(short = 'f', value_parser = parse_list_file_path)]
    /// File which contains list of URLs to download and local names for files
    pub list_file:   String,
    #[clap(short = 'n', value_parser = 1.., default_value_t = 1)]
    /// Number of worker threads to use
    pub threads_num: usize,
    #[clap(short = 'l', value_parser = parse_speed_limit, default_value_t = 0, verbatim_doc_comment)]
    /// Global speed limit, in bytes per second. 0 means no limit
    ///
    /// Suffixes supported:
    ///     k, K - kilobytes, i.e. 1024's of bytes
    ///     m, M - megabytes, i.e. 1024*1024's of bytes
    pub speed_limit: usize,
}

fn parse_dest_dir(arg: &str) -> Result<String> {
    if fs::metadata(arg)?.is_dir() {
        Ok(arg.to_owned())
    }
    else {
        bail!("{}: not a directory", arg)
    }
}

fn parse_list_file_path(arg: &str) -> Result<String> {
    if fs::metadata(arg)?.is_file() {
        Ok(arg.to_owned())
    }
    else {
        bail!("{}: not a file", arg)
    }
}

fn parse_speed_limit(arg: &str) -> Result<usize> {
    match arg.char_indices().last() {
        None => Ok(0),
        Some((last_index, last_char)) => {
            // Set multiplier based on speed limit suffix
            let mult: usize = match last_char {
                'k' | 'K' => 1024,
                'm' | 'M' => 1024 * 1024,
                _ => 1
            };
            // Next, get actual number string based on multiplier being recognized or not
            let num_str = if mult == 1 { arg } else { arg.split_at(last_index).0 };
            // We could map error, but it's also possible to use '?'
            // and simply return result wrapped into Ok 
            Ok(usize::from_str(num_str).map(|n| n * mult)?)
        }
    }
}

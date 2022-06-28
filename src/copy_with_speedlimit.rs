use std::io::{Result, ErrorKind};

use tokio::task::yield_now;
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
/// Performs asynchronous copying from one byte stream into another, with respect to specified speed limiter
/// 
/// # Arguments
/// * reader  - source asynchronous reader
/// * writer  - destination asynchronous writer
/// * limiter - speed limiter func, specifies how many bytes
///             can be read and then written on each iteration of copying
/// 
/// Reads data from reader and writes into writer in a loop,
/// until reader returns 0, or any error occurs.
/// On each iteration, limiter func is supplied with buffer size,
/// then minimum of buffer size and its return value is used
/// as actual buffer size, then copy operation is performed on that buffer slice
pub async fn copy_with_speedlimit<R, W, L>(
    reader:  &mut R,
    writer:  &mut W,
    limiter: &L
) -> Result<u64>
    where
        R: AsyncRead + Unpin + ?Sized,
        W: AsyncWrite + Unpin + ?Sized,
        L: Fn(usize) -> usize
{
    let mut buf = [0u8; 8 * 1_024];
    let mut written = 0u64;
    loop {
        let limit = limiter(buf.len()).min(buf.len());
        if limit == 0 {
            yield_now().await;
            continue;
        }
        let part = &mut buf[..limit];
        let len = match reader.read(part).await {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => Err(e)?,
        };
        writer.write_all(&mut part[..len]).await?;
        written += len as u64;
    }
}

#[cfg(test)]
mod tests {
    //! TODO: consider fuzzing Read and Write to write some failing tests

    use assert_matches::assert_matches;
    use rand::{thread_rng, Rng, RngCore};
    use super::copy_with_speedlimit;
    use tokio_test::block_on;

    fn unlimited(amount: usize) -> usize{
        amount
    }
    
    fn simple_limit_16(amount: usize) -> usize {
        amount.min(16)
    }

    fn random_limit(amount: usize) -> usize {
        thread_rng().gen_range(0..=amount)
    }
}

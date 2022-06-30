use std::io::{ErrorKind, Result};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::task::yield_now;
/// Size of buffer in bytes, used by asynchronous copy
/// Public to whole crate because of use in tests for main download function
pub(crate) const BUFFER_SIZE: usize = 8 * 1_024;
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
    reader: &mut R,
    writer: &mut W,
    limiter: &L,
) -> Result<u64>
where
    R: AsyncRead + Unpin + ?Sized,
    W: AsyncWrite + Unpin + ?Sized,
    L: Fn(usize) -> usize,
{
    let mut buf = [0u8; BUFFER_SIZE];
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
        writer.write_all(&part[..len]).await?;
        written += len as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::copy_with_speedlimit;
    use super::BUFFER_SIZE;
    use assert_matches::assert_matches;
    use rand::{thread_rng, Rng, RngCore};
    use tokio_test::{block_on, io};

    fn unlimited(amount: usize) -> usize {
        amount
    }

    fn simple_limit_16(amount: usize) -> usize {
        amount.min(16)
    }

    fn random_limit(amount: usize) -> usize {
        thread_rng().gen_range(0..=amount)
    }

    #[test]
    fn successful_copies() {
        // Limiter functions
        let limiters = [unlimited, simple_limit_16, random_limit];
        // Sample buffers
        let samples: Vec<_> = [
            0,
            10,
            BUFFER_SIZE - 1,
            BUFFER_SIZE,
            BUFFER_SIZE + 1,
            BUFFER_SIZE * 2,
        ]
        .map(|size| {
            let mut buf = vec![0u8; size];
            thread_rng().fill_bytes(&mut buf);
            buf
        })
        .into();

        block_on(async move {
            for limiter in limiters {
                for sample in &samples {
                    let mut reader = io::Builder::new().read(sample).build();
                    let mut writer = io::Builder::new().write(sample).build();

                    assert_matches!(
                        copy_with_speedlimit(&mut reader, &mut writer, &limiter).await,
                        Ok(len) if len == sample.len() as u64
                    );
                }
            }
        });
    }
}

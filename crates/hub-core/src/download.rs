use crate::{Checksum, ChecksumAlgorithm, sources::validate_url};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use sha2::Digest;
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, watch},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadControl {
    Running,
    Paused,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Downloading,
    Paused,
    Verifying,
    Extracting,
    Complete,
}
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub stage: DownloadStage,
}
pub(crate) fn progress(
    tx: &Option<mpsc::UnboundedSender<DownloadProgress>>,
    downloaded: u64,
    total: u64,
    stage: DownloadStage,
) {
    if let Some(tx) = tx {
        let _ = tx.send(DownloadProgress {
            downloaded,
            total,
            stage,
        });
    }
}
async fn control_state(
    control: &mut watch::Receiver<DownloadControl>,
    tx: &Option<mpsc::UnboundedSender<DownloadProgress>>,
    current: u64,
    size: u64,
) -> Result<()> {
    loop {
        let state = *control.borrow_and_update();
        match state {
            DownloadControl::Running => return Ok(()),
            DownloadControl::Cancelled => {
                bail!("Download cancelled; partial archive retained for resume")
            }
            DownloadControl::Paused => {
                progress(tx, current, size, DownloadStage::Paused);
                ensure!(
                    control.changed().await.is_ok(),
                    "Paused download controller closed"
                );
            }
        }
    }
}
pub fn verify_file(path: &Path, checksum: &Checksum, size: u64) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() == size,
        "Archive size differs from catalog"
    );
    let mut buffer = vec![0u8; 1024 * 1024];
    let digest = match checksum.algorithm {
        ChecksumAlgorithm::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
        ChecksumAlgorithm::Sha1 => {
            let mut hasher = sha1::Sha1::new();
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
    };
    ensure!(
        digest.eq_ignore_ascii_case(&checksum.value),
        "Archive checksum mismatch: expected {}, received {digest}",
        checksum.value
    );
    Ok(())
}
/// Aborting this future or cancelling preserves the `.part` file. Repeating the same
/// request resumes it; an ignored Range restarts safely and a digest mismatch deletes it.
pub async fn download_verified(
    client: &Client,
    url: &str,
    destination: &Path,
    checksum: &Checksum,
    size: u64,
    tx: Option<mpsc::UnboundedSender<DownloadProgress>>,
    mut control: watch::Receiver<DownloadControl>,
) -> Result<PathBuf> {
    ensure!(size > 0, "Download size must be positive");
    let url = validate_url(url)?;
    let parent = destination
        .parent()
        .context("Download destination has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let lock_path = destination.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock).context("This download is already running")?;
    if destination.exists() {
        let dest = destination.to_path_buf();
        let digest = checksum.clone();
        if tokio::task::spawn_blocking(move || verify_file(&dest, &digest, size))
            .await?
            .is_ok()
        {
            return Ok(destination.into());
        }
        tokio::fs::remove_file(destination).await?;
    }
    let partial = destination.with_extension("part");
    let existing = tokio::fs::metadata(&partial)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let available = fs2::available_space(parent)?;
    ensure!(
        available
            > size
                .saturating_sub(existing)
                .saturating_add(64 * 1024 * 1024),
        "Not enough disk space for download"
    );
    let mut last_error = None;
    for attempt in 0..3 {
        control_state(&mut control, &tx, 0, size).await?;
        let result = transfer(client, &url, &partial, size, &tx, &mut control).await;
        match result {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(error) => {
                if *control.borrow() == DownloadControl::Cancelled {
                    return Err(error);
                }
                last_error = Some(error);
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
            }
        }
    }
    if let Some(error) = last_error {
        return Err(
            error.context("Download failed after three attempts; partial data can be resumed")
        );
    }
    control_state(&mut control, &tx, size, size).await?;
    progress(&tx, size, size, DownloadStage::Verifying);
    let file = partial.clone();
    let digest = checksum.clone();
    let verification =
        tokio::task::spawn_blocking(move || verify_file(&file, &digest, size)).await?;
    if let Err(error) = verification {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(error);
    }
    tokio::fs::rename(&partial, destination).await?;
    Ok(destination.into())
}
async fn transfer(
    client: &Client,
    url: &reqwest::Url,
    partial: &Path,
    size: u64,
    tx: &Option<mpsc::UnboundedSender<DownloadProgress>>,
    control: &mut watch::Receiver<DownloadControl>,
) -> Result<()> {
    let mut offset = tokio::fs::metadata(partial)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if offset > size {
        tokio::fs::remove_file(partial).await?;
        offset = 0;
    }
    if offset == size {
        return Ok(());
    }
    let mut request = client
        .get(url.clone())
        .header(header::ACCEPT_ENCODING, "identity");
    if offset > 0 {
        request = request.header(header::RANGE, format!("bytes={offset}-"));
    }
    let response = request.send().await?.error_for_status()?;
    validate_url(response.url().as_str())?;
    if response.status() == StatusCode::PARTIAL_CONTENT {
        let range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .context("Range response missing Content-Range")?;
        let (start, end, total) = parse_range(range).context("Invalid Content-Range")?;
        ensure!(
            start == offset && total == size && end == size - 1,
            "Server returned an unexpected byte range"
        );
    } else {
        ensure!(
            response.status() == StatusCode::OK,
            "Unexpected download HTTP status {}",
            response.status()
        );
        offset = 0;
    }
    if let Some(length) = response.content_length() {
        ensure!(
            length == size - offset,
            "Server archive length differs from catalog"
        );
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(offset == 0)
        .append(offset > 0)
        .open(partial)
        .await?;
    let mut stream = response.bytes_stream();
    let mut current = offset;
    let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);
    progress(tx, current, size, DownloadStage::Downloading);
    loop {
        control_state(control, tx, current, size).await?;
        let next = tokio::select! {
            value = stream.next() => value,
            changed = control.changed() => { if changed.is_err() { /* sender may be intentionally dropped; keep last state */ } else { continue; }
                stream.next().await
            },
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk?;
        ensure!(
            current.saturating_add(chunk.len() as u64) <= size,
            "Archive exceeds catalog size"
        );
        file.write_all(&chunk).await?;
        current += chunk.len() as u64;
        if last_progress.elapsed() > Duration::from_millis(100) {
            progress(tx, current, size, DownloadStage::Downloading);
            last_progress = std::time::Instant::now();
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    ensure!(
        current == size,
        "Download ended before the complete archive arrived"
    );
    Ok(())
}
fn parse_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, size) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, size.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    #[tokio::test]
    async fn ignored_range_restarts_and_validates() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut b = [0; 4096];
            let n = s.read(&mut b).await.unwrap();
            assert!(
                String::from_utf8_lossy(&b[..n])
                    .to_lowercase()
                    .contains("range: bytes=3-")
            );
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef")
                .await
                .unwrap();
        });
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("x.zip");
        std::fs::write(dest.with_extension("part"), b"abc").unwrap();
        let c = Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: format!("{:x}", sha2::Sha256::digest(b"abcdef")),
        };
        let (_tx, control) = watch::channel(DownloadControl::Running);
        download_verified(
            &Client::new(),
            &format!("http://{addr}/x"),
            &dest,
            &c,
            6,
            None,
            control,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"abcdef");
    }
    #[tokio::test]
    async fn checksum_failure_removes_partial() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("x.zip");
        std::fs::write(dest.with_extension("part"), b"abcdef").unwrap();
        let c = Checksum {
            algorithm: ChecksumAlgorithm::Sha256,
            value: "0".repeat(64),
        };
        let (_tx, control) = watch::channel(DownloadControl::Running);
        assert!(
            download_verified(
                &Client::new(),
                "https://example.com/x",
                &dest,
                &c,
                6,
                None,
                control
            )
            .await
            .is_err()
        );
        assert!(!dest.with_extension("part").exists());
        assert!(!dest.exists());
    }
    #[test]
    fn range_syntax() {
        assert_eq!(parse_range("bytes 3-5/6"), Some((3, 5, 6)));
        assert!(parse_range("bytes */6").is_none());
    }
    #[tokio::test]
    async fn resumes_partial_with_valid_content_range() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let n = socket.read(&mut request).await.unwrap();
            assert!(
                String::from_utf8_lossy(&request[..n])
                    .to_ascii_lowercase()
                    .contains("range: bytes=3-")
            );
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef").await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("resume.zip");
        std::fs::write(destination.with_extension("part"), b"abc").unwrap();
        let checksum = Checksum {
            algorithm: ChecksumAlgorithm::Sha1,
            value: format!("{:x}", sha1::Sha1::digest(b"abcdef")),
        };
        let (_tx, control) = watch::channel(DownloadControl::Running);
        download_verified(
            &Client::new(),
            &format!("http://{address}/x"),
            &destination,
            &checksum,
            6,
            None,
            control,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), b"abcdef");
    }
}

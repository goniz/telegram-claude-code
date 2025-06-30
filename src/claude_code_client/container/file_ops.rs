//! File operations module for Docker containers
//!
//! This module provides functionality for bidirectional file operations
//! with Docker containers using tar archives for efficient file transfers.

use bollard::query_parameters::{DownloadFromContainerOptions, UploadToContainerOptions};
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{Either, Full};
use std::io::Read;
use tar::{Archive, Builder};

/// Get a file from a container
///
/// Downloads a file from the specified container using Docker's download API.
/// The file is transferred as a tar archive and extracted to return the file content.
///
/// # Arguments
///
/// * `docker` - Docker client instance
/// * `container_id` - ID of the container to download from
/// * `file_path` - Path to the file inside the container
///
/// # Returns
///
/// Returns the file content as a Vec<u8> on success, or an error if the operation fails.
///
/// # Example
///
/// ```rust,no_run
/// # use bollard::Docker;
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// let docker = Docker::connect_with_local_defaults()?;
/// let content = container_get_file(&docker, "container_id", "/path/to/file.txt").await?;
/// # Ok(())
/// # }
/// ```
pub async fn container_get_file(
    docker: &Docker,
    container_id: &str,
    file_path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let download_options = DownloadFromContainerOptions {
        path: file_path.to_string(),
    };

    let mut stream = docker.download_from_container(container_id, Some(download_options));
    let mut tar_data = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        tar_data.extend_from_slice(&chunk);
    }

    // Extract the file from the tar archive
    let mut archive = Archive::new(tar_data.as_slice());
    if let Some(entry) = archive.entries()?.next() {
        let mut entry = entry?;
        let mut file_content = Vec::new();
        entry.read_to_end(&mut file_content)?;
        return Ok(file_content);
    }

    Err("File not found in tar archive".into())
}

/// Put a file into a container
///
/// Uploads a file to the specified container using Docker's upload API.
/// The file content is packaged into a tar archive before transfer.
///
/// # Arguments
///
/// * `docker` - Docker client instance
/// * `container_id` - ID of the container to upload to
/// * `file_path` - Path where the file should be created inside the container
/// * `file_content` - Content of the file as bytes
/// * `permissions` - Optional file permissions (defaults to 0o644)
///
/// # Returns
///
/// Returns () on success, or an error if the operation fails.
///
/// # Example
///
/// ```rust,no_run
/// # use bollard::Docker;
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// let docker = Docker::connect_with_local_defaults()?;
/// let content = b"Hello, world!";
/// container_put_file(&docker, "container_id", "/path/to/file.txt", content, Some(0o755)).await?;
/// # Ok(())
/// # }
/// ```
pub async fn container_put_file(
    docker: &Docker,
    container_id: &str,
    file_path: &str,
    file_content: &[u8],
    permissions: Option<u32>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create a tar archive containing the file
    let mut tar_data = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_data);

        // Extract the filename from the path
        let filename = std::path::Path::new(file_path)
            .file_name()
            .ok_or("Invalid file path")?
            .to_str()
            .ok_or("Invalid filename encoding")?;

        let mut header = tar::Header::new_gnu();
        header.set_size(file_content.len() as u64);
        header.set_mode(permissions.unwrap_or(0o644));
        header.set_cksum();

        builder.append_data(&mut header, filename, file_content)?;
        builder.finish()?;
    }

    // Get the directory path for upload
    let dir_path = std::path::Path::new(file_path)
        .parent()
        .ok_or("Invalid file path")?
        .to_str()
        .ok_or("Invalid directory path encoding")?;

    let upload_options = UploadToContainerOptions {
        path: dir_path.to_string(),
        ..Default::default()
    };

    // Upload the tar archive to the container
    docker
        .upload_to_container(
            container_id,
            Some(upload_options),
            Either::Left(Full::new(Bytes::from(tar_data))),
        )
        .await?;

    Ok(())
}

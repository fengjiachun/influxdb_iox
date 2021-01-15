#![deny(rust_2018_idioms)]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    clippy::explicit_iter_loop,
    clippy::use_self
)]

//! # object_store
//!
//! This crate provides APIs for interacting with object storage services. It
//! currently supports PUT, GET, DELETE, and list for Google Cloud Storage,
//! Amazon S3, in-memory and local file storage.
//!
//! Future compatibility will include Azure Blob Storage, Minio, and Ceph.

pub mod aws;
pub mod azure;
pub mod disk;
pub mod gcp;
pub mod memory;
pub mod path;

use aws::AmazonS3;
use azure::MicrosoftAzure;
use disk::File;
use gcp::GoogleCloudStorage;
use memory::InMemory;
use path::ObjectStorePath;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::{stream::BoxStream, Stream, StreamExt, TryStreamExt};
use snafu::Snafu;
use std::{io, path::PathBuf, unimplemented};

#[allow(missing_docs)]
#[async_trait]
pub trait ObjSto: Send + Sync + 'static {
    type Path: path::Osp;

    /// Save the provided bytes to the specified location.
    async fn put<S>(&self, location: &Self::Path, bytes: S, length: usize) -> Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Sync + 'static;

    /// Return the bytes that are stored at the specified location.
    async fn get(&self, location: &Self::Path) -> Result<BoxStream<'static, Result<Bytes>>>;

    /// Delete the object at the specified location.
    async fn delete(&self, location: &Self::Path) -> Result<()>;

    /// List all the objects with the given prefix.
    async fn list<'a>(
        &'a self,
        prefix: Option<&'a Self::Path>,
    ) -> Result<BoxStream<'a, Result<Vec<Self::Path>>>>;

    /// List objects with the given prefix and an implementation specific
    /// delimiter. Returns common prefixes (directories) in addition to object
    /// metadata.
    async fn list_with_delimiter(&self, prefix: &Self::Path) -> Result<ListResult<Self::Path>>;
}

#[async_trait]
impl<T> ObjSto for T
where
    T: std::ops::Deref + Send + Sync + 'static,
    T::Target: ObjSto,
{
    type Path = <T::Target as ObjSto>::Path;

    async fn put<S>(&self, location: &Self::Path, bytes: S, length: usize) -> Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Sync + 'static,
    {
        T::put(self, location, bytes, length).await
    }

    async fn get(&self, location: &Self::Path) -> Result<BoxStream<'static, Result<Bytes>>> {
        T::get(self, location).await
    }

    async fn delete(&self, location: &Self::Path) -> Result<()> {
        T::delete(self, location).await
    }

    async fn list<'a>(
        &'a self,
        prefix: Option<&'a Self::Path>,
    ) -> Result<BoxStream<'a, Result<Vec<Self::Path>>>> {
        T::list(self, prefix).await
    }

    async fn list_with_delimiter(&self, prefix: &Self::Path) -> Result<ListResult<Self::Path>> {
        T::list_with_delimiter(self, prefix).await
    }
}

/// Universal interface to multiple object store services.
#[derive(Debug)]
pub struct ObjectStore(pub ObjectStoreIntegration);

impl ObjectStore {
    /// Configure a connection to Amazon S3.
    pub fn new_amazon_s3(s3: AmazonS3) -> Self {
        Self(ObjectStoreIntegration::AmazonS3(s3))
    }

    /// Configure a connection to Google Cloud Storage.
    pub fn new_google_cloud_storage(gcs: GoogleCloudStorage) -> Self {
        Self(ObjectStoreIntegration::GoogleCloudStorage(gcs))
    }

    /// Configure in-memory storage.
    pub fn new_in_memory(in_mem: InMemory) -> Self {
        Self(ObjectStoreIntegration::InMemory(in_mem))
    }

    /// Configure local file storage.
    pub fn new_file(file: File) -> Self {
        Self(ObjectStoreIntegration::File(file))
    }

    /// Configure a connection to Microsoft Azure Blob store.
    pub fn new_microsoft_azure(azure: MicrosoftAzure) -> Self {
        Self(ObjectStoreIntegration::MicrosoftAzure(Box::new(azure)))
    }
}

#[async_trait]
impl ObjSto for ObjectStore {
    type Path = ObjectStorePath;

    async fn put<S>(&self, location: &Self::Path, bytes: S, length: usize) -> Result<()>
    where
        S: Stream<Item = io::Result<Bytes>> + Send + Sync + 'static,
    {
        use ObjectStoreIntegration::*;
        match &self.0 {
            AmazonS3(s3) => s3.put(location, bytes, length).await?,
            GoogleCloudStorage(gcs) => gcs.put(location, bytes, length).await?,
            InMemory(in_mem) => in_mem.put(location, bytes, length).await?,
            File(file) => file.put(location, bytes, length).await?,
            MicrosoftAzure(azure) => azure.put(location, bytes, length).await?,
        }

        Ok(())
    }

    async fn get(&self, location: &Self::Path) -> Result<BoxStream<'static, Result<Bytes>>> {
        use ObjectStoreIntegration::*;
        Ok(match &self.0 {
            AmazonS3(s3) => s3.get(location).await?.err_into().boxed(),
            GoogleCloudStorage(gcs) => gcs.get(location).await?.err_into().boxed(),
            InMemory(in_mem) => in_mem.get(location).await?.err_into().boxed(),
            File(file) => file.get(location).await?.err_into().boxed(),
            MicrosoftAzure(azure) => azure.get(location).await?.err_into().boxed(),
        })
    }

    async fn delete(&self, location: &Self::Path) -> Result<()> {
        use ObjectStoreIntegration::*;
        match &self.0 {
            AmazonS3(s3) => s3.delete(location).await?,
            GoogleCloudStorage(gcs) => gcs.delete(location).await?,
            InMemory(in_mem) => in_mem.delete(location).await?,
            File(file) => file.delete(location).await?,
            MicrosoftAzure(azure) => azure.delete(location).await?,
        }

        Ok(())
    }

    async fn list<'a>(
        &'a self,
        prefix: Option<&'a Self::Path>,
    ) -> Result<BoxStream<'a, Result<Vec<Self::Path>>>> {
        use ObjectStoreIntegration::*;
        Ok(match &self.0 {
            AmazonS3(s3) => s3.list(prefix).await?.err_into().boxed(),
            GoogleCloudStorage(gcs) => gcs.list(prefix).await?.err_into().boxed(),
            InMemory(in_mem) => in_mem.list(prefix).await?.err_into().boxed(),
            File(file) => file.list(prefix).await?.err_into().boxed(),
            MicrosoftAzure(azure) => azure.list(prefix).await?.err_into().boxed(),
        })
    }

    async fn list_with_delimiter(&self, prefix: &Self::Path) -> Result<ListResult<Self::Path>> {
        use ObjectStoreIntegration::*;
        match &self.0 {
            AmazonS3(s3) => s3.list_with_delimiter(prefix, &None).await,
            GoogleCloudStorage(_gcs) => unimplemented!(),
            InMemory(in_mem) => in_mem.list_with_delimiter(prefix, &None).await,
            File(_file) => unimplemented!(),
            MicrosoftAzure(_azure) => unimplemented!(),
        }
    }
}

/// All supported object storage integrations
#[derive(Debug)]
pub enum ObjectStoreIntegration {
    /// GCP storage
    GoogleCloudStorage(GoogleCloudStorage),
    /// Amazon storage
    AmazonS3(AmazonS3),
    /// In memory storage for testing
    InMemory(InMemory),
    /// Local file system storage
    File(File),
    /// Microsoft Azure Blob storage
    MicrosoftAzure(Box<MicrosoftAzure>),
}

/// Result of a list call that includes objects, prefixes (directories) and a
/// token for the next set of results. Individual results sets are limited to
/// 1,000 objects.
#[derive(Debug)]
pub struct ListResult<P> {
    /// Token passed to the API for the next page of list results.
    pub next_token: Option<String>,
    /// Prefixes that are common (like directories)
    pub common_prefixes: Vec<P>, // TODO: why is this a vec instead of just the path?
    /// Object metadata for the listing
    pub objects: Vec<ObjectMeta<P>>,
}

/// The metadata that describes an object.
#[derive(Debug)]
pub struct ObjectMeta<P> {
    /// The full path to the object
    pub location: P,
    /// The last modified time
    pub last_modified: DateTime<Utc>,
    /// The size in bytes of the object
    pub size: usize,
}

/// A specialized `Result` for object store-related errors
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A specialized `Error` for object store-related errors
#[derive(Debug, Snafu)]
#[allow(missing_docs)]
pub enum Error {
    DataDoesNotMatchLength {
        expected: usize,
        actual: usize,
    },
    #[snafu(display("Unable to parse last modified time {}: {}", value, err))]
    UnableToParseLastModifiedTime {
        value: String,
        err: chrono::ParseError,
    },

    UnableToPutDataToGcs {
        source: tokio::task::JoinError,
        bucket: String,
        location: String,
    },
    UnableToListDataFromGcs {
        source: tokio::task::JoinError,
        bucket: String,
    },
    UnableToListDataFromGcs2 {
        source: cloud_storage::Error,
        bucket: String,
    },
    UnableToDeleteDataFromGcs {
        source: tokio::task::JoinError,
        bucket: String,
        location: String,
    },
    UnableToDeleteDataFromGcs2 {
        source: cloud_storage::Error,
        bucket: String,
        location: String,
    },
    UnableToGetDataFromGcs {
        source: tokio::task::JoinError,
        bucket: String,
        location: String,
    },
    UnableToGetDataFromGcs2 {
        source: cloud_storage::Error,
        bucket: String,
        location: String,
    },

    UnableToPutDataToS3 {
        source: rusoto_core::RusotoError<rusoto_s3::PutObjectError>,
        bucket: String,
        location: String,
    },
    UnableToGetDataFromS3 {
        source: rusoto_core::RusotoError<rusoto_s3::GetObjectError>,
        bucket: String,
        location: String,
    },
    UnableToDeleteDataFromS3 {
        source: rusoto_core::RusotoError<rusoto_s3::DeleteObjectError>,
        bucket: String,
        location: String,
    },
    NoDataFromS3 {
        bucket: String,
        location: String,
    },
    UnableToReadBytesFromS3 {
        source: std::io::Error,
        bucket: String,
        location: String,
    },
    UnableToGetPieceOfDataFromS3 {
        source: std::io::Error,
        bucket: String,
        location: String,
    },
    UnableToListDataFromS3 {
        source: rusoto_core::RusotoError<rusoto_s3::ListObjectsV2Error>,
        bucket: String,
    },

    UnableToPutDataInMemory {
        source: std::io::Error,
    },
    NoDataInMemory,

    UnableToPutDataToAzure {
        source: azure_sdk_core::errors::AzureError,
        location: String,
    },
    UnableToGetDataFromAzure {
        source: azure_sdk_core::errors::AzureError,
        location: String,
    },
    UnableToDeleteDataFromAzure {
        source: azure_sdk_core::errors::AzureError,
        location: String,
    },
    UnableToListDataFromAzure {
        source: azure_sdk_core::errors::AzureError,
    },

    #[snafu(display("Unable to create file {}: {}", path.display(), err))]
    UnableToCreateFile {
        err: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to create dir {}: {}", path.display(), source))]
    UnableToCreateDir {
        source: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to open file {}: {}", path.display(), source))]
    UnableToOpenFile {
        source: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to read data from file {}: {}", path.display(), source))]
    UnableToReadBytes {
        source: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to delete file {}: {}", path.display(), source))]
    UnableToDeleteFile {
        source: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to list directory {}: {}", path.display(), source))]
    UnableToListDirectory {
        source: io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to process directory entry: {}", source))]
    UnableToProcessEntry {
        source: io::Error,
    },
    #[snafu(display("Unable to copy data to file: {}", source))]
    UnableToCopyDataToFile {
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
    type Result<T, E = Error> = std::result::Result<T, E>;

    async fn flatten_list_stream(
        storage: &ObjectStore,
        prefix: Option<&ObjectStorePath>,
    ) -> Result<Vec<ObjectStorePath>> {
        storage
            .list(prefix)
            .await?
            .map_ok(|v| stream::iter(v).map(Ok))
            .try_flatten()
            .try_collect()
            .await
    }

    pub(crate) async fn put_get_delete_list(storage: &ObjectStore) -> Result<()> {
        delete_fixtures(storage).await;

        let content_list = flatten_list_stream(storage, None).await?;
        assert!(
            content_list.is_empty(),
            "Expected list to be empty; found: {:?}",
            content_list
        );

        let data = Bytes::from("arbitrary data");
        let mut location = ObjectStorePath::default();
        location.push_dir("test_dir");
        location.set_file_name("test_file.json");

        let stream_data = std::io::Result::Ok(data.clone());
        storage
            .put(
                &location,
                futures::stream::once(async move { stream_data }),
                data.len(),
            )
            .await?;

        // List everything
        let content_list = flatten_list_stream(storage, None).await?;
        assert_eq!(content_list, &[location.clone()]);

        // List everything starting with a prefix that should return results
        let mut prefix = ObjectStorePath::default();
        prefix.push_dir("test_dir");
        let content_list = flatten_list_stream(storage, Some(&prefix)).await?;
        assert_eq!(content_list, &[location.clone()]);

        // List everything starting with a prefix that shouldn't return results
        let mut prefix = ObjectStorePath::default();
        prefix.push_dir("something");
        let content_list = flatten_list_stream(storage, Some(&prefix)).await?;
        assert!(content_list.is_empty());

        let read_data = storage
            .get(&location)
            .await?
            .map_ok(|b| bytes::BytesMut::from(&b[..]))
            .try_concat()
            .await?;
        assert_eq!(&*read_data, data);

        storage.delete(&location).await?;

        let content_list = flatten_list_stream(storage, None).await?;
        assert!(content_list.is_empty());

        Ok(())
    }

    pub(crate) async fn list_with_delimiter(storage: &ObjectStore) -> Result<()> {
        delete_fixtures(storage).await;

        let content_list = flatten_list_stream(storage, None).await?;
        assert!(content_list.is_empty());

        let data = Bytes::from("arbitrary data");

        let files: Vec<_> = [
            "mydb/wal/000/000/000.segment",
            "mydb/wal/000/000/001.segment",
            "mydb/wal/000/000/002.segment",
            "mydb/wal/001/001/000.segment",
            "mydb/wal/foo.test",
            "mydb/data/whatevs",
        ]
        .iter()
        .map(|&s| ObjectStorePath::from_cloud_unchecked(s))
        .collect();

        let time_before_creation = Utc::now();

        for f in &files {
            let stream_data = std::io::Result::Ok(data.clone());
            storage
                .put(
                    f,
                    futures::stream::once(async move { stream_data }),
                    data.len(),
                )
                .await
                .unwrap();
        }

        let mut prefix = ObjectStorePath::default();
        prefix.push_all_dirs(&["mydb", "wal"]);

        let mut expected_000 = prefix.clone();
        expected_000.push_dir("000");
        let mut expected_001 = prefix.clone();
        expected_001.push_dir("001");
        let mut expected_location = prefix.clone();
        expected_location.set_file_name("foo.test");

        let result = storage.list_with_delimiter(&prefix).await.unwrap();

        assert_eq!(result.common_prefixes, vec![expected_000, expected_001]);
        assert_eq!(result.objects.len(), 1);

        let object = &result.objects[0];

        assert_eq!(object.location, expected_location);
        assert_eq!(object.size, data.len());
        assert!(object.last_modified > time_before_creation);

        // List with a prefix containing a partial "file name"
        let mut prefix = ObjectStorePath::default();
        prefix.push_all_dirs(&["mydb", "wal", "000", "000"]);
        prefix.set_file_name("001");

        let mut expected_location = ObjectStorePath::default();
        expected_location.push_all_dirs(&["mydb", "wal", "000", "000"]);
        expected_location.set_file_name("001.segment");

        let result = storage.list_with_delimiter(&prefix).await.unwrap();
        assert!(result.common_prefixes.is_empty());
        assert_eq!(result.objects.len(), 1);

        let object = &result.objects[0];

        assert_eq!(object.location, expected_location);

        for f in &files {
            storage.delete(f).await.unwrap();
        }

        let content_list = flatten_list_stream(storage, None).await?;
        assert!(content_list.is_empty());

        Ok(())
    }

    pub(crate) async fn get_nonexistent_object(
        storage: &ObjectStore,
        location: Option<ObjectStorePath>,
    ) -> Result<Bytes> {
        let location = location
            .unwrap_or_else(|| ObjectStorePath::from_cloud_unchecked("this_file_should_not_exist"));

        let content_list = flatten_list_stream(storage, Some(&location)).await?;
        assert!(content_list.is_empty());

        Ok(storage
            .get(&location)
            .await?
            .map_ok(|b| bytes::BytesMut::from(&b[..]))
            .try_concat()
            .await?
            .freeze())
    }

    async fn delete_fixtures(storage: &ObjectStore) {
        let files: Vec<_> = [
            "test_file",
            "mydb/wal/000/000/000.segment",
            "mydb/wal/000/000/001.segment",
            "mydb/wal/000/000/002.segment",
            "mydb/wal/001/001/000.segment",
            "mydb/wal/foo.test",
            "mydb/data/whatevs",
        ]
        .iter()
        .map(|&s| ObjectStorePath::from_cloud_unchecked(s))
        .collect();

        for f in &files {
            // don't care if it errors, should fail elsewhere
            let _ = storage.delete(f).await;
        }
    }

    // Tests TODO:
    // GET nonexisting location (in_memory/file)
    // DELETE nonexisting location
    // PUT overwriting
}

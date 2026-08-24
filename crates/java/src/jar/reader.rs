//! Reusable access to lazy original and owned entry payloads.

use std::io::Cursor;
use std::sync::Arc;

use zip::ZipArchive;

use crate::{Error, Result};

use super::entry::{EntryData, JarEntry};
use super::{JarFile, read_zip_file};

type SourceReader = Cursor<Arc<[u8]>>;

pub(crate) struct EntryReader {
    original: Option<Arc<[u8]>>,
    archive: Option<ZipArchive<SourceReader>>,
    #[cfg(test)]
    archive_constructions: usize,
}

impl EntryReader {
    pub(crate) fn new(jar: &JarFile) -> Self {
        Self {
            original: jar.original.clone(),
            archive: None,
            #[cfg(test)]
            archive_constructions: 0,
        }
    }

    pub(crate) fn read(&mut self, entry: &JarEntry) -> Result<Vec<u8>> {
        match &entry.data {
            EntryData::Owned(bytes) => Ok(bytes.clone()),
            EntryData::Original(index) => {
                let mut file = self.archive()?.by_index(*index)?;
                read_zip_file(&mut file)
            }
        }
    }

    fn archive(&mut self) -> Result<&mut ZipArchive<SourceReader>> {
        if self.archive.is_none() {
            let original = self.original.as_ref().ok_or_else(|| {
                Error::InvalidJar("entry has no original archive backing".to_owned())
            })?;
            self.archive = Some(ZipArchive::new(Cursor::new(Arc::clone(original)))?);
            #[cfg(test)]
            {
                self.archive_constructions += 1;
            }
        }
        Ok(self
            .archive
            .as_mut()
            .expect("archive was initialized immediately above"))
    }

    #[cfg(test)]
    pub(crate) const fn archive_constructions(&self) -> usize {
        self.archive_constructions
    }
}

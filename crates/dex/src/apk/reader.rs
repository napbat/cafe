//! Reusable access to lazy original and owned APK entry payloads.

use std::io::Cursor;
use std::sync::Arc;

use zip::ZipArchive;

use crate::{Error, Result};

use super::entry::{ApkEntry, EntryData};
use super::{ApkFile, read_zip_file};

type SourceReader = Cursor<Arc<[u8]>>;

pub(super) struct EntryReader {
    original: Option<Arc<[u8]>>,
    archive: Option<ZipArchive<SourceReader>>,
    #[cfg(test)]
    archive_constructions: usize,
}

impl EntryReader {
    pub(super) fn new(apk: &ApkFile) -> Self {
        Self {
            original: apk.original.clone(),
            archive: None,
            #[cfg(test)]
            archive_constructions: 0,
        }
    }

    pub(super) fn read(&mut self, entry: &ApkEntry) -> Result<Vec<u8>> {
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
            let original = self
                .original
                .as_ref()
                .ok_or_else(|| Error::invalid_apk("entry has no original archive backing"))?;
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
    pub(super) const fn archive_constructions(&self) -> usize {
        self.archive_constructions
    }
}

#[cfg(test)]
mod tests {
    use super::{ApkFile, EntryReader};
    use crate::Result;

    const BULK_ENTRY_COUNT: usize = 128;

    #[test]
    fn bulk_reads_construct_one_archive_reader() -> Result<()> {
        let mut source = ApkFile::new();
        for index in 0..BULK_ENTRY_COUNT {
            source.add_file(
                format!("assets/{index}.txt"),
                index.to_string().into_bytes(),
            )?;
        }
        let apk = ApkFile::from_bytes(source.to_bytes()?)?;
        let mut reader = EntryReader::new(&apk);

        for entry in &apk.entries {
            assert!(!reader.read(entry)?.is_empty());
        }

        assert_eq!(reader.archive_constructions(), 1);
        Ok(())
    }
}

//! Named ZIP layout values shared by JAR parsing and assembly.

pub(super) const ZIP_U16_FIELD_WIDTH: usize = size_of::<u16>();
pub(super) const ZIP_U16_MAXIMUM: usize = u16::MAX as usize;
pub(super) const ZIP_EXTRA_FIELD_ID_OFFSET: usize = 0;
pub(super) const ZIP_EXTRA_FIELD_LENGTH_OFFSET: usize = ZIP_U16_FIELD_WIDTH;
pub(super) const ZIP_EXTRA_FIELD_HEADER_SIZE: usize = ZIP_U16_FIELD_WIDTH * 2;

pub(super) const PORTABLE_FILE_MODE: u32 = 0o644;
pub(super) const PORTABLE_DIRECTORY_MODE: u32 = 0o755;
pub(super) const PORTABLE_SYMLINK_MODE: u32 = 0o777;

pub(super) const ARCHIVE_SEPARATOR: char = '/';
pub(super) const WINDOWS_SEPARATOR: char = '\\';
pub(super) const NUL_CHARACTER: char = '\0';
pub(super) const CURRENT_DIRECTORY_COMPONENT: &str = ".";
pub(super) const PARENT_DIRECTORY_COMPONENT: &str = "..";

pub(super) const INITIAL_ENTRY_ID: u64 = 0;
pub(super) const ENTRY_ID_INCREMENT: u64 = 1;

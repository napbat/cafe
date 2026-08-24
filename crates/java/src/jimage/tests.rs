use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write;

use crate::classfile::{ClassAccessFlags, ClassFile, JAVA_8_MAJOR_VERSION};

use super::*;

const ATTRIBUTE_MODULE: u8 = 1;
const ATTRIBUTE_PARENT: u8 = 2;
const ATTRIBUTE_BASE: u8 = 3;
const ATTRIBUTE_EXTENSION: u8 = 4;
const ATTRIBUTE_OFFSET: u8 = 5;
const ATTRIBUTE_COMPRESSED: u8 = 6;
const ATTRIBUTE_UNCOMPRESSED: u8 = 7;

#[test]
fn inventories_and_reads_class_resources() -> Result<()> {
    let class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/Thing",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC,
    )?;
    let image = synthetic_image(&class.to_bytes()?, false)?;
    let parsed = JimageFile::from_bytes(image.clone())?;
    assert_eq!(parsed.original_bytes(), image);
    assert_eq!(parsed.entries()[0].name, "/java.base/sample/Thing.class");
    assert_eq!(
        parsed
            .read_class("java.base", "sample.Thing")?
            .class_name()?,
        "sample/Thing"
    );
    Ok(())
}

#[test]
fn decompresses_zip_resource_layers() -> Result<()> {
    let class = ClassFile::new(
        JAVA_8_MAJOR_VERSION,
        "sample/Thing",
        Some("java/lang/Object"),
        ClassAccessFlags::PUBLIC,
    )?;
    let parsed = JimageFile::from_bytes(synthetic_image(&class.to_bytes()?, true)?)?;
    assert_eq!(
        parsed
            .read_class("java.base", "sample/Thing")?
            .class_name()?,
        "sample/Thing"
    );
    Ok(())
}

#[test]
fn rejects_out_of_bounds_resource() -> Result<()> {
    let mut image = synthetic_image(&[1, 2, 3], false)?;
    image.pop();
    assert!(matches!(
        JimageFile::from_bytes(image),
        Err(Error::InvalidJimage { .. })
    ));
    Ok(())
}

fn synthetic_image(resource: &[u8], compressed: bool) -> Result<Vec<u8>> {
    let mut strings = vec![0];
    let module = add_string(&mut strings, "java.base");
    let parent = add_string(&mut strings, "sample");
    let base = add_string(&mut strings, "Thing");
    let extension = add_string(&mut strings, "class");
    let zip = add_string(&mut strings, "zip");

    let stored = if compressed {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(resource)?;
        let payload = encoder.finish()?;
        let mut layer = Vec::new();
        layer.extend_from_slice(&COMPRESSED_RESOURCE_MAGIC.to_le_bytes());
        layer.extend_from_slice(&u64::try_from(payload.len()).unwrap().to_le_bytes());
        layer.extend_from_slice(&u64::try_from(resource.len()).unwrap().to_le_bytes());
        layer.extend_from_slice(&zip.to_le_bytes());
        layer.extend_from_slice(&u32::MAX.to_le_bytes());
        layer.push(1);
        layer.extend_from_slice(&payload);
        layer
    } else {
        resource.to_owned()
    };

    let mut locations = vec![0];
    push_attribute(&mut locations, ATTRIBUTE_MODULE, u64::from(module));
    push_attribute(&mut locations, ATTRIBUTE_PARENT, u64::from(parent));
    push_attribute(&mut locations, ATTRIBUTE_BASE, u64::from(base));
    push_attribute(&mut locations, ATTRIBUTE_EXTENSION, u64::from(extension));
    push_attribute(&mut locations, ATTRIBUTE_OFFSET, 0);
    if compressed {
        push_attribute(
            &mut locations,
            ATTRIBUTE_COMPRESSED,
            u64::try_from(stored.len()).unwrap(),
        );
    }
    push_attribute(
        &mut locations,
        ATTRIBUTE_UNCOMPRESSED,
        u64::try_from(resource.len()).unwrap(),
    );
    locations.push(0);

    let table_length = 1_u32;
    let mut image = Vec::new();
    image.extend_from_slice(&JIMAGE_MAGIC.to_le_bytes());
    image.extend_from_slice(
        &((u32::from(JIMAGE_MAJOR_VERSION) << 16) | u32::from(JIMAGE_MINOR_VERSION)).to_le_bytes(),
    );
    image.extend_from_slice(&0_u32.to_le_bytes());
    image.extend_from_slice(&1_u32.to_le_bytes());
    image.extend_from_slice(&table_length.to_le_bytes());
    image.extend_from_slice(&u32::try_from(locations.len()).unwrap().to_le_bytes());
    image.extend_from_slice(&u32::try_from(strings.len()).unwrap().to_le_bytes());
    image.extend_from_slice(&0_u32.to_le_bytes());
    image.extend_from_slice(&1_u32.to_le_bytes());
    image.extend_from_slice(&locations);
    image.extend_from_slice(&strings);
    image.extend_from_slice(&stored);
    Ok(image)
}

fn add_string(strings: &mut Vec<u8>, value: &str) -> u32 {
    let offset = u32::try_from(strings.len()).unwrap();
    strings.extend_from_slice(value.as_bytes());
    strings.push(0);
    offset
}

fn push_attribute(output: &mut Vec<u8>, kind: u8, value: u64) {
    if value == 0 {
        return;
    }
    let width = usize::try_from((u64::BITS - value.leading_zeros()).div_ceil(u8::BITS)).unwrap();
    output.push((kind << 3) | u8::try_from(width - 1).unwrap());
    for shift in (0..width).rev() {
        output.push(u8::try_from((value >> (shift * 8)) & 0xff).unwrap());
    }
}

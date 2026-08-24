//! Prints a compact inventory summary for a JDK `lib/modules` image.

use std::collections::BTreeSet;
use std::env;

use java::Result;
use java::jimage::JimageFile;

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .expect("usage: jimage_inventory <lib/modules>");
    let image = JimageFile::open(path)?;
    let modules = image
        .entries()
        .iter()
        .map(|entry| entry.module.as_str())
        .filter(|module| !module.is_empty())
        .collect::<BTreeSet<_>>();
    let classes = image
        .entries()
        .iter()
        .filter(|entry| entry.is_class())
        .count();
    let object = image.read_class("java.base", "java/lang/Object")?;
    println!(
        "{} resources, {classes} classes, {} modules; java/lang/Object major {}",
        image.entries().len(),
        modules.len(),
        object.major_version
    );
    Ok(())
}

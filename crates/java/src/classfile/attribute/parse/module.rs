//! Parser for the Java Platform Module System attribute.

use crate::{Error, Result};

use super::super::super::io::Reader;
use super::super::super::{
    Constant, ConstantPool, ModuleAccessFlags, ModuleExportsFlags, ModuleOpensFlags,
    ModuleRequiresFlags,
};
use super::super::{ModuleAttribute, ModuleExport, ModuleOpen, ModuleProvide, ModuleRequire};

pub(super) fn parse_module(
    name_index: u16,
    reader: &mut Reader<'_>,
    pool: &ConstantPool,
) -> Result<ModuleAttribute> {
    let module_name_index = reader.read_u16()?;
    expect_module(pool, module_name_index)?;
    let module_flags = ModuleAccessFlags::from_bits_retain(reader.read_u16()?);
    let module_version_index = reader.read_u16()?;
    expect_optional_utf8(pool, module_version_index)?;

    let requires_count = usize::from(reader.read_u16()?);
    let mut requires = Vec::with_capacity(requires_count);
    for _ in 0..requires_count {
        let requirement = ModuleRequire {
            module_index: reader.read_u16()?,
            flags: ModuleRequiresFlags::from_bits_retain(reader.read_u16()?),
            version_index: reader.read_u16()?,
        };
        expect_module(pool, requirement.module_index)?;
        expect_optional_utf8(pool, requirement.version_index)?;
        requires.push(requirement);
    }

    let exports = parse_exports(reader, pool)?;
    let opens = parse_opens(reader, pool)?;
    let uses = read_u16_list(reader)?;
    expect_each(pool, &uses, "Class", |constant| {
        matches!(constant, Constant::Class { .. })
    })?;

    let provides_count = usize::from(reader.read_u16()?);
    let mut provides = Vec::with_capacity(provides_count);
    for _ in 0..provides_count {
        let service_index = reader.read_u16()?;
        expect_tag(pool, service_index, "Class", |constant| {
            matches!(constant, Constant::Class { .. })
        })?;
        let implementation_indices = read_u16_list(reader)?;
        if implementation_indices.is_empty() {
            return Err(Error::invalid_class(
                reader.absolute_position(),
                "module provides directive has no implementation",
            ));
        }
        expect_each(pool, &implementation_indices, "Class", |constant| {
            matches!(constant, Constant::Class { .. })
        })?;
        provides.push(ModuleProvide {
            service_index,
            implementation_indices,
        });
    }
    Ok(ModuleAttribute {
        name_index,
        module_name_index,
        module_flags,
        module_version_index,
        requires,
        exports,
        opens,
        uses,
        provides,
    })
}

fn parse_exports(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<ModuleExport>> {
    let count = usize::from(reader.read_u16()?);
    let mut exports = Vec::with_capacity(count);
    for _ in 0..count {
        let package_index = reader.read_u16()?;
        expect_package(pool, package_index)?;
        let flags = ModuleExportsFlags::from_bits_retain(reader.read_u16()?);
        let to_modules = read_u16_list(reader)?;
        expect_each_module(pool, &to_modules)?;
        exports.push(ModuleExport {
            package_index,
            flags,
            to_modules,
        });
    }
    Ok(exports)
}

fn parse_opens(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<ModuleOpen>> {
    let count = usize::from(reader.read_u16()?);
    let mut opens = Vec::with_capacity(count);
    for _ in 0..count {
        let package_index = reader.read_u16()?;
        expect_package(pool, package_index)?;
        let flags = ModuleOpensFlags::from_bits_retain(reader.read_u16()?);
        let to_modules = read_u16_list(reader)?;
        expect_each_module(pool, &to_modules)?;
        opens.push(ModuleOpen {
            package_index,
            flags,
            to_modules,
        });
    }
    Ok(opens)
}

fn read_u16_list(reader: &mut Reader<'_>) -> Result<Vec<u16>> {
    let count = usize::from(reader.read_u16()?);
    (0..count).map(|_| reader.read_u16()).collect()
}

fn expect_optional_utf8(pool: &ConstantPool, index: u16) -> Result<()> {
    if index == 0 {
        Ok(())
    } else {
        expect_tag(pool, index, "Utf8", |constant| {
            matches!(constant, Constant::Utf8(_))
        })
    }
}

fn expect_module(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Module", |constant| {
        matches!(constant, Constant::Module { .. })
    })
}

fn expect_package(pool: &ConstantPool, index: u16) -> Result<()> {
    expect_tag(pool, index, "Package", |constant| {
        matches!(constant, Constant::Package { .. })
    })
}

fn expect_each_module(pool: &ConstantPool, indices: &[u16]) -> Result<()> {
    expect_each(pool, indices, "Module", |constant| {
        matches!(constant, Constant::Module { .. })
    })
}

fn expect_each(
    pool: &ConstantPool,
    indices: &[u16],
    expected: &str,
    predicate: impl Fn(&Constant) -> bool + Copy,
) -> Result<()> {
    for &index in indices {
        expect_tag(pool, index, expected, predicate)?;
    }
    Ok(())
}

fn expect_tag(
    pool: &ConstantPool,
    index: u16,
    expected: &str,
    predicate: impl Fn(&Constant) -> bool,
) -> Result<()> {
    let constant = pool.get(index)?;
    if predicate(constant) {
        Ok(())
    } else {
        Err(Error::invalid_class(
            0,
            format!(
                "constant-pool index #{index} is {}, expected {expected}",
                constant.tag_name()
            ),
        ))
    }
}

//! Whole-function Dalvik RTL storage placement.

use cfglib::ir::rtl::{Error, Place, Placement, Result};
use disassembler::BinaryFormat;
use mlil::{Function, SourceStorage};

use super::{DexRtlDialect, DexStorage};

pub(super) fn plan(function: &Function) -> Result<Placement<DexRtlDialect>> {
    let mut next_register = function
        .variables()
        .iter()
        .filter_map(|variable| variable.native)
        .filter(|native| native.format == BinaryFormat::Dex)
        .filter_map(|native| match native.storage {
            SourceStorage::DexRegister(index) => index.checked_add(1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut placement = Placement::new();
    for variable in function.variables() {
        let preserved = variable.native.filter(|native| {
            native.format == BinaryFormat::Dex
                && matches!(
                    native.storage,
                    SourceStorage::DexRegister(_)
                        | SourceStorage::DexResult
                        | SourceStorage::DexException
                )
        });
        let storage = if let Some(native) = preserved {
            match native.storage {
                SourceStorage::DexRegister(index) => DexStorage::SourceRegister(index),
                SourceStorage::DexResult => DexStorage::SourceResult,
                SourceStorage::DexException => DexStorage::SourceException,
                _ => unreachable!("preserved DEX storage was filtered above"),
            }
        } else {
            let index = next_register;
            next_register = next_register
                .checked_add(1)
                .ok_or_else(|| Error::Lowering("Dalvik RTL register space exceeds u16".into()))?;
            DexStorage::GeneratedRegister(index)
        };
        placement.assign(
            variable.id,
            Place {
                storage,
                lanes: vec![0],
            },
        );
    }
    Ok(placement)
}

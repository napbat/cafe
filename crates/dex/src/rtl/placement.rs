//! Whole-function Dalvik RTL storage placement.

use cfglib::ir::rtl::{Error, Placement, Result, plan_whole_places};
use disassembler::BinaryFormat;
use mlil::{Function, SourceStorage};

use super::{DexRtlDialect, DexStorage};

pub(super) fn plan(function: &Function) -> Result<Placement<DexRtlDialect>> {
    let next_register = u64::from(
        function
            .variables()
            .iter()
            .filter_map(|variable| variable.native)
            .filter(|native| native.format == BinaryFormat::Dex)
            .filter_map(|native| match native.storage {
                SourceStorage::DexRegister(index) => index.checked_add(1),
                _ => None,
            })
            .max()
            .unwrap_or(0),
    );
    plan_whole_places(
        function,
        |variable| {
            variable
                .native
                .filter(|native| native.format == BinaryFormat::Dex)
                .and_then(|native| match native.storage {
                    SourceStorage::DexRegister(index) => Some(DexStorage::SourceRegister(index)),
                    SourceStorage::DexResult => Some(DexStorage::SourceResult),
                    SourceStorage::DexException => Some(DexStorage::SourceException),
                    _ => None,
                })
        },
        |ordinal| {
            next_register
                .checked_add(ordinal)
                .filter(|index| *index < u64::from(u16::MAX))
                .and_then(|index| u16::try_from(index).ok())
                .map(DexStorage::GeneratedRegister)
                .ok_or_else(|| Error::Lowering("Dalvik RTL register space exceeds u16".into()))
        },
    )
}

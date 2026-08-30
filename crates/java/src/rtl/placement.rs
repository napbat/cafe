//! Whole-function JVM RTL storage placement.

use cfglib::ir::rtl::{Error, Placement, Result, plan_whole_places};
use disassembler::BinaryFormat;
use mlil::{Function, SourceStorage};

use super::{JvmRtlDialect, JvmStorage};

pub(super) fn plan(function: &Function) -> Result<Placement<JvmRtlDialect>> {
    let next_local = u64::from(
        function
            .variables()
            .iter()
            .filter_map(|variable| variable.native)
            .filter(|native| native.format == BinaryFormat::JavaClass)
            .filter_map(|native| match native.storage {
                SourceStorage::JvmLocal(index) => index.checked_add(1),
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
                .filter(|native| native.format == BinaryFormat::JavaClass)
                .and_then(|native| match native.storage {
                    SourceStorage::JvmLocal(index) => Some(JvmStorage::SourceLocal(index)),
                    SourceStorage::JvmStack(index) => Some(JvmStorage::SourceStack(index)),
                    _ => None,
                })
        },
        |ordinal| {
            next_local
                .checked_add(ordinal)
                .filter(|index| *index < u64::from(u16::MAX))
                .and_then(|index| u16::try_from(index).ok())
                .map(JvmStorage::GeneratedLocal)
                .ok_or_else(|| Error::Lowering("JVM RTL local space exceeds u16".into()))
        },
    )
}

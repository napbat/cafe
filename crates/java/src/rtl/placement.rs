//! Whole-function JVM RTL storage placement.

use cfglib::ir::rtl::{Error, Place, Placement, Result};
use disassembler::BinaryFormat;
use mlil::{Function, SourceStorage};

use super::{JvmRtlDialect, JvmStorage};

pub(super) fn plan(function: &Function) -> Result<Placement<JvmRtlDialect>> {
    let mut next_local = function
        .variables()
        .iter()
        .filter_map(|variable| variable.native)
        .filter(|native| native.format == BinaryFormat::JavaClass)
        .filter_map(|native| match native.storage {
            SourceStorage::JvmLocal(index) => index.checked_add(1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut placement = Placement::new();
    for variable in function.variables() {
        let preserved = variable.native.filter(|native| {
            native.format == BinaryFormat::JavaClass
                && matches!(
                    native.storage,
                    SourceStorage::JvmLocal(_) | SourceStorage::JvmStack(_)
                )
        });
        let storage = if let Some(native) = preserved {
            match native.storage {
                SourceStorage::JvmLocal(index) => JvmStorage::SourceLocal(index),
                SourceStorage::JvmStack(index) => JvmStorage::SourceStack(index),
                _ => unreachable!("preserved JVM storage was filtered above"),
            }
        } else {
            let index = next_local;
            next_local = next_local
                .checked_add(1)
                .ok_or_else(|| Error::Lowering("JVM RTL local space exceeds u16".into()))?;
            JvmStorage::GeneratedLocal(index)
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

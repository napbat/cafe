//! Target-neutral array signature verification.

use crate::descriptor;
use crate::{ArrayType, Constant, ValueType};

use super::{integer_like, reference_like, valid_type_reference};

pub(super) fn valid_array_allocation(
    array_type: &ArrayType,
    dimensions: u8,
    uses: &[ValueType],
    defs: &[ValueType],
) -> bool {
    dimensions != 0
        && valid_array_type(array_type)
        && array_type
            .descriptor()
            .bytes()
            .take_while(|byte| *byte == b'[')
            .count()
            >= usize::from(dimensions)
        && uses.len() == usize::from(dimensions)
        && defs.len() == 1
        && uses.iter().all(integer_like)
        && exact_array_result(array_type, &defs[0])
}

pub(super) fn valid_initialized_array(
    array_type: &ArrayType,
    uses: &[ValueType],
    defs: &[ValueType],
) -> bool {
    if !valid_array_type(array_type) || defs.len() != 1 || !exact_array_result(array_type, &defs[0])
    {
        return false;
    }
    let Some(component_descriptor) = array_type.descriptor().strip_prefix('[') else {
        return false;
    };
    let Some(component) = descriptor::field_type(component_descriptor) else {
        return false;
    };
    uses.iter()
        .all(|value| descriptor::accepts(&component, value))
}

pub(super) fn valid_array_initialization(
    array_type: &ArrayType,
    values: &[Constant],
    uses: &[ValueType],
    defs: &[ValueType],
) -> bool {
    if !valid_array_type(array_type)
        || uses.len() != 1
        || !defs.is_empty()
        || !compatible_array_use(array_type, &uses[0])
    {
        return false;
    }
    let Some(component) = array_type.descriptor().strip_prefix('[') else {
        return false;
    };
    values.iter().all(|value| match component.as_bytes() {
        [b'Z' | b'B' | b'C' | b'S' | b'I'] => matches!(value, Constant::Integer(_)),
        [b'J'] => matches!(value, Constant::Long(_)),
        [b'F'] => matches!(value, Constant::Float(_)),
        [b'D'] => matches!(value, Constant::Double(_)),
        _ => false,
    })
}

fn valid_array_type(array_type: &ArrayType) -> bool {
    matches!(
        descriptor::field_type(array_type.descriptor()),
        Some(ValueType::Reference(Some(descriptor))) if descriptor == array_type.descriptor()
    ) && array_type
        .source_reference()
        .is_none_or(valid_type_reference)
}

fn exact_array_result(array_type: &ArrayType, value_type: &ValueType) -> bool {
    match value_type {
        ValueType::Reference(None) => true,
        ValueType::Reference(Some(descriptor)) => descriptor == array_type.descriptor(),
        _ => false,
    }
}

fn compatible_array_use(array_type: &ArrayType, value_type: &ValueType) -> bool {
    reference_like(value_type)
        && !matches!(
            value_type,
            ValueType::Reference(Some(descriptor)) if descriptor != array_type.descriptor()
        )
}

#[cfg(test)]
mod tests {
    use super::{valid_array_allocation, valid_array_initialization};
    use crate::{ArrayType, Constant, ValueType};

    #[test]
    fn exact_array_descriptors_are_part_of_the_typed_signature() {
        let array_type = ArrayType::new("[I");
        assert!(valid_array_allocation(
            &array_type,
            1,
            &[ValueType::Integer],
            &[ValueType::Reference(Some("[I".to_owned()))],
        ));
        assert!(!valid_array_allocation(
            &array_type,
            1,
            &[ValueType::Integer],
            &[ValueType::Reference(Some("[J".to_owned()))],
        ));
        assert!(valid_array_initialization(
            &array_type,
            &[Constant::Integer(1)],
            &[ValueType::Reference(Some("[I".to_owned()))],
            &[],
        ));
        assert!(!valid_array_initialization(
            &array_type,
            &[Constant::Integer(1)],
            &[ValueType::Reference(Some("[J".to_owned()))],
            &[],
        ));
    }
}

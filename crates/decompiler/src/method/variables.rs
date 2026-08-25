//! Java local storage for mutable, multiply typed MLIL variables.

use std::collections::{BTreeMap, BTreeSet};

use java::descriptor::JavaType;
use mlil::{Function, ValueType, VariableId, VariableRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SlotKind {
    Int,
    Long,
    Float,
    Double,
    Object,
}

impl SlotKind {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Int => "i",
            Self::Long => "l",
            Self::Float => "f",
            Self::Double => "d",
            Self::Object => "o",
        }
    }

    const fn declaration_type(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
            Self::Object => "java.lang.Object",
        }
    }

    const fn default_value(self) -> &'static str {
        match self {
            Self::Int => "0",
            Self::Long => "0L",
            Self::Float => "0.0f",
            Self::Double => "0.0d",
            Self::Object => "null",
        }
    }
}

pub(super) struct VariableLayout {
    slots: BTreeSet<(VariableId, SlotKind)>,
    parameters: BTreeMap<u16, String>,
    instance: bool,
}

impl VariableLayout {
    pub(super) fn new(
        function: &Function,
        parameters: &[JavaType],
        parameter_names: &[String],
        instance: bool,
    ) -> Self {
        let mut slots = BTreeSet::new();
        for block in function.cfg().blocks() {
            for instruction in block.instructions() {
                for (&variable, value_type) in instruction
                    .uses()
                    .iter()
                    .zip(instruction.use_types())
                    .chain(instruction.defs().iter().zip(instruction.def_types()))
                {
                    for kind in kinds(value_type) {
                        slots.insert((variable, kind));
                    }
                }
            }
        }
        let parameter_offset = u16::from(instance);
        for (index, parameter) in parameters.iter().enumerate() {
            let ordinal = u16::try_from(index)
                .unwrap_or(u16::MAX)
                .saturating_add(parameter_offset);
            if let Some(variable) = function
                .variables()
                .iter()
                .find(|variable| variable.role == VariableRole::Parameter(ordinal))
            {
                slots.insert((variable.id, java_kind(parameter)));
            }
        }
        if instance
            && let Some(variable) = function
                .variables()
                .iter()
                .find(|variable| variable.role == VariableRole::Parameter(0))
        {
            slots.insert((variable.id, SlotKind::Object));
        }
        let parameters = parameter_names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                u16::try_from(index).ok().map(|index| (index, name.clone()))
            })
            .collect();
        Self {
            slots,
            parameters,
            instance,
        }
    }

    pub(super) fn declarations(&self, function: &Function, parameters: &[JavaType]) -> Vec<String> {
        self.slots
            .iter()
            .map(|&(variable, kind)| {
                let initializer = self.initializer(function, parameters, variable, kind);
                format!(
                    "{} {} = {};",
                    kind.declaration_type(),
                    self.name(variable, kind),
                    initializer
                )
            })
            .collect()
    }

    fn initializer(
        &self,
        function: &Function,
        parameters: &[JavaType],
        variable: VariableId,
        kind: SlotKind,
    ) -> String {
        let Some(declaration) = function.variable(variable) else {
            return kind.default_value().to_owned();
        };
        let VariableRole::Parameter(ordinal) = declaration.role else {
            return kind.default_value().to_owned();
        };
        if self.instance && ordinal == 0 {
            return if kind == SlotKind::Object {
                "this".to_owned()
            } else {
                kind.default_value().to_owned()
            };
        }
        let parameter_index = usize::from(ordinal.saturating_sub(u16::from(self.instance)));
        let Some(parameter_type) = parameters.get(parameter_index) else {
            return kind.default_value().to_owned();
        };
        if java_kind(parameter_type) != kind {
            return kind.default_value().to_owned();
        }
        let name = self
            .parameters
            .get(&u16::try_from(parameter_index).unwrap_or(u16::MAX))
            .cloned()
            .unwrap_or_else(|| format!("parameter{parameter_index}"));
        if matches!(parameter_type, JavaType::Boolean) {
            format!("({name} ? 1 : 0)")
        } else {
            name
        }
    }

    pub(super) fn value(&self, variable: VariableId, value_type: &ValueType) -> String {
        self.name(variable, primary_kind(value_type))
    }

    pub(super) fn int(&self, variable: VariableId) -> String {
        self.name(variable, SlotKind::Int)
    }

    pub(super) fn object(&self, variable: VariableId) -> String {
        self.name(variable, SlotKind::Object)
    }

    #[allow(clippy::unused_self)]
    pub(super) fn name(&self, variable: VariableId, kind: SlotKind) -> String {
        format!("cafe_v{}_{}", variable.raw(), kind.suffix())
    }

    #[allow(clippy::unused_self)]
    pub(super) fn kind(&self, value_type: &ValueType) -> SlotKind {
        primary_kind(value_type)
    }
}

pub(super) fn java_kind(value: &JavaType) -> SlotKind {
    match value {
        JavaType::Boolean | JavaType::Byte | JavaType::Char | JavaType::Int | JavaType::Short => {
            SlotKind::Int
        }
        JavaType::Long => SlotKind::Long,
        JavaType::Float => SlotKind::Float,
        JavaType::Double => SlotKind::Double,
        JavaType::Object(_) | JavaType::Array(_) => SlotKind::Object,
    }
}

fn kinds(value_type: &ValueType) -> Vec<SlotKind> {
    match value_type {
        ValueType::Zero | ValueType::Unknown | ValueType::Conflict => {
            vec![SlotKind::Int, SlotKind::Object]
        }
        value => vec![primary_kind(value)],
    }
}

fn primary_kind(value_type: &ValueType) -> SlotKind {
    match value_type {
        ValueType::Boolean | ValueType::Integer | ValueType::Bits32 | ValueType::Zero => {
            SlotKind::Int
        }
        ValueType::Long | ValueType::Bits64 | ValueType::ReturnAddress => SlotKind::Long,
        ValueType::Float => SlotKind::Float,
        ValueType::Double => SlotKind::Double,
        ValueType::Null
        | ValueType::Reference(_)
        | ValueType::UninitializedThis(_)
        | ValueType::Uninitialized { .. }
        | ValueType::Unknown
        | ValueType::Conflict => SlotKind::Object,
    }
}

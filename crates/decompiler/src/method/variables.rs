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
    roles: BTreeMap<VariableId, VariableRole>,
    /// Exact declared Java type of the object view, for variables whose
    /// every reference occurrence names one type.
    object_types: BTreeMap<VariableId, String>,
    instance: bool,
}

/// Running unification of one variable's reference occurrences.
enum Witness {
    Exact(String),
    Poisoned,
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
        let roles = function
            .variables()
            .iter()
            .map(|variable| (variable.id, variable.role))
            .collect();
        Self::assemble(slots, roles, parameters, parameter_names, instance)
    }

    /// A layout over the HLIL view of the same function: occurrences come
    /// from expression nodes, and identities correspond to MLIL variables
    /// by index (HLIL-introduced temporaries extend the identity space).
    /// Object views whose every reference occurrence names one exact type
    /// declare that type instead of `java.lang.Object`.
    pub(super) fn new_hlil(
        function: &super::hlil::HlilFunction,
        request: &super::control::BodyRequest<'_>,
        forwarded: &BTreeSet<VariableId>,
    ) -> Self {
        use mlil::cfglib::ir::hlil::ExpressionKind;
        let canonical: &Function = request.function;
        let parameters = request.parameters;
        let parameter_names = request.parameter_names;
        let instance = request.kind.instance();
        let names = request.names;
        let owner = request.owner;
        let mut slots = BTreeSet::new();
        for expression in function.expressions() {
            if let ExpressionKind::Variable(variable) = expression.kind() {
                let variable = VariableId::from_raw(variable.raw());
                // A variable fully consumed by return forwarding never
                // renders, so it needs no declaration.
                if forwarded.contains(&variable) {
                    continue;
                }
                for kind in kinds(expression.value_type()) {
                    slots.insert((variable, kind));
                }
            }
        }
        // Declared reference types unify over the CANONICAL occurrences: a
        // variable's static type is a semantic fact, so presentation
        // surgery on the derived view (removed copies, cleared blocks)
        // must never change it.
        let mut witnesses: BTreeMap<VariableId, Witness> = BTreeMap::new();
        for block in canonical.cfg().blocks() {
            for instruction in block.instructions() {
                let occurrences = instruction
                    .uses()
                    .iter()
                    .zip(instruction.use_types())
                    .chain(instruction.defs().iter().zip(instruction.def_types()));
                for (&variable, value_type) in occurrences {
                    unify(&mut witnesses, variable, value_type, names);
                }
            }
        }
        let roles: BTreeMap<VariableId, VariableRole> = function
            .variables()
            .iter()
            .map(|variable| (VariableId::from_raw(variable.id.raw()), variable.role))
            .collect();
        let mut object_types = BTreeMap::new();
        for (variable, witness) in witnesses {
            let Witness::Exact(name) = witness else {
                continue;
            };
            match roles.get(&variable) {
                // The delivered-exception name is `java.lang.Throwable`, so
                // exception views stay `Object`-compatible.
                Some(VariableRole::Exception) => continue,
                Some(VariableRole::Parameter(ordinal)) => {
                    // The initializer assigns the incoming parameter (or
                    // `this`), so the declared type must match it exactly.
                    let declared = if instance && *ordinal == 0 {
                        Some(names.class_name(owner))
                    } else {
                        parameters
                            .get(usize::from(ordinal.saturating_sub(u16::from(instance))))
                            .filter(|value| {
                                matches!(value, JavaType::Object(_) | JavaType::Array(_))
                            })
                            .map(|value| names.value_type(value))
                    };
                    if declared.as_deref() != Some(name.as_str()) {
                        continue;
                    }
                }
                _ => {}
            }
            object_types.insert(variable, name);
        }
        let mut layout = Self::assemble(slots, roles, parameters, parameter_names, instance);
        layout.object_types = object_types;
        layout
    }

    fn assemble(
        mut slots: BTreeSet<(VariableId, SlotKind)>,
        roles: BTreeMap<VariableId, VariableRole>,
        parameters: &[JavaType],
        parameter_names: &[String],
        instance: bool,
    ) -> Self {
        let object_types = BTreeMap::new();
        let parameter_offset = u16::from(instance);
        for (index, parameter) in parameters.iter().enumerate() {
            let ordinal = u16::try_from(index)
                .unwrap_or(u16::MAX)
                .saturating_add(parameter_offset);
            if let Some((&variable, _)) = roles
                .iter()
                .find(|(_, role)| **role == VariableRole::Parameter(ordinal))
            {
                slots.insert((variable, java_kind(parameter)));
            }
        }
        if instance
            && let Some((&variable, _)) = roles
                .iter()
                .find(|(_, role)| **role == VariableRole::Parameter(0))
        {
            slots.insert((variable, SlotKind::Object));
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
            roles,
            object_types,
            instance,
        }
    }

    /// The exact declared Java type of one variable's object view, when its
    /// reference occurrences unified to one type.
    pub(super) fn object_type(&self, variable: VariableId) -> Option<&str> {
        self.object_types.get(&variable).map(String::as_str)
    }

    /// Whether the variable occurs through this slot view at all — and is
    /// therefore declared.
    pub(super) fn has_slot(&self, variable: VariableId, kind: SlotKind) -> bool {
        self.slots.contains(&(variable, kind))
    }

    pub(super) fn declarations(&self, parameters: &[JavaType]) -> Vec<String> {
        self.slots
            .iter()
            .map(|&(variable, kind)| {
                let initializer = self.initializer(parameters, variable, kind);
                let declared = if kind == SlotKind::Object {
                    self.object_type(variable)
                        .unwrap_or_else(|| kind.declaration_type())
                } else {
                    kind.declaration_type()
                };
                format!("{declared} {} = {initializer};", self.name(variable, kind))
            })
            .collect()
    }

    fn initializer(&self, parameters: &[JavaType], variable: VariableId, kind: SlotKind) -> String {
        let Some(VariableRole::Parameter(ordinal)) = self.roles.get(&variable) else {
            return kind.default_value().to_owned();
        };
        let ordinal = *ordinal;
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

/// Merges one occurrence's reference typing into the running witnesses.
/// `Null` and the Dalvik zero pattern are assignable to any declared
/// reference type; an unnamed or conflicting reference pins the view to
/// `java.lang.Object`.
fn unify(
    witnesses: &mut BTreeMap<VariableId, Witness>,
    variable: VariableId,
    value_type: &ValueType,
    names: &crate::names::SourceNames,
) {
    let witness = match value_type {
        ValueType::Reference(Some(descriptor))
        | ValueType::UninitializedThis(descriptor)
        | ValueType::Uninitialized { descriptor, .. } => {
            match super::instruction::reference_type_name(descriptor, names) {
                Ok(name) => Witness::Exact(name),
                Err(_) => Witness::Poisoned,
            }
        }
        ValueType::Reference(None) | ValueType::Unknown | ValueType::Conflict => Witness::Poisoned,
        // `Null` and the Dalvik zero pattern are assignable to any declared
        // reference type, and primitive occurrences do not constrain the
        // object view.
        _ => return,
    };
    match (witnesses.get(&variable), witness) {
        (None, witness) => {
            witnesses.insert(variable, witness);
        }
        (Some(Witness::Exact(existing)), Witness::Exact(name)) if *existing == name => {}
        (Some(Witness::Poisoned), _) => {}
        (Some(_), _) => {
            witnesses.insert(variable, Witness::Poisoned);
        }
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

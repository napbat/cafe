//! MLIL function analyses and Java method-body rendering.

mod constructor;
mod control;
mod instruction;
mod variables;

use java::descriptor::parse_method;
use mlil::{Function, VariableRole};

use crate::diagnostic::MethodIdentity;
use crate::model::DecompiledBody;
use crate::options::DecompilerOptions;

pub(crate) use self::control::{BodyKind, BodyRequest, render};

/// Decompiles one verified MLIL function into Java body statements.
///
/// Parameter variables are named `parameter0`, `parameter1`, and so on. The
/// returned fragment omits surrounding method braces. Class decompilation uses
/// richer declaration and debug metadata through [`crate::decompile_class`].
#[must_use]
pub fn decompile_function(function: &Function, options: &DecompilerOptions) -> DecompiledBody {
    let descriptor = match parse_method(&function.source().symbol.signature) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return DecompiledBody {
                source: format!(
                    "throw new java.lang.UnsupportedOperationException({});\n",
                    crate::names::rust_string_literal(&error.to_string())
                ),
                diagnostics: vec![crate::Diagnostic::method_error(
                    crate::DiagnosticCode::UnsupportedSemantics,
                    &function.source().symbol.owner,
                    MethodIdentity::new(
                        &function.source().symbol.name,
                        &function.source().symbol.signature,
                    ),
                    error.to_string(),
                )],
                source_map: Vec::new(),
            };
        }
    };
    let names = crate::names::SourceNames::default();
    let parameter_names = (0..descriptor.parameters.len())
        .map(|index| format!("parameter{index}"))
        .collect::<Vec<_>>();
    let parameter_roles = function
        .variables()
        .iter()
        .filter_map(|variable| match variable.role {
            VariableRole::Parameter(ordinal) => Some(ordinal),
            VariableRole::Local
            | VariableRole::Temporary
            | VariableRole::Condition
            | VariableRole::Exception => None,
        })
        .max()
        .map_or(0usize, |ordinal| usize::from(ordinal) + 1);
    let instance = parameter_roles > descriptor.parameters.len();
    let request = BodyRequest {
        function,
        owner: &function.source().symbol.owner,
        method: MethodIdentity::new(
            &function.source().symbol.name,
            &function.source().symbol.signature,
        ),
        parameters: &descriptor.parameters,
        parameter_names: &parameter_names,
        return_type: &descriptor.return_type,
        kind: BodyKind::for_method(&function.source().symbol.name, instance, false),
        options,
        rethrow: "cafe_rethrow",
        names: &names,
    };
    let rendered = render(&request);
    DecompiledBody {
        source: rendered.source,
        diagnostics: rendered.diagnostics,
        source_map: rendered.source_map,
    }
}

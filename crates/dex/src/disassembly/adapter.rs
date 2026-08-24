//! DEX file, class, method, and exception lowering.

use disassembler::{
    AddressRange, AddressUnit, BinaryFormat, CatchType, CodeAddress, Disassembly,
    DisassemblySource, ExceptionHandler as SharedExceptionHandler, Function, FunctionBody,
    FunctionSymbol, RawAccessFlags,
};

use super::instruction::{Payloads, lower_instruction};
use crate::file::{CodeItem, DexFile, EncodedMethod};
use crate::{DEFAULT_DEX_FILE_NAME, Error, Result};

/// Lowers a parsed DEX file using [`DEFAULT_DEX_FILE_NAME`] as its artifact name.
///
/// # Errors
///
/// Returns an error when identifiers, instructions, switch payloads, exception
/// metadata, or the resulting control-flow graph cannot be resolved.
pub fn lower_file(file: &DexFile) -> Result<Disassembly> {
    lower_file_named(file, DEFAULT_DEX_FILE_NAME)
}

/// Lowers a parsed DEX file using an explicit artifact name.
///
/// # Errors
///
/// Returns an error when identifiers, instructions, switch payloads, exception
/// metadata, or the resulting control-flow graph cannot be resolved.
pub fn lower_file_named(file: &DexFile, name: impl Into<String>) -> Result<Disassembly> {
    let mut functions = Vec::new();
    for class in file.classes() {
        let Some(data) = &class.class_data else {
            continue;
        };
        for method in data.direct_methods.iter().chain(&data.virtual_methods) {
            functions.push(lower_method(file, method)?);
        }
    }
    Ok(Disassembly {
        format: BinaryFormat::Dex,
        name: name.into(),
        functions,
    })
}

impl DisassemblySource for DexFile {
    type Error = Error;

    fn disassemble(&self) -> Result<Disassembly> {
        lower_file(self)
    }
}

fn lower_method(file: &DexFile, declaration: &EncodedMethod) -> Result<Function> {
    let identity = file.resolve_method(declaration.method)?;
    let body = declaration
        .code
        .as_ref()
        .map(|code| lower_body(file, code))
        .transpose()
        .map_err(|error| {
            error.in_method(identity.owner, identity.name, identity.signature.clone())
        })?;
    Ok(Function {
        symbol: FunctionSymbol {
            owner: identity.owner.to_owned(),
            name: identity.name.to_owned(),
            signature: identity.signature,
        },
        access_flags: RawAccessFlags::new(declaration.access_flags.bits()),
        body,
    })
}

pub(crate) fn lower_body(file: &DexFile, code: &CodeItem) -> Result<FunctionBody> {
    let payloads = Payloads::new(&code.instructions)?;
    let instructions = code
        .instructions
        .iter()
        .map(|instruction| lower_instruction(instruction, file, &payloads))
        .collect::<Result<Vec<_>>>()?;
    let mut exception_handlers = Vec::new();
    for protected in &code.tries {
        let end = protected
            .start_address
            .checked_add(u32::from(protected.instruction_count))
            .ok_or_else(|| {
                Error::invalid_instruction(
                    protected.start_address,
                    "exception range exceeds the DEX address space",
                )
            })?;
        for handler in &protected.handlers {
            let catch = match handler.exception_type {
                Some(exception_type) => {
                    CatchType::Type(file.type_descriptor(exception_type)?.to_owned())
                }
                None => CatchType::Any,
            };
            exception_handlers.push(SharedExceptionHandler {
                protected: AddressRange::new(
                    CodeAddress::from(protected.start_address),
                    CodeAddress::from(end),
                ),
                handler: CodeAddress::from(handler.address),
                catch,
            });
        }
    }

    let body = FunctionBody::new(AddressUnit::CodeUnit16, instructions, exception_handlers);
    body.control_flow_graph()?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use disassembler::{BinaryFormat, DisassemblySource, InstructionFlow, Operand};

    use super::lower_file;
    use crate::file::{
        AccessFlags, AnnotationDirectory, ClassData, ClassDefinition, CodeItem, DexFile, DexString,
        DexVersion, EncodedMethod, MethodId, PrototypeId, TypeId,
    };
    use crate::instruction::{Instruction, Opcode, Operands, PackedSwitchPayload};

    fn branching_file() -> DexFile {
        let mut file = DexFile::new(DexVersion::V040);
        let method_name = file.push_string(DexString::new("choose")).unwrap();
        let owner_descriptor = file.push_string(DexString::new("LExample;")).unwrap();
        let void_descriptor = file.push_string(DexString::new("V")).unwrap();
        let owner = file
            .push_type(TypeId {
                descriptor: owner_descriptor,
            })
            .unwrap();
        let void = file
            .push_type(TypeId {
                descriptor: void_descriptor,
            })
            .unwrap();
        let prototype = file
            .push_prototype(PrototypeId {
                shorty: void_descriptor,
                return_type: void,
                parameters: Vec::new(),
                parameters_offset: 0,
            })
            .unwrap();
        let method = file
            .push_method(MethodId {
                class: owner,
                prototype,
                name: method_name,
            })
            .unwrap();
        let code = CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            instructions: vec![
                Instruction::operation(
                    0,
                    Opcode::PackedSwitch,
                    Operands::RegisterBranch {
                        register: 0,
                        target: 4,
                    },
                ),
                Instruction::operation(3, Opcode::ReturnVoid, Operands::None),
                Instruction::packed_switch(
                    4,
                    PackedSwitchPayload {
                        first_key: 7,
                        targets: vec![3],
                    },
                ),
            ],
            tries: Vec::new(),
            debug_info: None,
            data_offset: 0,
        };
        file.push_class(ClassDefinition {
            class: owner,
            access_flags: AccessFlags::PUBLIC,
            superclass: None,
            interfaces: Vec::new(),
            source_file: None,
            annotations: AnnotationDirectory::default(),
            class_data: Some(ClassData {
                static_fields: Vec::new(),
                instance_fields: Vec::new(),
                direct_methods: vec![EncodedMethod {
                    method,
                    access_flags: AccessFlags::STATIC,
                    code: Some(code),
                }],
                virtual_methods: Vec::new(),
                data_offset: 0,
            }),
            static_values: Vec::new(),
            definition_index: 0,
        })
        .unwrap();
        file
    }

    #[test]
    fn lowers_switches_payloads_and_verified_graphs() {
        let file = branching_file();
        let direct = lower_file(&file).unwrap();
        let through_trait = file.disassemble().unwrap();

        assert_eq!(direct, through_trait);
        assert_eq!(direct.format, BinaryFormat::Dex);
        let body = direct.functions[0].body.as_ref().unwrap();
        assert!(matches!(
            body.instructions[0].flow,
            InstructionFlow::Switch { .. }
        ));
        assert!(matches!(
            body.instructions[0].operands[2],
            Operand::Switch(_)
        ));
        let graph = body.control_flow_graph().unwrap();
        assert_eq!(graph.cfg().num_edges(), 2);
    }
}

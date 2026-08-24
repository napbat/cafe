//! Binary parser for JVM class files and their attributes.

use super::attribute::parse_attributes;
use super::io::Reader;
use super::{
    AttributeLocation, CLASS_MAGIC, CLASS_MAGIC_OFFSET, ClassAccessFlags, ClassFile, ConstantPool,
    FieldAccessFlags, FieldInfo, MethodAccessFlags, MethodInfo, NO_SUPER_CLASS_INDEX,
};
use crate::{Error, Result};

pub(super) fn parse_class(bytes: &[u8]) -> Result<ClassFile> {
    let mut reader = Reader::new(bytes);
    let magic = reader.read_u32()?;
    if magic != CLASS_MAGIC {
        return Err(Error::invalid_class(
            CLASS_MAGIC_OFFSET,
            format!("bad magic 0x{magic:08x}; expected 0x{CLASS_MAGIC:08x}"),
        ));
    }

    let minor_version = reader.read_u16()?;
    let major_version = reader.read_u16()?;
    let constant_pool = ConstantPool::parse(&mut reader)?;
    constant_pool.validate()?;

    let access_flags = ClassAccessFlags::from_bits_retain(reader.read_u16()?);
    let this_class = reader.read_u16()?;
    constant_pool.expect_class(this_class)?;
    let super_class = reader.read_u16()?;
    if super_class != NO_SUPER_CLASS_INDEX {
        constant_pool.expect_class(super_class)?;
    }

    let interfaces = read_indices(&mut reader)?;
    for &index in &interfaces {
        constant_pool.expect_class(index)?;
    }

    let fields = parse_fields(&mut reader, &constant_pool)?;
    let methods = parse_methods(&mut reader, &constant_pool)?;
    let attributes = parse_attributes(&mut reader, &constant_pool, AttributeLocation::Class)?;
    reader.finish("class file")?;

    let class = ClassFile {
        minor_version,
        major_version,
        constant_pool,
        access_flags,
        this_class,
        super_class,
        interfaces,
        fields,
        methods,
        attributes,
    };
    class.validate()?;
    Ok(class)
}

fn parse_fields(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<FieldInfo>> {
    let count = usize::from(reader.read_u16()?);
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        let access_flags = FieldAccessFlags::from_bits_retain(reader.read_u16()?);
        let name_index = reader.read_u16()?;
        pool.expect_utf8(name_index)?;
        let descriptor_index = reader.read_u16()?;
        pool.expect_utf8(descriptor_index)?;
        let attributes = parse_attributes(reader, pool, AttributeLocation::Field)?;
        fields.push(FieldInfo {
            access_flags,
            name_index,
            descriptor_index,
            attributes,
        });
    }
    Ok(fields)
}

fn parse_methods(reader: &mut Reader<'_>, pool: &ConstantPool) -> Result<Vec<MethodInfo>> {
    let count = usize::from(reader.read_u16()?);
    let mut methods = Vec::with_capacity(count);
    for _ in 0..count {
        let access_flags = MethodAccessFlags::from_bits_retain(reader.read_u16()?);
        let name_index = reader.read_u16()?;
        pool.expect_utf8(name_index)?;
        let descriptor_index = reader.read_u16()?;
        pool.expect_utf8(descriptor_index)?;
        let attributes = parse_attributes(reader, pool, AttributeLocation::Method)?;
        methods.push(MethodInfo {
            access_flags,
            name_index,
            descriptor_index,
            attributes,
        });
    }
    Ok(methods)
}

fn read_indices(reader: &mut Reader<'_>) -> Result<Vec<u16>> {
    let count = usize::from(reader.read_u16()?);
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(reader.read_u16()?);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{self, Opcode};
    use crate::classfile::{
        CLASS_MAGIC, ClassAccessFlags, ClassFile, Constant, ConstantPool, ConstantTag,
        MethodAccessFlags,
    };

    const JAVA_8_CLASS_MAJOR: u16 = 52;

    fn minimal_class() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&CLASS_MAGIC.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&JAVA_8_CLASS_MAJOR.to_be_bytes());
        bytes.extend_from_slice(&5_u16.to_be_bytes());
        push_utf8(&mut bytes, "Example");
        bytes.push(ConstantTag::Class.byte());
        bytes.extend_from_slice(&1_u16.to_be_bytes());
        push_utf8(&mut bytes, "java/lang/Object");
        bytes.push(ConstantTag::Class.byte());
        bytes.extend_from_slice(&3_u16.to_be_bytes());
        bytes.extend_from_slice(
            &(ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER)
                .bits()
                .to_be_bytes(),
        );
        bytes.extend_from_slice(&2_u16.to_be_bytes());
        bytes.extend_from_slice(&4_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes
    }

    fn push_utf8(bytes: &mut Vec<u8>, value: &str) {
        bytes.push(ConstantTag::Utf8.byte());
        bytes.extend_from_slice(
            &u16::try_from(value.len())
                .expect("test strings fit in a u16")
                .to_be_bytes(),
        );
        bytes.extend_from_slice(value.as_bytes());
    }

    fn class_with_constructor() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&CLASS_MAGIC.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        bytes.extend_from_slice(&JAVA_8_CLASS_MAJOR.to_be_bytes());
        bytes.extend_from_slice(&10_u16.to_be_bytes());
        push_utf8(&mut bytes, "Example"); // #1
        bytes.push(ConstantTag::Class.byte());
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // #2 Class Example
        push_utf8(&mut bytes, "java/lang/Object"); // #3
        bytes.push(ConstantTag::Class.byte());
        bytes.extend_from_slice(&3_u16.to_be_bytes()); // #4 Class Object
        push_utf8(&mut bytes, "<init>"); // #5
        push_utf8(&mut bytes, "()V"); // #6
        push_utf8(&mut bytes, "Code"); // #7
        bytes.push(ConstantTag::NameAndType.byte());
        bytes.extend_from_slice(&5_u16.to_be_bytes());
        bytes.extend_from_slice(&6_u16.to_be_bytes()); // #8 NameAndType
        bytes.push(ConstantTag::MethodRef.byte());
        bytes.extend_from_slice(&4_u16.to_be_bytes());
        bytes.extend_from_slice(&8_u16.to_be_bytes()); // #9 Methodref

        bytes.extend_from_slice(
            &(ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER)
                .bits()
                .to_be_bytes(),
        );
        bytes.extend_from_slice(&2_u16.to_be_bytes());
        bytes.extend_from_slice(&4_u16.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // interfaces
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // fields
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // methods
        bytes.extend_from_slice(&MethodAccessFlags::PUBLIC.bits().to_be_bytes());
        bytes.extend_from_slice(&5_u16.to_be_bytes());
        bytes.extend_from_slice(&6_u16.to_be_bytes());
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // method attributes
        bytes.extend_from_slice(&7_u16.to_be_bytes());
        bytes.extend_from_slice(&17_u32.to_be_bytes());
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // max_stack
        bytes.extend_from_slice(&1_u16.to_be_bytes()); // max_locals
        bytes.extend_from_slice(&5_u32.to_be_bytes());
        bytes.extend_from_slice(&[
            Opcode::ALoad0.byte(),
            Opcode::InvokeSpecial.byte(),
            0x00,
            0x09,
            Opcode::Return.byte(),
        ]);
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // exception table
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // code attributes
        bytes.extend_from_slice(&0_u16.to_be_bytes()); // class attributes
        bytes
    }

    #[test]
    fn parses_minimal_class() {
        let class = ClassFile::parse(&minimal_class()).unwrap();
        assert_eq!(class.class_name().unwrap(), "Example");
        assert_eq!(class.super_name().unwrap(), Some("java/lang/Object"));
        assert_eq!(class.java_release(), Some(8));
        assert!(matches!(
            class.constant_pool.get(1).unwrap(),
            Constant::Utf8(value) if value.as_str() == "Example"
        ));
    }

    #[test]
    fn assembles_a_class_built_from_typed_structures() {
        let mut constant_pool = ConstantPool::new();
        let class_name = constant_pool.push_utf8("Example").unwrap();
        let this_class = constant_pool
            .push(Constant::Class {
                name_index: class_name,
            })
            .unwrap();
        let super_name = constant_pool.push_utf8("java/lang/Object").unwrap();
        let super_class = constant_pool
            .push(Constant::Class {
                name_index: super_name,
            })
            .unwrap();
        let class = ClassFile {
            minor_version: 0,
            major_version: JAVA_8_CLASS_MAJOR,
            constant_pool,
            access_flags: ClassAccessFlags::PUBLIC | ClassAccessFlags::SUPER,
            this_class,
            super_class,
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            attributes: Vec::new(),
        };

        let encoded = class.to_bytes().unwrap();
        assert_eq!(encoded, minimal_class());
        assert_eq!(
            ClassFile::parse(&encoded).unwrap().class_name().unwrap(),
            "Example"
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = minimal_class();
        bytes.push(0);
        assert!(
            ClassFile::parse(&bytes)
                .unwrap_err()
                .to_string()
                .contains("trailing")
        );
    }

    #[test]
    fn parses_and_decodes_code_attribute() {
        let original = class_with_constructor();
        let class = ClassFile::parse(&original).unwrap();
        let code = class.methods[0].code().expect("constructor has code");
        assert_eq!(code.max_stack, 1);
        assert_eq!(code.max_locals, 1);
        let instructions = bytecode::decode_code(code).unwrap();
        assert_eq!(
            instructions
                .iter()
                .map(bytecode::Instruction::mnemonic)
                .collect::<Vec<_>>(),
            ["aload_0", "invokespecial", "return"]
        );
        assert_eq!(class.to_bytes().unwrap(), original);
    }
}

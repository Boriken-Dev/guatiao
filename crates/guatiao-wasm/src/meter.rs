// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bulk instructions charged by their length. wasmtime counts one unit of
//! fuel per instruction, so `memory.fill` over a whole memory costs what
//! `i32.add` does; under an operations limit a module is rewritten to call
//! the host's [`BULK`] with each bulk instruction's length first, which
//! charges for it and answers the length unchanged.

use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{
    CodeSection, EntityType, ImportSection, Instruction, Module, SectionId, TypeSection, ValType,
};
use wasmparser::{FunctionBody, Operator, Parser, Payload};

/// The module the host's charge is imported from: no guest may import it.
pub(crate) const MODULE: &str = "guatiao:meter";
/// `(len: i32) -> i32`: charges for `len` bytes, answers `len`.
pub(crate) const BULK: &str = "bulk";

/// Whether `op` touches as many bytes or entries as the `i32` on top of
/// the stack says.
fn is_bulk(op: &Operator<'_>) -> bool {
    matches!(
        op,
        Operator::MemoryFill { .. }
            | Operator::MemoryCopy { .. }
            | Operator::MemoryInit { .. }
            | Operator::TableFill { .. }
            | Operator::TableCopy { .. }
            | Operator::TableInit { .. }
    )
}

/// What the rewrite needs to know of a module before it starts.
struct Survey {
    imported_functions: u32,
    types: u32,
    has_imports: bool,
    has_bulk: bool,
}

fn survey(bytes: &[u8]) -> Result<Survey, String> {
    let malformed = |e: wasmparser::BinaryReaderError| e.message().to_string();
    let mut survey = Survey {
        imported_functions: 0,
        types: 0,
        has_imports: false,
        has_bulk: false,
    };
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(malformed)? {
            Payload::TypeSection(types) => {
                for group in types {
                    survey.types += group.map_err(malformed)?.types().len() as u32;
                }
            }
            Payload::ImportSection(imports) => {
                survey.has_imports = true;
                for import in imports {
                    let import = import.map_err(malformed)?;
                    if import.module == MODULE {
                        return Err(format!(
                            "it imports from `{MODULE}`, which is the host's own"
                        ));
                    }
                    if matches!(import.ty, wasmparser::TypeRef::Func(_)) {
                        survey.imported_functions += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) if !survey.has_bulk => {
                let mut ops = body.get_operators_reader().map_err(malformed)?;
                while !ops.eof() {
                    if is_bulk(&ops.read().map_err(malformed)?) {
                        survey.has_bulk = true;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(survey)
}

/// Re-encodes a module with the charge imported after its own imports, so
/// its functions move up by one.
struct Meter {
    imported_functions: u32,
    /// The charge's type: appended after the module's own.
    bulk_type: u32,
    has_imports: bool,
    imported: bool,
}

impl Meter {
    fn charge(&self) -> u32 {
        self.imported_functions
    }

    fn import(&mut self, imports: &mut ImportSection) {
        imports.import(MODULE, BULK, EntityType::Function(self.bulk_type));
        self.imported = true;
    }
}

impl Reencode for Meter {
    type Error = core::convert::Infallible;

    fn function_index(&mut self, func: u32) -> Result<u32, reencode::Error> {
        Ok(if func >= self.imported_functions {
            func + 1
        } else {
            func
        })
    }

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        reencode::utils::parse_type_section(self, types, section)?;
        types.ty().function([ValType::I32], [ValType::I32]);
        Ok(())
    }

    fn parse_import_section(
        &mut self,
        imports: &mut ImportSection,
        section: wasmparser::ImportSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        reencode::utils::parse_import_section(self, imports, section)?;
        self.import(imports);
        Ok(())
    }

    /// A module with no imports gets a section holding the charge alone,
    /// where the import section goes.
    fn intersperse_section_hook(
        &mut self,
        module: &mut Module,
        _after: Option<SectionId>,
        before: Option<SectionId>,
    ) -> Result<(), reencode::Error> {
        let past_imports = !matches!(before, Some(SectionId::Type | SectionId::Import));
        if !self.has_imports && !self.imported && past_imports {
            let mut imports = ImportSection::new();
            self.import(&mut imports);
            module.section(&imports);
        }
        Ok(())
    }

    fn parse_function_body(
        &mut self,
        code: &mut CodeSection,
        func: FunctionBody<'_>,
    ) -> Result<(), reencode::Error> {
        let mut f = self.new_function_with_parsed_locals(&func)?;
        let mut ops = func.get_operators_reader()?;
        while !ops.eof() {
            let op = ops.read()?;
            if is_bulk(&op) {
                f.instruction(&Instruction::Call(self.charge()));
            }
            f.instruction(&self.instruction(op)?);
        }
        code.function(&f);
        Ok(())
    }

    /// DWARF names code offsets the rewrite moved: it goes.
    fn parse_custom_section(
        &mut self,
        module: &mut Module,
        section: wasmparser::CustomSectionReader<'_>,
    ) -> Result<(), reencode::Error> {
        if section.name().starts_with(".debug") {
            return Ok(());
        }
        reencode::utils::parse_custom_section(self, module, section)
    }
}

/// `bytes` with every bulk instruction charged for, or unchanged when it
/// has none.
pub(crate) fn metered(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let survey = survey(bytes)?;
    if !survey.has_bulk {
        return Ok(bytes.to_vec());
    }
    let mut meter = Meter {
        imported_functions: survey.imported_functions,
        bulk_type: survey.types,
        has_imports: survey.has_imports,
        imported: false,
    };
    let mut module = Module::new();
    meter
        .parse_core_module(&mut module, Parser::new(0), bytes)
        .map_err(|e| e.to_string())?;
    Ok(module.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module's imports, how many calls its code makes, and the function
    /// its start section names.
    fn imports_and_calls(bytes: &[u8]) -> (Vec<(String, String)>, usize, Option<u32>) {
        let mut imports = Vec::new();
        let mut calls = 0;
        let mut start = None;
        for payload in Parser::new(0).parse_all(bytes) {
            match payload.expect("parses") {
                Payload::ImportSection(s) => {
                    for i in s {
                        let i = i.expect("an import");
                        imports.push((i.module.to_string(), i.name.to_string()));
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    let mut ops = body.get_operators_reader().expect("operators");
                    while !ops.eof() {
                        if let Operator::Call { .. } = ops.read().expect("an operator") {
                            calls += 1;
                        }
                    }
                }
                Payload::StartSection { func, .. } => start = Some(func),
                _ => {}
            }
        }
        (imports, calls, start)
    }

    #[test]
    fn a_module_without_bulk_instructions_is_left_alone() {
        let bytes = wat::parse_str(r#"(module (func (export "f") (result i32) (i32.const 1)))"#)
            .expect("parses");
        assert_eq!(metered(&bytes).expect("metered"), bytes);
    }

    #[test]
    fn each_bulk_instruction_calls_the_charge_and_calls_move_up_by_one() {
        let bytes = wat::parse_str(
            r#"(module
                 (import "guatiao" "answer" (func $answer (param i32 i32)))
                 (memory 1)
                 (table 2 funcref)
                 (elem (i32.const 0) func $fill $copy)
                 (data "abc")
                 (func $fill (export "fill") (param i32)
                   (memory.fill (i32.const 0) (i32.const 1) (local.get 0))
                   (call $copy))
                 (func $copy
                   (memory.copy (i32.const 0) (i32.const 8) (i32.const 4))
                   (memory.init 0 (i32.const 0) (i32.const 0) (i32.const 3))
                   (call $answer (i32.const 0) (i32.const 3)))
                 (start $copy))"#,
        )
        .expect("parses");
        let out = metered(&bytes).expect("metered");
        wasmparser::Validator::new()
            .validate_all(&out)
            .expect("still valid");
        let (imports, calls, start) = imports_and_calls(&out);
        assert_eq!(
            imports,
            [
                ("guatiao".to_string(), "answer".to_string()),
                (MODULE.to_string(), BULK.to_string())
            ]
        );
        // Two calls of the module's own, and three charges.
        assert_eq!(calls, 5);
        // `$copy` was function 2, after the one import.
        assert_eq!(start, Some(3));
    }

    #[test]
    fn a_module_with_no_imports_gets_the_charge_alone() {
        let bytes = wat::parse_str(
            r#"(module (memory 1)
                 (func (export "f") (memory.fill (i32.const 0) (i32.const 0) (i32.const 10))))"#,
        )
        .expect("parses");
        let out = metered(&bytes).expect("metered");
        wasmparser::Validator::new()
            .validate_all(&out)
            .expect("still valid");
        assert_eq!(
            imports_and_calls(&out),
            (vec![(MODULE.to_string(), BULK.to_string())], 1, None)
        );
    }

    #[test]
    fn a_module_importing_the_charge_itself_is_refused() {
        let bytes = wat::parse_str(
            r#"(module (import "guatiao:meter" "bulk" (func (param i32) (result i32))))"#,
        )
        .expect("parses");
        assert!(metered(&bytes).unwrap_err().contains("host's own"));
    }
}
